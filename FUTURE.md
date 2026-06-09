# Future concerns

Ideas worth building *later* — captured so they aren't lost, but deliberately
kept out of the active [`PLAN.md`](./PLAN.md) so the near-term work stays focused.

---

## Facts & requirements: a data-driven objective system

**Motivation.** The near-term persistence layer (PLAN decision B) is a flat set
of boolean facts with a per-fact persistence scope — enough for "the vault code
is known / not known." It does **not** compose: a level designer can't express
"the vault key is made of 2 (or 3) components" without a code change. This
section generalises facts into a tiny rules system — the pattern big engines use
for quests / achievements / crafting prerequisites — so designers get real power
from data alone, with no recompile.

### Two primitives

**1. Facts** — atomic named truths, each with a *scope* (this is where the
persistence policy lives):

```rust
struct FactDef { id: FactId, scope: FactScope }   // authored in data
enum FactScope { PerLoop, Persistent }            // cleared on LoopReset, or kept forever

#[derive(Resource, Default)]
struct FactStore { set: HashSet<FactId> }          // currently-true facts
```

`fragment_a_taken` might be `Persistent` (knowledge you keep across loops);
`distraction_active` is `PerLoop`. `LoopReset` clears only `PerLoop` facts — the
whole persistence policy becomes per-fact and designer-controlled.

**2. Requirements** — a boolean *expression* over facts; this is the piece that
composes:

```rust
enum Requirement {
    Fact(FactId),
    All(Vec<Requirement>),            // AND
    Any(Vec<Requirement>),            // OR
    Not(Box<Requirement>),
    AtLeast(u32, Vec<Requirement>),   // "k of n"
}
impl Requirement { fn eval(&self, facts: &FactStore) -> bool { /* pure */ } }
```

A gated thing holds a requirement and an action to run when it flips true:

```rust
#[derive(Component)]
struct Gate { requires: Requirement, on_satisfied: GateAction } // GateAction::Unlock, ...
```

Facts get *set* by data-tagged triggers rather than bespoke code:

```rust
#[derive(Component)] struct SetFactOnPickup(FactId);   // on each fragment item
#[derive(Component)] struct SetFactOnObserve(FactId);  // e.g. watch employee enter the code
```

### The designer's 2-or-3-component vault, as pure data

```ron
facts: [
    (id: "frag_a", scope: Persistent),
    (id: "frag_b", scope: Persistent),
    (id: "frag_c", scope: Persistent),
]
items: [
    (asset: "key_fragment.glb", room: GameTables, set_fact_on_pickup: "frag_a"),
    (asset: "key_fragment.glb", room: Security,   set_fact_on_pickup: "frag_b"),
    (asset: "key_fragment.glb", room: Lobby,      set_fact_on_pickup: "frag_c"),
]
gates: [
    ( target: Vault, on_satisfied: Unlock,
      requires: All([Fact("frag_a"), Fact("frag_b"), Fact("frag_c")]) ),
]
```

- "Any 2 of 3" → change one line: `requires: AtLeast(2, [Fact("frag_a"), Fact("frag_b"), Fact("frag_c")])`.
- Allow a directly-observed code as a shortcut → `Any([ All([...3 frags]), Fact("code_observed") ])`.
- Adding a fourth fragment or rewiring the logic is a **data edit, no recompile**.

### Why it's cheap and fits the codebase

- Three small pieces: a `FactStore` resource, a `Requirement` enum with a **pure
  `eval`** (no `App` needed → unit-testable, matching the module guidelines in
  `CLAUDE.md`), and one system that re-evaluates each `Gate` when facts change.
  Lives in a small `facts.rs` / `objectives.rs`.
- **Subsumes** PLAN decision B rather than running beside it — persistence is
  just "which facts have `Persistent` scope."
- Deterministic: fact changes are discrete events and `eval` is pure, so it slots
  into the seeded sim cleanly.
- Loads from the same RON path as the `FileSource` levels (PLAN P1), so
  facts/gates are authored alongside the level — designers get one file.

### Scope control

Keep v1 boolean-only. The natural extension is **counters/values** (a
`Counter(FactId, u32)` requirement variant + a `Counters` map, for
`Counter("chips") >= 8`-style tallies) — but boolean + `AtLeast(k, …)` already
covers "N of M components," so don't build counters until a mechanic needs a true
tally.

### Migration path from the near-term plumbing

PLAN decision B ships a flat `Persistent { facts: HashSet<FactId> }` plus the
vault-code worked example. To adopt this system later: rename it `FactStore`, add
the `scope` table (existing facts → `Persistent`), introduce `Requirement` +
`Gate`, and replace the hardcoded vault-open check with a `Gate`. No gameplay
data is lost in the transition.
