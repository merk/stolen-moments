# Stolen Moments — Development Plan

This document expands the terse items in [`TODO.md`](./TODO.md) into a sequenced,
detailed proposal. The game is pivoting from a generic noise-cavern toy into a
**casino heist with a time-loop mechanic**: explore, learn guard routines and
vault codes across runs, then execute a clean heist in a single loop while your
past runs replay as ghosts.

Three cross-cutting design decisions (settled with Tim) shape everything below;
they are described once here and referenced by the per-item plans.

---

## Cross-cutting decisions

### A. Configurable level *sources* (not a single generator)

`dungeon.rs` today hardcodes one Perlin generator. We generalise to a **level
source** abstraction so the same downstream systems work whether a level is
procedurally generated, a noise/room hybrid, or fully hand-authored later by a
designer. This is primarily about *development speed and flexibility*, not about
committing to one art direction.

```rust
/// Produces a fully-described level: tile grid + semantic room tags +
/// spawn metadata. Implemented by procedural, hybrid, and file-backed sources.
trait LevelSource {
    fn build(&self, seed: u64) -> Level;
}

struct Level {
    map: DungeonMap,                 // tiles (existing type, extended)
    rooms: Vec<Room>,                // semantic regions (NEW)
    spawn: SpawnPoint,               // existing
}

struct Room {
    kind: RoomKind,                  // Start | Lobby | GameTables | Vault | Security | Service
    rect: TileRect,                  // bounding region on the grid
    tiles: Vec<(usize, usize)>,      // member floor tiles
}
```

Implementations:
- `NoiseSource` — today's behaviour, kept as the "organic back-of-house" filler.
- `HybridSource` (the default for now) — runs the noise pass, then *stamps* typed
  rooms into the grid (see "Stamping & sealing" below). Preserves the current
  organic look while adding the structure items 10/11 need.
- `FileSource` (later) — loads a hand-authored level (RON/Tiled-style grid +
  room tags). No engine changes needed once the trait exists.

#### Stamping & sealing

A stamped room is a region of `Floor` tiles overwritten into the grid. *How* it
meets the surrounding caverns is a per-`RoomKind` choice:

- **Open room** (e.g. Lobby, back-of-house) — stamp floor only, no forced wall
  ring. Its floor merges with adjacent cavern floor, so the resulting open space
  naturally *follows the cavern shape*; the rectangle just guarantees a minimum
  clear footprint inside an otherwise organic area.
  - *Enclosure is implicit, not absent:* "no walls" means we don't carve a ring —
    it does **not** mean the floor floats in a void. Any tile outside the stamp
    that's still `Wall` and now borders the new floor renders as a wall
    (`borders_floor`). So where an open room stamps *beyond* the cavern into solid
    rock, the surrounding rock encloses it automatically; only the edges that
    abut existing cavern floor stay open.
  - *Aesthetic note (not correctness):* those auto-walls follow the rectangle's
    straight edge, so a room jutting into rock looks organic on its cavern side
    and clean-edged on its rock side. Optional fix: a placement rule requiring an
    open room to sit mostly within existing cavern floor, so its whole boundary
    stays organic. (A clean edge may be fine/desirable for the Lobby.)
- **Sealed room** (Vault, Security) — stamp the interior as floor **and force a
  wall ring around the perimeter**, overwriting any cavern floor at the border so
  the room *clips/truncates* the cavern rather than merging with it. Leave 1–2
  deliberate **doorway** tiles. This is what gives the heist its controlled
  chokepoints. (A future variant could trace walls along the cavern outline for a
  sealed-but-irregular room; rectangles are the default.)

Each `RoomKind` carries a `seal: Sealed | Open` policy.

#### Reachability contract

*Every room must be reachable from the Start/spawn after generation, and sealed
rooms are entered only through their intended doorways.* The existing
`connect_regions`/`carve_corridor` is **not** protected-aware — its blind L-shaped
carve would drill straight through a sealed wall ring, creating uncontrolled
entrances — so connectivity is reworked:

- Stamping a sealed room marks its wall-ring tiles **protected** (immutable
  `Wall`) for the rest of generation, and records each doorway + the **approach**
  tile just outside it.
- Connectivity becomes **protected-aware routing**: from each room's
  doorway-approach, route into the main connected component through *carvable*
  (non-protected) tiles only — a BFS/router, not a blind L — treating every
  room's ring and the map border as impassable. Doorways stay the only sanctioned
  entrances.
- Run after *all* rooms are stamped, then mop up any leftover disconnected noise
  caverns with the same protected-aware carver. Reachability is measured from the
  Start room.
- Failure handling: if a room genuinely can't be connected (rings box it in), log
  and fall back — relocate or re-stamp that room. Seeded, so a given seed
  reliably either succeeds or takes the fallback.

