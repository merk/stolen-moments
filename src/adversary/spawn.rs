//! Guard placement: drop one of each [`GuardKind`] into the level, posting
//! static guards in the Security room and scattering the roaming kinds a safe
//! distance from the player's spawn. All choices draw from seeded RNG streams so
//! the roster is identical across runs of the same seed.

use bevy::prelude::*;
use rand::rngs::SmallRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};

use crate::billboard::OverlayAssets;
use crate::level::{LevelMap, RoomKind, SpawnPoint};
use crate::loading::LoadingAssets;
use crate::seed::RunSeed;
use crate::state::InGame;

use super::overlay::attach_overlays;
use super::path::random_walkable;
use super::vision::horizontal;
use super::{
    Adversary, Awareness, GUARD_KINDS, GuardKind, Mode, Navigation, PATROL_RADIUS,
    PATROL_WAYPOINTS, PatrolRoute, Post, SPAWN_CLEARANCE, Vision, Wander,
};

pub(super) fn spawn_adversaries(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut loading: ResMut<LoadingAssets>,
    overlays: Res<OverlayAssets>,
    map: Res<LevelMap>,
    spawn: Res<SpawnPoint>,
    run_seed: Res<RunSeed>,
) {
    let scene = loading.track(
        asset_server
            .load(GltfAssetLabel::Scene(0).from_asset("Models/GLB format/character-orc.glb")),
    );

    // One RNG seeds placement/initial facing; each guard then carries its own
    // seeded stream so they wander/sweep independently yet reproducibly.
    let mut rng = SmallRng::seed_from_u64(run_seed.derive("adversary.spawn"));
    let (sx, sy) = (spawn.tile.0 as i32, spawn.tile.1 as i32);

    // Static guards post up in the Security room; a shuffled pool hands out
    // distinct posts. Without one (e.g. a roomless source), they fall back to a
    // far tile like the roaming kinds.
    let mut security: Vec<(usize, usize)> = map
        .rooms()
        .iter()
        .filter(|r| r.kind == RoomKind::Security)
        .flat_map(|r| r.tiles.iter().copied())
        .collect();
    security.shuffle(&mut rng);
    let mut security = security.into_iter();

    for (i, &kind) in GUARD_KINDS.iter().enumerate() {
        let tile = match kind {
            GuardKind::Static => security
                .next()
                .unwrap_or_else(|| far_tile(&map, &mut rng, sx, sy)),
            GuardKind::Patrolling | GuardKind::Wandering => far_tile(&map, &mut rng, sx, sy),
        };

        let world = map.tile_to_world(tile.0, tile.1);
        // A posted static guard watches something interesting — the doorway of
        // its room, or failing that the nearest entrance / the player's spawn.
        // Roaming kinds just start on a seeded random heading.
        let heading = match kind {
            GuardKind::Static => interesting_facing(&map, tile, spawn.tile),
            GuardKind::Patrolling | GuardKind::Wandering => {
                let angle = rng.gen_range(0.0..std::f32::consts::TAU);
                Vec3::new(angle.cos(), 0.0, angle.sin())
            }
        };
        let sweep_phase = rng.gen_range(0.0..std::f32::consts::TAU);

        let mut entity = commands.spawn((
            Adversary,
            SceneRoot(scene.clone()),
            Transform::from_translation(world)
                .with_scale(Vec3::splat(kind.scale()))
                .looking_to(-heading, Vec3::Y),
            Vision {
                heading,
                look_dir: heading,
                look_vel: 0.0,
                sweep_phase,
            },
            Awareness {
                mode: Mode::Patrol,
                interest: 0.0,
                last_seen: world,
                search_timer: 0.0,
                search_points: Vec::new(),
                search_step: 0,
                alert_timer: 0.0,
            },
            Navigation::default(),
            Post {
                home: world,
                heading,
                sweep_phase,
            },
            DespawnOnExit(InGame),
            Name::new(format!("Adversary {i} ({})", kind.label())),
        ));
        let guard = entity.id();

        // Kind-specific state: only patrolling guards carry a route, only
        // wandering guards carry an RNG. Static guards need neither.
        match kind {
            GuardKind::Patrolling => {
                // Off its own seeded stream, so the loop is stable across runs
                // and doesn't perturb the placement stream.
                let mut route_rng =
                    SmallRng::seed_from_u64(run_seed.derive_indexed("adversary.patrol", i));
                entity.insert(PatrolRoute {
                    waypoints: patrol_route(&map, &mut route_rng, tile),
                    index: 0,
                });
            }
            GuardKind::Wandering => {
                entity.insert(Wander(SmallRng::seed_from_u64(
                    run_seed.derive_indexed("adversary", i),
                )));
            }
            GuardKind::Static => {}
        }

        // `entity`'s borrow of `commands` has ended (last used above), so it's
        // free again to spawn the guard's overlay children.
        attach_overlays(&mut commands, guard, &overlays);
    }
}

