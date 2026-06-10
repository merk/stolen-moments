//! Guard placement: drop one of each [`GuardKind`] into the level, posting
//! static guards in the Security room and scattering the roaming kinds a safe
//! distance from the player's spawn. All choices draw from seeded RNG streams so
//! the roster is identical across runs of the same seed.

use bevy::prelude::*;
use rand::rngs::SmallRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};

use crate::level::{LevelMap, RoomKind, SpawnPoint};
use crate::loading::LoadingAssets;
use crate::seed::RunSeed;
use crate::state::InGame;

use super::path::random_walkable;
use super::{Adversary, GUARD_KINDS, GuardKind, Mode, PATROL_WAYPOINTS, SPAWN_CLEARANCE};

pub(super) fn spawn_adversaries(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut loading: ResMut<LoadingAssets>,
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

        // Patrolling guards get a fixed route off their own seeded stream, so the
        // loop is stable across runs and doesn't perturb the wander stream.
        let patrol = if kind == GuardKind::Patrolling {
            let mut route_rng =
                SmallRng::seed_from_u64(run_seed.derive_indexed("adversary.patrol", i));
            patrol_route(&map, &mut route_rng, tile)
        } else {
            Vec::new()
        };

        let world = map.tile_to_world(tile.0, tile.1);
        let angle = rng.gen_range(0.0..std::f32::consts::TAU);
        let heading = Vec3::new(angle.cos(), 0.0, angle.sin());
        let sweep_phase = rng.gen_range(0.0..std::f32::consts::TAU);

        commands.spawn((
            SceneRoot(scene.clone()),
            Transform::from_translation(world)
                .with_scale(Vec3::splat(kind.scale()))
                .looking_to(-heading, Vec3::Y),
            Adversary {
                kind,
                mode: Mode::Patrol,
                interest: 0.0,
                home: world,
                spawn_heading: heading,
                spawn_sweep_phase: sweep_phase,
                heading,
                look_dir: heading,
                sweep_phase,
                last_seen: world,
                repath_timer: 0.0,
                path: Vec::new(),
                path_index: 0,
                patrol,
                patrol_index: 0,
                // Per-guard RNG so wandering routes are independent yet reproducible.
                rng: SmallRng::seed_from_u64(run_seed.derive_indexed("adversary", i)),
            },
            DespawnOnExit(InGame),
            Name::new(format!("Adversary {i} ({})", kind.label())),
        ));
    }
}

/// Build a fixed patrol loop: the spawn tile followed by a handful of random
/// reachable tiles. The connected map guarantees BFS links each leg at runtime.
fn patrol_route(map: &LevelMap, rng: &mut SmallRng, start: (usize, usize)) -> Vec<(usize, usize)> {
    let mut route = Vec::with_capacity(PATROL_WAYPOINTS);
    route.push(start);
    for _ in 1..PATROL_WAYPOINTS {
        route.push(random_walkable(map, rng));
    }
    route
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
