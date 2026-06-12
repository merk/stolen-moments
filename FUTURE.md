# Future concerns

Ideas worth building *later* — captured so they aren't lost, but deliberately
kept out of the active [`PLAN.md`](./PLAN.md) so the near-term work stays focused.

Each concern lives in its own file under [`docs/future/`](./docs/future):

- [Facts & requirements: a data-driven objective system](./docs/future/facts-and-requirements.md)
  — generalise the flat persistence facts into a tiny composable rules system so
  designers author quests/gates from RON, no recompile.
- [Gameplay objects as composed primitives](./docs/future/gameplay-primitives.md)
  — extract the cross-cutting engine bits (loop-reset snapshot, scene tint,
  proximity, model load, room lookup, grid nav) that the heist modules each
  re-implement, on the rule of three.
- [A configuration system for tuning constants](./docs/future/configuration-system.md)
  — pull scattered tuning `const`s into a RON config (per-module resources) plus
  per-component overrides defaulted from it, without breaking the seeded sim.
- [Ecosystem crates for navigation & guard AI](./docs/future/navigation-ai-crates.md)
  — a map of the pathfinding/FSM/spatial crates that cover the hand-rolled
  adversary system, filtered by the determinism constraint.
