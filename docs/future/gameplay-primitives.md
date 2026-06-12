# Gameplay objects as composed primitives

**Motivation.** P3.2 shipped `vault.rs` and `employee.rs` as bespoke feature
modules. That's deliberate (they're the worked example the
[facts/requirements](./facts-and-requirements.md) system later subsumes) — but
building them surfaced cross-cutting *engine* primitives that several modules now
each re-implement. The heist objects themselves should eventually be thin
composition: a door is `Interactable + Gate{Unlock} + SpawnSnapshot`; the code
note is `Pickup + SetFactOnPickup`. Just as important, the underlying primitives
below should be **shared**, not copied — extracted on the rule of three (and once
the mechanic is validated by playtest), never speculatively.

### Recurring primitives (with the duplication that motivates each)

| Primitive | Shape | Current consumers | Extract to |
|---|---|---|---|
| **Loop-reset snapshot** | `On<LoopReset>` restoring a recorded spawn state | `coins::reset_coins`, `adversary::reset_adversaries` (via `Post`), `vault::relock_vault`, `employee::reset_employee` + `reset_note` (5) | `SpawnSnapshot { transform, visible }` + one generic reset system, honouring `PersistPolicy` (skip `KeepForever`). Adversary keeps its richer `Post` bits, shares the transform/visibility restore. |
| **Scene material tint** | clone a scene's materials on `SceneInstanceReady` and recolour | `time_loop::make_ghost_transparent`, `employee::tint_employee` (2) | `SceneTint { base, emissive_factor, alpha }` + shared observer. |
| **Proximity trigger** | XZ-plane radius test player↔thing | `coins::collect_coins`, `employee::pickup_note`, `vault::within_range` (3) | a `near_xz(a, b, r)` helper now; `Pickup` / `Interactable` components later (the data layer's `SetFactOnPickup` rides on this). |
| **Face movement dir** | `looking_to(-dir, Y)` (+ optional slerp) | `player`, `adversary` (×2), `employee` (5) | a `face_dir(dir, turn_speed?)` helper. |
| **GLB load + track** | `loading.track(assets.load(GltfAssetLabel::Scene(0).from_asset("Models/GLB format/…")))` | `player`, `adversary`, `props`, `vault`, `employee`, `level::render`, `time_loop` (~9 sites) | `load_model(&assets, &mut loading, "name.glb")` — also centralises the asset-path prefix. |
| **Room-by-kind lookup** | `map.rooms().iter().find(\|r\| r.kind == …)` | `vault`, `employee`, `adversary::spawn`, `level::source` (6) | `LevelMap::room(kind)` / `rooms_of(kind)`. |
| **Grid nav + waypoints** | BFS routing; "advance toward waypoint, face dir" | `adversary` (`bfs_path`, `random_walkable`, nav step), `employee::walk_employee` (straight-line re-impl) | a shared `nav` module: the pure routers + a `Waypoints { points, index, speed }` follower. |

### Where they'd live
- a small `common` (or `gameplay`) module for the cross-cutting ECS bits
  (`SpawnSnapshot`/reset, `SceneTint`, proximity, `load_model`);
- a `nav` module lifting the pure grid helpers out of `adversary`;
- `LevelMap::room(kind)` on the map itself.

### Sequencing / scope control
- Extract only patterns with **≥2 real consumers**: the snapshot-reset (5), GLB
  load (9), room lookup (6), facing (5), proximity (3), and scene tint (2)
  qualify today. The `Door`/`Interactable`-*as-components* blocks have a single
  consumer (the vault) — defer until a second appears.
- Do this **with** (or just before) the
  [facts/requirements](./facts-and-requirements.md) work — same refactor, and
  `vault`/`employee` then collapse into data plus a couple of generic components.
- Playtest the heist mechanic before freezing the gameplay-object abstractions.

### CLAUDE.md follow-up (apply once the primitives land)
Once `common`/`nav` exist, add to the **Module guidelines**: *"Cross-cutting
mechanics — loop-reset snapshots, scene tinting, proximity tests, model loading,
grid nav — live as shared primitives in `common`/`nav`; a feature module
composes these rather than re-implementing them. When a second consumer of a
pattern appears, lift it into the primitive instead of copying it."* Not added
now: it would point at modules that don't yet exist.
