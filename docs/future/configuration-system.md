# A configuration system for tuning constants

**Motivation.** Tuning values live as `const`s scattered across ~20 modules —
`player::MOVE_SPEED`, `camera::FOLLOW_SPEED`/`VIEW_HEIGHT`, `coins::PICKUP_RADIUS`,
`catch::{CONTACT_RADIUS, GRAB_FILL_TIME, GRAB_DRAIN_TIME}`, `time_loop::{GHOST_ALPHA,
TRAIL_*}`, `vault::INTERACT_RANGE`, `employee::{EMPLOYEE_SPEED, …}`, and the large
guard block in `adversary/mod.rs` (`PATROL_SPEED`, `CHASE_SPEED`, the whole
interest curve, scan cadence, search timings, …). Changing any of them means an
edit-and-recompile cycle, and there's no single place to see or sweep the game's
feel. This section captures a config system that pulls these knobs out into data,
with sensible per-entity overrides — **without** turning every structural constant
into config or breaking the seeded sim.

### Not everything is a "knob"
Separate two kinds of constant before touching anything:

- **Tuning knobs** — floats that designers want to sweep for *feel*: speeds, radii,
  durations, the interest curve, alpha/colour tints. These belong in config.
- **Structural facts** — things that are part of *what the thing is*, not how it
  feels: `billboard::{ATLAS_W, ATLAS_H, CELL}` (fixed by the sprite sheet), asset
  path strings, `rooms::ROOM_PLAN`, `adversary::GUARD_KINDS`, the `cone` colour
  *identities*. These stay as code/data-of-their-own-kind; promoting them to a
  config file adds ceremony with no payoff and invites invalid states.

Only the first group moves. When in doubt, ask "would a non-programmer ever want
to change this to make the game feel better?"

### Two delivery mechanisms (the user's framing)

**1. A config asset, loaded into resources.** The Bevy-idiomatic path is a **RON**
file deserialised with `serde` — Bevy already uses RON for scenes, and
`bevy_common_assets`'s `RonAssetPlugin` gives a typed `AssetLoader` for free (so it
hot-reloads and rides the normal asset pipeline). The same RON pipeline the
`FileSource` levels use (PLAN P1) and the [facts & requirements](./facts-and-requirements.md)
system want — config is authored alongside the level, one file for designers.

```ron
// assets/config/game.ron
(
    player:  ( move_speed: 5.0, radius: 0.3, turn_speed: 12.0 ),
    camera:  ( view_height: 18.0, follow_speed: 6.0 ),
    catch:   ( contact_radius: 0.6, grab_fill_time: 1.2, grab_drain_time: 0.6 ),
    guard:   ( patrol_speed: 2.6, chase_speed: 4.2, interest_threshold: 0.5, /* … */ ),
)
```

Each module owns its slice as a `Resource` (mirroring today's `CatchConfig`), and
the loader populates them. A module reads `cfg.move_speed` instead of a `const`;
the `const` becomes the `Default` impl, so the file is optional and missing keys
fall back to today's values.

**2. Per-component overrides, defaulted from config.** A component carries the
tuning fields that can vary *per entity*; the spawning system fills them from the
config resource as the default, but data (the RON level, or a scene) may override
per spawn. This is exactly the shape `GuardKind` already hints at — its
`speed_factor()` scales a shared base — generalised so a level can place a *fast*
guard without a new enum variant:

```rust
#[derive(Component)]
struct Movement { speed: f32, turn_speed: f32 }

// spawn: default from config, but let placed data override
let m = spawn_def.movement.unwrap_or(cfg.guard.movement());
commands.spawn((guard_scene, m, /* … */));
```

So: **global feel** comes from the config resource; **per-entity variation** lives
on the component, defaulted from that resource by whatever spawned it. Most
modules only need mechanism 1; reach for 2 only where per-entity variation already
exists (guards today; props/items later).

### Determinism caveat
Config is read into resources **once at load, before the sim starts**, and the
values feed the seeded `FixedUpdate` sim as plain numbers — so a given config +
seed is still bit-for-bit reproducible. Two rules keep it that way: (a) don't
hot-reload config *into a running loop* (apply on the next `LoopReset`/reload, not
mid-tick), and (b) treat the config as part of the run's identity — a ghost
recorded under one tuning isn't guaranteed to replay under another, so bundle the
config (or its hash) with saved runs if that ever matters.

### Why it fits the codebase
- Mirrors the existing `CatchConfig` resource pattern — this generalises that one
  module's "owns its knobs" approach to the rest of the crate, rather than
  inventing a new idiom.
- Keeps modules self-contained (CLAUDE.md guideline): each owns its config struct
  and its `Default`; there's no monolithic settings god-object, just a loader that
  fans a parsed file out to per-module resources.
- Pure `Default` impls stay unit-testable with no `App`.

### Scope control
- **v1:** one RON file → per-module config resources, `Default` = today's consts,
  file optional. No per-component overrides, no hot-reload. This alone kills the
  recompile-to-tune loop.
- **v2:** opt-in hot-reload (apply at loop boundaries) for live tuning during
  playtests.
- **v3:** per-component overrides (mechanism 2) **only** where the level format
  needs to vary an entity — co-develop with the `FileSource` level work and the
  gameplay-primitive extraction so spawners thread config through once.
- **Don't:** promote structural constants, build a settings UI, or add a config
  layer to a module that has a single hard-coded value nobody tunes.
