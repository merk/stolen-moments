//! Level *sources*: pluggable generators that produce a fully-described
//! [`Level`] from a seed.
//!
//! Decoupling generation from the runtime [`LevelMap`] lets the same downstream
//! systems work whether a level is purely procedural ([`NoiseSource`]) or a
//! noise/room hybrid ([`HybridSource`], the current default). A future
//! file-backed source can implement [`LevelSource`] with no engine changes.

use rand::SeedableRng;
use rand::rngs::SmallRng;

use super::connect::carve_connect;
use super::map::{LevelMap, RoomId, RoomKind, SpawnPoint};
use super::noise::{border_mask, build_noise_map, find_central_floor};
use super::rooms::stamp_rooms;

/// A fully-described level ready to be turned into resources and meshes.
pub struct Level {
    pub map: LevelMap,
    pub spawn: SpawnPoint,
}

/// Produces a [`Level`] from a seed. Implemented by procedural, hybrid, and
/// (later) file-backed sources.
pub trait LevelSource {
    fn build(&self, seed: u64) -> Level;
}

/// Pure noise caverns with no semantic rooms — the original behaviour, kept as
/// the "organic back-of-house" filler and as a reference source. Not currently
/// wired into the build; [`HybridSource`] is the default.
#[allow(dead_code)]
pub struct NoiseSource;

impl LevelSource for NoiseSource {
    fn build(&self, seed: u64) -> Level {
        let mut map = build_noise_map(seed as u32);
        let start = find_central_floor(&map);
        let protected = border_mask();
        carve_connect(&mut map, &protected, start);
        let world = map.tile_to_world(start.0, start.1);
        Level {
            map,
            spawn: SpawnPoint { tile: start, world },
        }
    }
}

/// Noise caverns with typed rooms stamped in — the current default.
pub struct HybridSource;

impl LevelSource for HybridSource {
    fn build(&self, seed: u64) -> Level {
        let mut rng = SmallRng::seed_from_u64(seed);
        let mut map = build_noise_map(seed as u32);
        let mut protected = border_mask();

        // Place and stamp the typed rooms, then tag their interior floor tiles
        // into the parallel `room_of` lookup.
        let rooms = stamp_rooms(&mut map, &mut protected, &mut rng);
        for (i, room) in rooms.iter().enumerate() {
            for &(x, y) in &room.tiles {
                map.assign_room(x, y, RoomId(i));
            }
        }

        // Spawn at the Start room's centre; connect everything from there.
        let start = rooms
            .iter()
            .find(|r| r.kind == RoomKind::Start)
            .map(|r| r.rect.center())
            .unwrap_or_else(|| find_central_floor(&map));
        map.set_rooms(rooms);

        carve_connect(&mut map, &protected, start);

        let world = map.tile_to_world(start.0, start.1);
        Level {
            map,
            spawn: SpawnPoint { tile: start, world },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::connect::label_components;
    use crate::level::map::Tile;

    /// Seeds to sweep so the contracts hold across many generated layouts.
    const SEEDS: std::ops::Range<u64> = 0..40;

    #[test]
    fn exactly_one_vault_and_one_security() {
        for seed in SEEDS {
            let level = HybridSource.build(seed);
            let vaults = level
                .map
                .rooms()
                .iter()
                .filter(|r| r.kind == RoomKind::Vault)
                .count();
            let security = level
                .map
                .rooms()
                .iter()
                .filter(|r| r.kind == RoomKind::Security)
                .count();
            assert_eq!(vaults, 1, "seed {seed}: expected exactly one Vault");
            assert_eq!(security, 1, "seed {seed}: expected exactly one Security");
        }
    }

    #[test]
    fn every_room_reachable_from_spawn() {
        for seed in SEEDS {
            let level = HybridSource.build(seed);
            let w = level.map.width;
            let labels = label_components(&level.map);
            let main = labels[level.spawn.tile.1 * w + level.spawn.tile.0]
                .expect("spawn tile must be floor");
            for room in level.map.rooms() {
                for &(x, y) in &room.tiles {
                    assert_eq!(
                        labels[y * w + x],
                        Some(main),
                        "seed {seed}: {:?} tile ({x},{y}) not reachable from spawn",
                        room.kind
                    );
                }
            }
        }
    }

    #[test]
    fn sealed_rooms_open_only_at_their_doorway() {
        for seed in SEEDS {
            let level = HybridSource.build(seed);
            for room in level.map.rooms().iter().filter(|r| r.kind.sealed()) {
                let openings = room
                    .rect
                    .perimeter()
                    .filter(|&(x, y)| level.map.get(x, y) == Tile::Floor)
                    .count();
                assert_eq!(
                    openings, 1,
                    "seed {seed}: sealed {:?} should expose exactly one doorway",
                    room.kind
                );
            }
        }
    }
}
