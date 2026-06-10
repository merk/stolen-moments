//! Cross-loop persistence: the small amount of state that must survive a
//! [`LoopReset`](crate::time_loop::LoopReset).
//!
//! The time loop resets the world every run, but a heist is built on *learning*
//! — a code discovered on one run should still be known on the next. Knowledge
//! lives in the [`Persistent`] fact set, which `LoopReset` deliberately never
//! touches; it is cleared only when a brand-new level is built (a fresh game or
//! a return to the menu and back), mirroring `time_loop`'s own level reset.
//!
//! [`PersistPolicy`] is the parallel hook for *physical* state: an entity tags
//! itself with how it should behave across loops. For now the only worked
//! examples are the persistent `VaultCodeKnown` fact and the vault door
//! (`ResetEachLoop`); richer per-entity persistence builds on this later.

use std::collections::HashSet;

use bevy::prelude::*;

use crate::state::GameState;

/// A piece of knowledge that, once learned, persists across loops within a
/// session. Extended as more of the heist becomes learnable.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Fact {
    /// The player has discovered the vault code and can open the vault directly.
    VaultCodeKnown,
}

/// Knowledge that survives [`LoopReset`](crate::time_loop::LoopReset). Reset only
/// when a fresh level is built (see [`clear_on_new_level`]).
#[derive(Resource, Default)]
pub struct Persistent {
    facts: HashSet<Fact>,
}

impl Persistent {
    /// Whether `fact` has been learned this session.
    pub fn knows(&self, fact: Fact) -> bool {
        self.facts.contains(&fact)
    }

    /// Record `fact` as known. Returns `true` if it was newly learned (so callers
    /// can fire one-shot feedback), `false` if it was already known.
    pub fn learn(&mut self, fact: Fact) -> bool {
        self.facts.insert(fact)
    }
}

/// How an entity's *physical* state should behave when a loop resets. Consulted
/// by the per-entity reset logic that owns the entity.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum PersistPolicy {
    /// Reverts to its spawn state on every loop (e.g. a vault door re-locks).
    ResetEachLoop,
    /// Keeps its current state across loops once changed. Plumbing for later
    /// heist objects; no consumer yet.
    #[allow(dead_code)]
    KeepForever,
}

pub struct PersistencePlugin;

impl Plugin for PersistencePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Persistent>()
            // A fresh level wipes learned facts; LoopReset deliberately does not.
            .add_systems(OnEnter(GameState::Loading), clear_on_new_level);
    }
}

/// Clear all learned facts when a new level is built, so a fresh game (or a
/// return to the menu and back) starts with nothing known.
fn clear_on_new_level(mut persistent: ResMut<Persistent>) {
    persistent.facts.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learn_is_idempotent_and_reports_novelty() {
        let mut p = Persistent::default();
        assert!(!p.knows(Fact::VaultCodeKnown));
        assert!(p.learn(Fact::VaultCodeKnown), "first learn is novel");
        assert!(p.knows(Fact::VaultCodeKnown));
        assert!(
            !p.learn(Fact::VaultCodeKnown),
            "re-learning a known fact is not novel"
        );
    }
}
