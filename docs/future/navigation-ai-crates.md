# Ecosystem crates for navigation & guard AI

**Motivation.** The adversary system (`adversary/`) is entirely hand-rolled:
grid BFS pathfinding (`path.rs`), a swept vision cone with grid line-of-sight
(`vision.rs`), and a `Patrol`/`Chase`/`Search` state machine driven by an
interest meter (`behaviour.rs`). That was the right call — but it's worth
recording which ecosystem crates cover this ground, so a future "should we adopt
one" decision starts from the map rather than a blank page. **Bevy core ships
nothing here** (no `bevy::navigation`/`bevy::ai`); navigation, perception, and
AI are all third-party, so nothing was skipped by hand-rolling.

### The hard constraint that filters the list: determinism
The time-loop's guarantee is that guard behaviour is a **pure function of seed +
FixedUpdate tick**, so ghosts replay identically. That single requirement
disqualifies most heavyweight options: navmesh baking, float-based path solvers
(Polyanya), and physics raycasts all introduce float nondeterminism or async
work that would desync replays. Our integer grid + BFS + tick-stepped FSM is
deterministic *by construction*. **Any crate adopted here must preserve that** —
evaluate every candidate against "does it stay bit-for-bit reproducible across
runs of the same seed."

### Pathfinding (today: our `bfs_path`)
| Crate | Verdict |
|---|---|
| **`pathfinding`** (samueltardieu) | **The one genuinely worth adopting.** Generic, battle-tested A*/Dijkstra/BFS over any graph; not Bevy-coupled, so no version-lag risk. `bfs_path` → `astar(...)` cuts chase-repath cost, stays integer-exact and deterministic. Slots behind the existing `nav`-module router signature (see the "Grid nav + waypoints" primitive in [gameplay-primitives](./gameplay-primitives.md)). |
| **`vleue_navigator`** (navmesh, Polyanya) | Overkill + float-based. Built for continuous geometry, not a 1-unit tile grid. Determinism risk. |
| **`oxidized_navigation`** (Recast/Detour) | Same — 3D navmesh generation for free-form worlds. Wrong shape for a grid. |

### Component-based AI structure (today: our `Mode` enum + interest meter)
What `behaviour.rs` hand-rolls is, formally, an FSM with a utility score. Two
**component-based** crates model exactly this and would express the state graph
declaratively instead of via a `match`:

- **`seldom_state`** — a component state machine: states are components, transitions
  are triggers. Our `Patrol`/`Chase`/`Search` modes and their guards (interest
  thresholds, `search_timer` expiry, re-sighting) map onto it directly. The
  closest fit if the state graph keeps growing.
- **`big-brain`** — component-based *utility* AI: "scorers" compute a value
  (our `interest_gain`), a "picker" chooses an action (chase/search/patrol). Our
  interest→threshold→chase logic is textbook big-brain.
- **`bevy_behave`** / behaviour-tree crates — more machinery than a three-state
  guard needs; note only for when behaviour branches substantially.

**Trade-off:** at three states the hand-rolled `match` is arguably *clearer* than
a DSL, and these crates add a dependency plus a determinism audit (their internal
ordering/iteration must be tick-stable). Adopt only when the state machine earns
it — e.g. alert tiers, guard-to-guard alarm propagation, squad coordination, or
per-kind behaviour trees — not for the current size.

### Supporting components
- **`bevy_spatial`** — component-based spatial index (KD-tree) for nearest-neighbour
  queries. Only pays off if the prey/target list grows large; today we scan a
  handful of entities linearly (`update_adversaries`), so it's premature.
- **Physics raycasts** (`avian3d` / `bevy_rapier3d`) could replace the grid-march
  LOS in `vision.rs` — but we run no physics, and float raycasts fight
  determinism. Keep the grid march.

### Scope control / when to revisit
- **First reach:** `pathfinding` for A*, taken *with* the `nav`-module extraction
  in [gameplay-primitives](./gameplay-primitives.md) — one refactor, keeps
  determinism, deletes our bespoke BFS.
- **Second reach:** `seldom_state` *only if* the guard FSM outgrows a readable
  `match` (alarm tiers / shared alerts / coordination).
- **Defer:** navmesh crates, `big-brain`, `bevy_spatial`, physics LOS — heavier
  than this game needs and/or determinism liabilities.
- **Caveat — Bevy version lag:** the project tracks a very fresh Bevy (`0.18`);
  ecosystem crates routinely trail engine releases. Before adopting *any* of the
  Bevy-coupled ones, confirm a `0.18`-compatible release exists. The
  non-Bevy-coupled `pathfinding` crate sidesteps this entirely — another point in
  its favour.
