//! Roaming adversaries that hunt the player and any replaying ghosts.
//!
//! Guards come in three [`GuardKind`]s that differ in how they move while
//! unaware — a **static** guard holds a fixed post and only sweeps its cone, a
//! **patrolling** guard walks a fixed seeded route, and a **wandering** guard
//! roams to random tiles. All three share one sensing model: a vision cone
//! sweeps side to side, and a target lingering inside it (within range, with
//! clear line of sight) fills a per-guard **interest** meter. Crossing the
//! interest threshold trips the guard into a chase toward the target's
//! last-known position; brief or distant exposure decays harmlessly. Everything
//! is a pure function of the seed and the FixedUpdate tick count, so a guard's
//! routes, sweeps, and interest build-up replay identically on every loop.

use std::collections::VecDeque;

use bevy::prelude::*;
use rand::rngs::SmallRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};

use crate::level::{LevelMap, RoomKind, SpawnPoint};
use crate::loading::LoadingAssets;
use crate::player::Player;
use crate::seed::RunSeed;
use crate::state::{GameState, InGame, WorldGen};
use crate::time_loop::{Ghost, LoopReset};

/// The roster of guards to spawn, by kind. One of each for now; the order also
/// indexes each guard's seeded RNG stream.
const GUARD_KINDS: [GuardKind; 3] = [
    GuardKind::Static,
    GuardKind::Patrolling,
    GuardKind::Wandering,
];

/// Don't spawn a non-posted guard within this tile distance of the spawn.
const SPAWN_CLEARANCE: i32 = 8;

/// How many waypoints a patrolling guard's fixed route cycles through.
const PATROL_WAYPOINTS: usize = 4;

/// Wander/patrol speed (world units/sec) while unaware.
const PATROL_SPEED: f32 = 2.6;
/// Pursuit speed once a target has been spotted — faster than patrol so a
/// fleeing player can't simply outrun a lock-on at equal speed forever.
const CHASE_SPEED: f32 = 4.2;
/// How quickly the body slerps to face the vision cone's direction.
const TURN_SPEED: f32 = 8.0;

/// How far the vision cone reaches (world units).
const VISION_RANGE: f32 = 9.0;
/// Half-angle of the cone (radians). ~34° each side of centre.
const VISION_HALF_ANGLE: f32 = 0.6;

/// Peak swing of the cone away from the heading while patrolling (radians).
const SWEEP_AMPLITUDE: f32 = 0.9;
/// Angular speed of the sweep oscillation (radians/sec of phase).
const SWEEP_SPEED: f32 = 1.6;

/// Interest needed to trip a guard from patrol into a chase.
const INTEREST_THRESHOLD: f32 = 1.0;
/// Interest gained per second with a target dead-centre and point-blank; scaled
/// down with distance by [`INTEREST_MIN_FACTOR`] so far sightings build slowly.
const INTEREST_GAIN: f32 = 1.6;
/// Floor on the distance-scaled gain — a target at the cone's far edge still
/// raises interest at this fraction of [`INTEREST_GAIN`].
const INTEREST_MIN_FACTOR: f32 = 0.4;
/// Interest lost per second while no target is visible.
const INTEREST_DECAY: f32 = 0.7;
/// Cap so interest can't bank arbitrarily high before a chase begins.
const INTEREST_MAX: f32 = 1.25;

/// Distance at which a path waypoint counts as reached.
const WAYPOINT_RADIUS: f32 = 0.15;
/// Seconds between chase re-paths to the target's current tile.
const REPATH_INTERVAL: f32 = 0.3;

/// Step size used when marching the line-of-sight ray across the grid.
const LOS_STEP: f32 = 0.25;

/// Height above the floor at which the cone gizmo is drawn (avoids z-fighting).
const CONE_LIFT: f32 = 0.08;

/// How a guard behaves while it hasn't yet locked onto a target.
#[derive(Clone, Copy, PartialEq, Eq)]
enum GuardKind {
    /// Holds a fixed post, sweeping its cone; only moves once roused to chase,
    /// returning to the post afterwards.
    Static,
    /// Walks a fixed seeded loop of waypoints, sweeping as it goes.
    Patrolling,
    /// Roams to random reachable tiles (the original behaviour).
    Wandering,
}

