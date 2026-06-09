//! Whole-simulation determinism: one master seed per launch, with stable
//! sub-seeds derived per subsystem.
//!
//! Every gameplay RNG (dungeon gen, prop scatter, adversary behaviour) draws
//! from a seed derived off [`RunSeed`] rather than the OS entropy pool, so a
//! given launch reproduces identical levels and guard routes on every loop.
//! Logging the seed (or setting `GAME_SEED`) reproduces a whole session.

use bevy::prelude::*;

/// The master seed for this session. Chosen once at launch (random, or from the
/// `GAME_SEED` env var) and held constant for every loop after.
///
/// Subsystems never use this directly; they call [`RunSeed::derive`] /
/// [`RunSeed::derive_indexed`] to get an independent, stable sub-seed.
#[derive(Resource, Clone, Copy, Debug)]
pub struct RunSeed(pub u64);

impl RunSeed {
    /// A stable sub-seed for a named subsystem (e.g. `"dungeon"`, `"props"`).
    ///
    /// Deterministic across runs and Rust versions: an FNV-1a hash of the label
    /// mixed with the master seed through splitmix64. The same `(master, label)`
    /// always yields the same value, and distinct labels are well-separated.
    pub fn derive(&self, label: &str) -> u64 {
        splitmix64(self.0 ^ fnv1a(label.as_bytes()))
    }

    /// Like [`derive`](Self::derive) but for one of many instances of the same
    /// subsystem (e.g. adversary `i`), keeping each instance's stream distinct.
    pub fn derive_indexed(&self, label: &str, index: usize) -> u64 {
        splitmix64(self.derive(label) ^ splitmix64(index as u64))
    }
}

/// Inserts [`RunSeed`] before any `Startup` system runs and logs the chosen
/// seed. Override with the `GAME_SEED` env var to replay a specific session.
pub struct SeedPlugin;

impl Plugin for SeedPlugin {
    fn build(&self, app: &mut App) {
        let seed = pick_seed();
        info!("RunSeed = {seed} (set GAME_SEED to reproduce)");
        app.insert_resource(RunSeed(seed));
    }
}

/// Honour a `GAME_SEED` override when present and parseable, else draw a fresh
/// random master seed. (Env vars are a no-op on wasm, which always randomises.)
fn pick_seed() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    if let Ok(raw) = std::env::var("GAME_SEED") {
        match raw.trim().parse::<u64>() {
            Ok(seed) => return seed,
            Err(_) => warn!("GAME_SEED='{raw}' is not a u64; using a random seed"),
        }
    }
    rand::random()
}

/// FNV-1a hash of a byte string. Tiny, allocation-free, and stable everywhere.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The splitmix64 finalising mixer — scrambles a seed so derived streams don't
/// share structure with the master or each other.
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_is_deterministic() {
        let s = RunSeed(12345);
        assert_eq!(s.derive("dungeon"), s.derive("dungeon"));
        assert_eq!(
            s.derive_indexed("adversary", 3),
            s.derive_indexed("adversary", 3)
        );
    }

    #[test]
    fn distinct_labels_and_indices_differ() {
        let s = RunSeed(12345);
        assert_ne!(s.derive("dungeon"), s.derive("props"));
        assert_ne!(s.derive("adversary"), s.derive_indexed("adversary", 0));
        assert_ne!(
            s.derive_indexed("adversary", 0),
            s.derive_indexed("adversary", 1)
        );
    }

    #[test]
    fn distinct_masters_differ() {
        assert_ne!(RunSeed(1).derive("dungeon"), RunSeed(2).derive("dungeon"));
    }
}