`DungeonMap` gains a parallel `room_of: Vec<Option<RoomId>>` lookup so any system
can ask "what kind of room is this tile in?" (used by spawns, AI, and mechanics).

### B. Tagged persistence layer

The existing `LoopReset` event resets *everything*. The heist needs some things
to carry forward. Rather than hardcode a policy (which will evolve with
playtesting), we add a **persistence registry** where individual facts/entities
opt into surviving a reset.

```rust
#[derive(Resource, Default)]
struct Persistent {
    facts: HashSet<FactId>,          // knowledge: vault_code_known, keycard_seen, ...
}

/// Component policy for world entities that may persist physical state.
#[derive(Component)]
enum PersistPolicy { ResetEachLoop, KeepForever }
```

- **Knowledge** (codes, observed routines, discovered locations) lives in
  `Persistent::facts` and is never cleared by `LoopReset`.
- **Physical state** can persist per-entity: an entity tagged `KeepForever`
  (e.g. an unlocked door) skips the reset that `ResetEachLoop` entities undergo.
- Individual items can be tagged to become "immediately and always available
  going forward" — i.e. once unlocked/learned, permanently so.

This is plumbing plus *one worked example* (the vault-code fact); which specific
things persist is deliberately left to evolve.

> **Deferred generalisation:** this flat fact set doesn't *compose* (a designer
> can't author "the vault key is 2 of 3 components" without code). A data-driven
> **facts & requirements** system that fixes that — and gives level designers
> real power from RON alone — is designed in [`FUTURE.md`](./FUTURE.md), with a
> clean migration path from this plumbing. Out of scope for now by choice.

### C. Whole-sim determinism (with a record/replay escape hatch)

Item 4 demands guard behaviour identical across loops. We make the **entire
simulation a deterministic function of one master seed** wherever feasible:

```rust
#[derive(Resource)] struct RunSeed(u64);
// sub-seeds: hash(master, "dungeon"), hash(master, "adversary", i), hash(master, "props") ...
```

- Replace every `SmallRng::from_entropy()` in `adversary.rs`, `props.rs`,
  `dungeon.rs` with a seed derived from `RunSeed`.
- Move deterministic agent logic to **`FixedUpdate`** so it's independent of
  frame rate (float accumulation otherwise diverges). This is the one
  unavoidable cost and it's cheap.
- **Reactive ≠ recorded.** A guard that *chases* the live player must be
  live-simulated (the player moves differently each loop). Determinism gives the
  right guarantee here: same `(seed, world, observed targets)` → same decision,
  so undisturbed patrols are *learnable* and repeat exactly.
- **Record-and-replay** (the mechanism ghosts already use in `time_loop.rs`) is
  reserved for (a) ghosts and (b) *non-reactive scripted actors* like the
  employee on a fixed route. It is also the **fallback** if a subsystem proves
  too costly to make deterministic — with the explicit caveat that a recorded
  actor cannot react to the player.

---

## Phased roadmap

Ordered so each phase unblocks the next. Phases 0–1 are foundations; the heist
gameplay (Phase 3) depends on them.

> **Progress** — ✅ done · 🚧 in progress · ⬜ not started. Per-item status is on
> each heading; the commit hash records where a done item landed.

### Phase 0 — Foundations (unblocks everything, speeds iteration)

**P0.1 — Determinism core** ✅ *(commit `9e53977`)* *(TODO item 4, partial; enables 9, 13)*
- Add `RunSeed` resource, seeded at startup (random per launch → stable for the
  whole session and all its loops; can later be overridden for shareable/
  debuggable seeds).
- Derive sub-seeds; purge `from_entropy()` from gameplay code.
- Introduce a `FixedUpdate` schedule for adversary movement/decisions.
- *Acceptance:* same launch → identical guard routes every loop; logging the
  seed reproduces a level.

**P0.2 — Game states + menus** ✅ *(commit `c713494`)* *(TODO item 1)*
- Add a Bevy `States` enum: `Boot → MainMenu → Loading → Playing → Paused →
  GameOver/Win`.
- Gate existing systems on `in_state(Playing)`; the PostStartup spawns become
  `OnEnter(Playing)` so a loop/level can be (re)entered cleanly.
- Minimal menu UI (start, quit-on-native, restart). Pause on Esc.
- *Risk:* the current `Startup`/`PostStartup` ordering (dungeon → player/props/
  adversary) must be preserved as `OnEnter(Playing)` system sets with explicit
  `.chain()`/ordering.

**P0.3 — Loading state** ✅ *(commit `9ff86ec`)* *(TODO item 2)*
- While in `Loading`, wait for all GLB scene handles to reach `LoadState::Loaded`
  before transitioning to `Playing` (avoids the white/untextured first frames,
  important for the slower web build). Simple progress text HUD.