impl GuardKind {
    /// Uniform scale for the guard model — a light visual tell, since all kinds
    /// share the one orc mesh.
    fn scale(self) -> f32 {
        match self {
            GuardKind::Static => 1.15,
            GuardKind::Patrolling => 1.0,
            GuardKind::Wandering => 0.9,
        }
    }

    fn label(self) -> &'static str {
        match self {
            GuardKind::Static => "static",
            GuardKind::Patrolling => "patrolling",
            GuardKind::Wandering => "wandering",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Unaware: moving per the guard's kind, cone sweeping side to side.
    Patrol,
    /// Locked onto a target, pathing toward its last-seen position.
    Chase,
}

#[derive(Component)]
pub struct Adversary {
    kind: GuardKind,
    mode: Mode,
    /// Suspicion accumulated from time-in-cone; a chase begins once it crosses
    /// [`INTEREST_THRESHOLD`].
    interest: f32,
    /// Spawn position, returned to whenever a time loop restarts.
    home: Vec3,
    /// Initial facing, restored on loop reset so each run sweeps identically.
    spawn_heading: Vec3,
    /// Initial sweep phase, restored on loop reset for the same reason.
    spawn_sweep_phase: f32,
    /// Base facing (normalised, horizontal). While patrolling the cone sweeps
    /// around this; it tracks the current movement direction.
    heading: Vec3,
    /// The actual cone-centre direction this frame (heading + sweep, or the
    /// bearing to the target while chasing). Cached for the gizmo pass.
    look_dir: Vec3,
    /// Phase of the patrol sweep oscillation.
    sweep_phase: f32,
    /// Where the target was last seen; the chase destination.
    last_seen: Vec3,
    /// Counts down to the next chase re-path.
    repath_timer: f32,
    /// Tile waypoints currently being followed.
    path: Vec<(usize, usize)>,
    path_index: usize,
    /// Fixed patrol loop (patrolling guards only); cycled by `patrol_index`.
    patrol: Vec<(usize, usize)>,
    patrol_index: usize,
    rng: SmallRng,
}

pub struct AdversaryPlugin;

impl Plugin for AdversaryPlugin {
    fn build(&self, app: &mut App) {
        // PostStartup so the dungeon map and spawn point already exist.
        // Sensing/decision/movement runs on FixedUpdate so behaviour is a pure
        // function of the seed and tick count, independent of frame rate — the
        // basis for guard routes repeating identically across loops. The cone
        // gizmo draws every frame in Update off the cached `look_dir`.
        app.add_systems(
            OnEnter(GameState::Loading),
            spawn_adversaries.in_set(WorldGen::Populate),
        )
        .add_systems(
            FixedUpdate,
            update_adversaries.run_if(in_state(GameState::Playing)),
        )
        .add_systems(
            Update,
            draw_vision_cones.run_if(in_state(GameState::Playing)),
        )
        .add_observer(reset_adversaries);
    }
}

fn spawn_adversaries(
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

/// On a loop restart, send every guard back to its post and restore its full
/// initial state (facing, sweep phase, interest) so the new run is identical.
fn reset_adversaries(
    _reset: On<LoopReset>,
    mut adversaries: Query<(&mut Transform, &mut Adversary)>,
) {
    for (mut transform, mut adv) in &mut adversaries {
        transform.translation = adv.home;
        transform.rotation = Transform::IDENTITY
            .looking_to(-adv.spawn_heading, Vec3::Y)
            .rotation;
        adv.mode = Mode::Patrol;
        adv.interest = 0.0;
        adv.heading = adv.spawn_heading;
        adv.look_dir = adv.spawn_heading;
        adv.sweep_phase = adv.spawn_sweep_phase;
        adv.last_seen = adv.home;
        adv.repath_timer = 0.0;
        adv.path.clear();
        adv.path_index = 0;
        adv.patrol_index = 0;
    }
}

/// Sense, decide, and move every adversary for this frame.
fn update_adversaries(
    time: Res<Time>,
    map: Res<LevelMap>,
    // The live player and every ghost are all valid prey. `Without<Adversary>`
    // keeps these read-only Transform queries disjoint from the mutable one below.
    player: Query<&Transform, (With<Player>, Without<Adversary>)>,
    ghosts: Query<(&Transform, &Ghost), Without<Adversary>>,
    mut adversaries: Query<(&mut Transform, &mut Adversary)>,
) {
    let dt = time.delta_secs();

    // Build the prey list in priority order: the live player first, then ghosts
    // from newest loop to oldest. Adversaries lock onto the first one they can
    // see in this order, regardless of which happens to be closer.
    let mut target_positions: Vec<Vec3> = Vec::new();
    if let Ok(p) = player.single() {
        target_positions.push(p.translation);
    }
    let mut ranked: Vec<(usize, Vec3)> = ghosts
        .iter()
        .map(|(t, g)| (g.loop_index(), t.translation))
        .collect();
    ranked.sort_by_key(|&(idx, _)| std::cmp::Reverse(idx));
    target_positions.extend(ranked.into_iter().map(|(_, pos)| pos));

    for (mut transform, mut adv) in &mut adversaries {
        let pos = transform.translation;
        let Some(here) = map.world_to_tile(pos) else {
            continue;
        };

        // 1. Work out where the cone is pointing this frame.
        adv.sweep_phase += SWEEP_SPEED * dt;
        let look_dir = match adv.mode {
            Mode::Patrol => {
                let swing = adv.sweep_phase.sin() * SWEEP_AMPLITUDE;
                rotate_y(adv.heading, swing)
            }
            Mode::Chase => {
                let to = horizontal(adv.last_seen - pos);
                to.normalize_or(adv.look_dir)
            }
        };
        adv.look_dir = look_dir;

        // 2. Scan for the highest-priority visible target inside the cone.
        let spotted = first_visible(&map, pos, look_dir, &target_positions);

        // 3. Update interest, state, and this frame's destination.
        match adv.mode {
            Mode::Patrol => {
                if let Some(target) = spotted {
                    adv.last_seen = target;
                    adv.interest =
                        (adv.interest + interest_gain(pos, target) * dt).min(INTEREST_MAX);
                } else {
                    adv.interest = (adv.interest - INTEREST_DECAY * dt).max(0.0);
                }

                if spotted.is_some() && adv.interest >= INTEREST_THRESHOLD {
                    enter_chase(&map, &mut adv, here);
                } else if adv.path_index >= adv.path.len() {
                    // Idle (reached the current goal) — pick the next per kind.
                    next_patrol_goal(&map, &mut adv, here);
                }
            }
            Mode::Chase => {
                if let Some(target) = spotted {
                    adv.last_seen = target;
                }
                let arrived = adv.path_index >= adv.path.len();
                if spotted.is_none() && arrived {
                    // Lost them and reached where they were — back to patrol.
                    adv.mode = Mode::Patrol;
                    adv.interest = 0.0;
                    adv.path.clear();
                    adv.path_index = 0;
                } else {
                    adv.repath_timer -= dt;
                    if adv.repath_timer <= 0.0 {
                        if let Some(goal) = map.world_to_tile(adv.last_seen) {
                            repath(&map, &mut adv, here, goal);
                        }
                        adv.repath_timer = REPATH_INTERVAL;
                    }
                }
            }
        }

        // 4. Advance along the current path.
        let speed = if adv.mode == Mode::Chase {
            CHASE_SPEED
        } else {
            PATROL_SPEED
        };
        if adv.path_index < adv.path.len() {
            let (tx, ty) = adv.path[adv.path_index];
            let waypoint = map.tile_to_world(tx, ty);
            let to = horizontal(waypoint - pos);
            let dist = to.length();
            if dist <= WAYPOINT_RADIUS {
                adv.path_index += 1;
            } else {
                let dir = to / dist;
                let step = (speed * dt).min(dist);
                transform.translation = pos + dir * step;
                adv.heading = dir;
            }
        }

        // 5. Turn the body to face the cone direction.
        let target_rot = Transform::from_translation(transform.translation)
            .looking_to(-look_dir, Vec3::Y)
            .rotation;
        transform.rotation = transform
            .rotation
            .slerp(target_rot, (TURN_SPEED * dt).min(1.0));
    }
}

/// Begin chasing toward the current `last_seen` position.
fn enter_chase(map: &LevelMap, adv: &mut Adversary, here: (usize, usize)) {
    adv.mode = Mode::Chase;
    adv.repath_timer = REPATH_INTERVAL;
    if let Some(goal) = map.world_to_tile(adv.last_seen) {
        repath(map, adv, here, goal);
    }
}

/// Choose the next patrol destination for an idle guard, per its kind: a static
/// guard heads back to its post, a patrolling guard advances along its loop, and
/// a wandering guard picks a fresh random tile.
fn next_patrol_goal(map: &LevelMap, adv: &mut Adversary, here: (usize, usize)) {
    match adv.kind {
        GuardKind::Static => {
            // Return to the post if displaced (e.g. after a chase); else idle.
            if let Some(post) = map.world_to_tile(adv.home)
                && here != post
            {
                repath(map, adv, here, post);
            }
        }
        GuardKind::Patrolling => {
            if adv.patrol.is_empty() {
                return;
            }
            adv.patrol_index = (adv.patrol_index + 1) % adv.patrol.len();
            let goal = adv.patrol[adv.patrol_index];
            repath(map, adv, here, goal);
        }
        GuardKind::Wandering => {
            let dest = random_walkable(map, &mut adv.rng);
            repath(map, adv, here, dest);
        }
    }
}

/// Interest gained per second for a target at `target` seen from `pos`: full
/// rate point-blank, tapering to [`INTEREST_MIN_FACTOR`] at the cone's far edge.
fn interest_gain(pos: Vec3, target: Vec3) -> f32 {
    let dist = horizontal(target - pos).length();
    let closeness = (1.0 - dist / VISION_RANGE).clamp(0.0, 1.0);
    INTEREST_GAIN * (INTEREST_MIN_FACTOR + (1.0 - INTEREST_MIN_FACTOR) * closeness)
}

/// Replace the followed path with a fresh BFS route from `here` to `goal`.
fn repath(map: &LevelMap, adv: &mut Adversary, here: (usize, usize), goal: (usize, usize)) {
    if let Some(path) = bfs_path(map, here, goal) {
        adv.path = path;
        adv.path_index = 0;
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

/// Return the first target (in caller-supplied priority order) that sits inside
/// the cone with clear line of sight.
fn first_visible(map: &LevelMap, pos: Vec3, look_dir: Vec3, targets: &[Vec3]) -> Option<Vec3> {
    let min_cos = VISION_HALF_ANGLE.cos();

    for &target in targets {
        let to = horizontal(target - pos);
        let dist = to.length();
        if dist > VISION_RANGE {
            continue;
        }
        // A target right on top of us is trivially "seen".
        if dist > 1e-3 {
            let dir = to / dist;
            if dir.dot(look_dir) < min_cos {
                continue;
            }
            if !clear_line_of_sight(map, pos, target) {
                continue;
            }
        }
        return Some(target);
    }

    None
}

/// March across the grid between two world points; blocked by any wall tile.
fn clear_line_of_sight(map: &LevelMap, from: Vec3, to: Vec3) -> bool {
    let delta = horizontal(to - from);
    let dist = delta.length();
    if dist < 1e-3 {
        return true;
    }
    let steps = (dist / LOS_STEP).ceil() as i32;
    for i in 1..=steps {
        let p = from + delta * (i as f32 / steps as f32);
        if !map.is_world_walkable(p) {
            return false;
        }
    }
    true
}

/// Breadth-first shortest path over walkable tiles, returning the waypoints
/// after `start` up to and including `goal`. Empty when already at the goal.
fn bfs_path(
    map: &LevelMap,
    start: (usize, usize),
    goal: (usize, usize),
) -> Option<Vec<(usize, usize)>> {
    if start == goal {
        return Some(Vec::new());
    }
    let (w, h) = (map.width, map.height);
    let mut came: Vec<Option<(usize, usize)>> = vec![None; w * h];
    let mut visited = vec![false; w * h];
    let mut queue = VecDeque::new();

    visited[start.1 * w + start.0] = true;
    queue.push_back(start);

    while let Some((cx, cy)) = queue.pop_front() {
        if (cx, cy) == goal {
            let mut path = Vec::new();
            let mut cur = goal;
            while cur != start {
                path.push(cur);
                cur = came[cur.1 * w + cur.0].expect("reconstruct reaches start");
            }
            path.reverse();
            return Some(path);
        }
        for (nx, ny) in neighbours(cx, cy, w, h) {
            let idx = ny * w + nx;
            if !visited[idx] && map.is_walkable(nx, ny) {
                visited[idx] = true;
                came[idx] = Some((cx, cy));
                queue.push_back((nx, ny));
            }
        }
    }
    None
}

/// Pick a uniformly random walkable tile. The connected map guarantees one
/// exists, so this always terminates.
fn random_walkable(map: &LevelMap, rng: &mut SmallRng) -> (usize, usize) {
    loop {
        let x = rng.gen_range(0..map.width);
        let y = rng.gen_range(0..map.height);
        if map.is_walkable(x, y) {
            return (x, y);
        }
    }
}

fn neighbours(x: usize, y: usize, w: usize, h: usize) -> impl Iterator<Item = (usize, usize)> {
    let mut out = Vec::with_capacity(4);
    if x > 0 {
        out.push((x - 1, y));
    }
    if x + 1 < w {
        out.push((x + 1, y));
    }
    if y > 0 {
        out.push((x, y - 1));
    }
    if y + 1 < h {
        out.push((x, y + 1));
    }
    out.into_iter()
}

/// Drop the Y component, keeping a flat XZ-plane vector.
fn horizontal(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

/// Rotate a horizontal vector about the Y axis by `angle` radians.
fn rotate_y(v: Vec3, angle: f32) -> Vec3 {
    let (s, c) = angle.sin_cos();
    Vec3::new(v.x * c + v.z * s, 0.0, -v.x * s + v.z * c)
}

/// Draw each adversary's vision cone on the floor: two edge rays plus the far
/// arc. Yellow at rest, warming through orange as interest builds, red once it's
/// locked onto a target.
fn draw_vision_cones(
    debug: Option<Res<crate::debug::DebugSettings>>,
    adversaries: Query<(&Transform, &Adversary)>,
    mut gizmos: Gizmos,
) {
    if debug.is_some_and(|d| !d.vision_cones) {
        return;
    }
    const ARC_SEGMENTS: usize = 12;

    for (transform, adv) in &adversaries {
        let origin = transform.translation + Vec3::Y * CONE_LIFT;
        let color = cone_color(adv);

        let mut prev: Option<Vec3> = None;
        for i in 0..=ARC_SEGMENTS {
            let t = i as f32 / ARC_SEGMENTS as f32;
            let angle = -VISION_HALF_ANGLE + t * (2.0 * VISION_HALF_ANGLE);
            let point = origin + rotate_y(adv.look_dir, angle) * VISION_RANGE;

            // The first and last spokes are the cone's straight edges.
            if i == 0 || i == ARC_SEGMENTS {
                gizmos.line(origin, point, color);
            }
            if let Some(previous) = prev {
                gizmos.line(previous, point, color);
            }
            prev = Some(point);
        }
    }
}

/// Cone tint: red while chasing, otherwise yellow→orange as interest builds.
fn cone_color(adv: &Adversary) -> Color {
    match adv.mode {
        Mode::Chase => Color::srgb(1.0, 0.2, 0.15),
        Mode::Patrol => {
            let t = (adv.interest / INTEREST_THRESHOLD).clamp(0.0, 1.0);
            Color::srgb(1.0, 0.85 - 0.45 * t, 0.2 - 0.05 * t)
        }
    }
}