/// Build a fixed patrol loop: the spawn tile plus a handful of random reachable
/// tiles within [`PATROL_RADIUS`] of it, ordered nearest-neighbour from the start
/// so the guard walks a coherent local beat rather than crisscrossing the map.
/// The connected map guarantees BFS links each leg at runtime.
fn patrol_route(map: &LevelMap, rng: &mut SmallRng, start: (usize, usize)) -> Vec<(usize, usize)> {
    let mut pool: Vec<(usize, usize)> = Vec::with_capacity(PATROL_WAYPOINTS - 1);
    while pool.len() < PATROL_WAYPOINTS - 1 {
        pool.push(near_tile(map, rng, start));
    }

    // Greedily chain the nearest unvisited tile from where we are, starting at
    // `start`, so consecutive legs stay short and the loop doesn't zigzag.
    let mut route = Vec::with_capacity(PATROL_WAYPOINTS);
    route.push(start);
    let mut cur = start;
    while !pool.is_empty() {
        let (i, &next) = pool
            .iter()
            .enumerate()
            .min_by_key(|&(_, &t)| chebyshev(cur, t))
            .expect("pool is non-empty");
        route.push(next);
        cur = next;
        pool.swap_remove(i);
    }
    route
}

/// Pick a random walkable tile within [`PATROL_RADIUS`] (Chebyshev) of `origin`.
/// The origin tile itself is walkable, so this always terminates.
fn near_tile(map: &LevelMap, rng: &mut SmallRng, origin: (usize, usize)) -> (usize, usize) {
    loop {
        let t = random_walkable(map, rng);
        if chebyshev(origin, t) <= PATROL_RADIUS {
            return t;
        }
    }
}

/// Chebyshev (king-move) tile distance.
fn chebyshev(a: (usize, usize), b: (usize, usize)) -> i32 {
    let dx = (a.0 as i32 - b.0 as i32).abs();
    let dy = (a.1 as i32 - b.1 as i32).abs();
    dx.max(dy)
}

/// A facing for a posted static guard: toward the doorway of the room it stands
/// in, else the nearest doorway anywhere, else the player's spawn — so it always
/// watches a meaningful approach rather than a random wall. Falls back to a fixed
/// heading only on a doorless, spawnless map.
fn interesting_facing(map: &LevelMap, tile: (usize, usize), spawn: (usize, usize)) -> Vec3 {
    let here = map.tile_to_world(tile.0, tile.1);
    let toward = |target: (usize, usize)| {
        let dir = horizontal(map.tile_to_world(target.0, target.1) - here);
        dir.try_normalize()
    };

    // The doorway of the room this guard stands in, if any.
    let own_door = map
        .rooms()
        .iter()
        .find(|r| r.doorway.is_some() && r.tiles.contains(&tile))
        .and_then(|r| r.doorway);
    // Otherwise the closest doorway on the whole map.
    let nearest_door = map
        .rooms()
        .iter()
        .filter_map(|r| r.doorway)
        .min_by_key(|&d| chebyshev(tile, d));

    own_door
        .or(nearest_door)
        .and_then(toward)
        .or_else(|| toward(spawn))
        .unwrap_or(Vec3::Z)
}

/// Pick a random walkable tile at least [`SPAWN_CLEARANCE`] tiles from the
/// player's spawn on at least one axis.
fn far_tile(map: &LevelMap, rng: &mut SmallRng, sx: i32, sy: i32) -> (usize, usize) {
    loop {
        let t = random_walkable(map, rng);
        let far = (t.0 as i32 - sx).abs() >= SPAWN_CLEARANCE
            || (t.1 as i32 - sy).abs() >= SPAWN_CLEARANCE;
        if far {
            return t;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::map::{Room, Tile, TileRect};

    /// An all-floor square map for exercising placement helpers.
    fn open_map(size: usize) -> LevelMap {
        let mut map = LevelMap::filled_with_walls(size, size);
        for y in 0..size {
            for x in 0..size {
                map.set(x, y, Tile::Floor);
            }
        }
        map
    }

    #[test]
    fn patrol_route_starts_at_spawn_and_stays_local() {
        let map = open_map(40);
        let start = (20, 20);
        let mut rng = SmallRng::seed_from_u64(7);
        let route = patrol_route(&map, &mut rng, start);

        assert_eq!(route.len(), PATROL_WAYPOINTS);
        assert_eq!(route[0], start, "route begins at the spawn tile");
        for &wp in &route[1..] {
            assert!(
                chebyshev(start, wp) <= PATROL_RADIUS,
                "waypoint {wp:?} is within the local beat radius of {start:?}",
            );
        }
    }

    #[test]
    fn static_guard_faces_its_room_doorway() {
        let mut map = open_map(11);
        // A room the guard stands in, with its doorway off to the +x side.
        let guard = (5, 5);
        let door = (8, 5);
        map.set_rooms(vec![Room {
            kind: RoomKind::Security,
            rect: TileRect {
                min_x: 3,
                min_y: 3,
                max_x: 7,
                max_y: 7,
            },
            tiles: vec![guard],
            doorway: Some(door),
        }]);

        let h = interesting_facing(&map, guard, (0, 0));
        assert!(h.x > 0.9, "faces toward the +x doorway: {h:?}");
        assert!(h.z.abs() < 0.1, "no sideways lean: {h:?}");
    }

    #[test]
    fn static_guard_without_doorways_faces_spawn() {
        let map = open_map(11); // no rooms recorded
        let guard = (5, 5);
        let spawn = (5, 9); // straight along +z from the guard
        let h = interesting_facing(&map, guard, spawn);
        assert!(h.z > 0.9, "faces toward the spawn: {h:?}");
        assert!(h.x.abs() < 0.1, "no sideways lean: {h:?}");
    }
}