**P0.4 — Debug tooling** ✅ *(TODO item 3)*
- A `DebugPlugin` behind an F3 toggle: overlay (FPS/seed/state/entity counts),
  F4 vision-cone toggle, F5 top-down floorplan overlay (the future home of the
  room-tag overlay), F6 force loop-reset (drives the new `CloseLoop` message that
  Shift+R now also routes through), F7 free-fly camera (IJKL pan, U/O raise).
- `camera.rs`/`adversary.rs` read a shared `DebugSettings` via `Option<Res<…>>`
  so they stay independent of the debug plugin.
- *Deferred (need later phases):* room-tag colouring of the map overlay waits on
  P1.1's `room_of`; live in-game seed entry remains the `GAME_SEED` env override
  (logged at launch) rather than a UI field.

### Phase 1 — Structured world

**P1.1 — Level source abstraction + room types** ⬜ *(TODO item 10)*
- Implement decision **A**: `LevelSource` trait, `Level`/`Room`/`RoomKind`,
  `HybridSource` as default, `DungeonMap.room_of`.
- Room kinds: `Start`, `Lobby`, `GameTables`, `Vault`, `Security`, `Service`.
- Spawn rules become room-aware: coins/props (`props.rs`) scatter by room kind
  (chips on game tables, loot in the vault), adversaries (`adversary.rs`) spawn
  in/around `Security`.
- *Acceptance:* a generated level always contains exactly one Vault and one
  Security room; **every** room is reachable from the Start/spawn, and sealed
  rooms are reachable only through their doorways (no drilled wall rings).

### Phase 2 — Adversary variety

**P2.1 — Adversary kinds** ⬜ *(TODO item 4)*
- Refactor `Adversary` so behaviour is data-driven by a `kind`, all RNG seeded
  per decision C:
  - **Static guard** — fixed post, cone sweeps; doesn't move until an *interest
    threshold* is exceeded, then chases.
  - **Patrolling guard** — follows a set patrol route (derived from room
    geometry/seed), cone sweeping as it walks, same interest threshold.
  - **Wandering guard** — closest to today's behaviour.
- Introduce an **interest/suspicion meter**: time-in-cone accumulates interest;
  crossing the threshold triggers `Chase` (replaces today's instant lock-on).
  This makes peeking/partial exposure survivable and is the hook for stealth.
- Appearance/positioning vary by kind (different GLB / spawn room).
- *Acceptance:* with a fixed seed, each guard kind reproduces identical
  patrol/sweep timing every loop; interest build-up is deterministic.

### Phase 3 — Heist mechanics

**P3.1 — Persistence layer** ⬜ *(TODO item 9, infra)*
- Implement decision **B**: `Persistent` resource + `PersistPolicy` component;
  `LoopReset` observers honour the policy.

**P3.2 — Vault + employee puzzle** ⬜ *(TODO item 11)*
- A locked Vault requiring a code. An **employee NPC** (scripted, non-reactive →
  record/replay or a simple state machine) walks a route and won't leave/reveal
  the code until a condition is met (e.g. a distraction triggered elsewhere, or
  observed entering the code).
- Learning the code sets a `vault_code_known` fact in `Persistent`; once known,
  the player can open the vault directly in a later loop — the worked example of
  knowledge persistence.

### Phase 4 — Presentation & fidelity

**P4.1 — Animated/rigged object recording** ⬜ *(TODO item 13)*
- Today `Sample` records only `translation`+`rotation`. Decide per actor type:
  - *Non-reactive scripted actors* (employee, opening doors, slot machines):
    record an **animation/state channel** alongside the transform, or replay a
    deterministic clip keyed to loop time. Fits decision C cleanly.
  - *The player/ghosts*: if a rig/walk-cycle is added, extend `Sample` to carry
    the active animation + normalized playback time so ghosts animate correctly.
- *Note:* this is why determinism + record/replay were settled early — item 13 is
  mostly a consequence of those choices, not a separate system.

**P4.2 — Resize to browser window** ⬜ *(TODO item 12)*
- Drive the camera's `ScalingMode`/viewport from the window size so the web build
  fills (and reflows with) the browser window. Verify against `build-web.sh`
  output and the GitHub Pages deploy.

---

## Dependency summary

```
P0.1 determinism ─┬─> P2.1 adversary kinds ─┐
                  ├─> P3.3 loop experiments  ├─> P4.1 animation recording
P0.2 states ──────┼─> P0.3 loading           │
                  └─> P3.* heist mechanics ───┘
P1.1 rooms ───────────> P2.1, P3.2 vault/employee
P3.1 persistence ─────> P3.2, P3.3
P0.4 debug ─ (supports all later tuning)
P4.2 resize ─ (independent, do anytime)
```
