//! Roaming adversaries that hunt the player and any replaying ghosts.
//!
//! Each adversary sweeps a vision cone left-and-right as it wanders random
//! routes around the cavern. The moment a target (the live player or a ghost
//! from any loop) falls inside the cone — within range and with clear line of
//! sight — it locks on and chases via grid pathfinding until it reaches that
//! target's last-known position, then resumes patrolling.

use std::collections::VecDeque;

use bevy::prelude::*;
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};

use crate::dungeon::{DungeonMap, SpawnPoint};
use crate::player::Player;
use crate::time_loop::{Ghost, LoopReset};

/// How many adversaries to scatter through the dungeon.
const ADVERSARY_COUNT: usize = 2;

/// Don't spawn an adversary within this tile distance of the player's start.
const SPAWN_CLEARANCE: i32 = 8;

/// Wander speed (world units/sec) while patrolling.
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

/// Distance at which a path waypoint counts as reached.
const WAYPOINT_RADIUS: f32 = 0.15;
/// Seconds between chase re-paths to the target's current tile.
const REPATH_INTERVAL: f32 = 0.3;

/// Step size used when marching the line-of-sight ray across the grid.
const LOS_STEP: f32 = 0.25;

/// Height above the floor at which the cone gizmo is drawn (avoids z-fighting).
const CONE_LIFT: f32 = 0.08;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Wandering between random reachable tiles, cone sweeping side to side.
    Patrol,
    /// Locked onto a target, pathing toward its last-seen position.
    Chase,
}

#[derive(Component)]
pub struct Adversary {
    mode: Mode,
    /// Spawn position, returned to whenever a time loop restarts.
    home: Vec3,
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
    rng: SmallRng,
}

pub struct AdversaryPlugin;

impl Plugin for AdversaryPlugin {
    fn build(&self, app: &mut App) {
        // PostStartup so the dungeon map and spawn point already exist.
        app.add_systems(PostStartup, spawn_adversaries)
            .add_systems(Update, (update_adversaries, draw_vision_cones).chain())
            .add_observer(reset_adversaries);
    }
}

fn spawn_adversaries(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    map: Res<DungeonMap>,
    spawn: Res<SpawnPoint>,
) {
    let scene = asset_server
        .load(GltfAssetLabel::Scene(0).from_asset("Models/GLB format/character-orc.glb"));

    let mut rng = SmallRng::from_entropy();
    let (sx, sy) = (spawn.tile.0 as i32, spawn.tile.1 as i32);

    for i in 0..ADVERSARY_COUNT {
        // Pick a floor tile a comfortable distance from the player's spawn.
        let tile = loop {
            let t = random_walkable(&map, &mut rng);
            let far = (t.0 as i32 - sx).abs() >= SPAWN_CLEARANCE
                || (t.1 as i32 - sy).abs() >= SPAWN_CLEARANCE;
            if far {
                break t;
            }
        };

        let world = map.tile_to_world(tile.0, tile.1);
        let angle = rng.gen_range(0.0..std::f32::consts::TAU);
        let heading = Vec3::new(angle.cos(), 0.0, angle.sin());

        commands.spawn((
            SceneRoot(scene.clone()),
            Transform::from_translation(world).looking_to(-heading, Vec3::Y),
            Adversary {
                mode: Mode::Patrol,
                home: world,
                heading,
                look_dir: heading,
                sweep_phase: rng.gen_range(0.0..std::f32::consts::TAU),
                last_seen: world,
                repath_timer: 0.0,
                path: Vec::new(),
                path_index: 0,
                // Per-adversary RNG so each wanders independently.
                rng: SmallRng::from_entropy(),
            },
            Name::new(format!("Adversary {i}")),
        ));
    }
}

/// On a loop restart, send every adversary back to its spawn and reset it to a
/// clean patrol so the new run starts from the same configuration each time.
fn reset_adversaries(
    _reset: On<LoopReset>,
    mut adversaries: Query<(&mut Transform, &mut Adversary)>,
) {
    for (mut transform, mut adv) in &mut adversaries {
        transform.translation = adv.home;
        adv.mode = Mode::Patrol;
        adv.last_seen = adv.home;
        adv.repath_timer = 0.0;
        adv.path.clear();
        adv.path_index = 0;
    }
}

/// Sense, decide, and move every adversary for this frame.
fn update_adversaries(
    time: Res<Time>,
    map: Res<DungeonMap>,
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

        // 3. Update state and choose this frame's destination.
        match adv.mode {
            Mode::Patrol => {
                if let Some(target) = spotted {
                    enter_chase(&map, &mut adv, here, target);
                } else if adv.path_index >= adv.path.len() {
                    // Reached the wander goal — pick a fresh one.
                    let dest = random_walkable(&map, &mut adv.rng);
                    repath(&map, &mut adv, here, dest);
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

/// Begin chasing `target`: lock the last-seen position and path straight to it.
fn enter_chase(map: &DungeonMap, adv: &mut Adversary, here: (usize, usize), target: Vec3) {
    adv.mode = Mode::Chase;
    adv.last_seen = target;
    adv.repath_timer = REPATH_INTERVAL;
    if let Some(goal) = map.world_to_tile(target) {
        repath(map, adv, here, goal);
    }
}

/// Replace the followed path with a fresh BFS route from `here` to `goal`.
fn repath(map: &DungeonMap, adv: &mut Adversary, here: (usize, usize), goal: (usize, usize)) {
    if let Some(path) = bfs_path(map, here, goal) {
        adv.path = path;
        adv.path_index = 0;
    }
}

/// Return the first target (in caller-supplied priority order) that sits inside
/// the cone with clear line of sight.
fn first_visible(map: &DungeonMap, pos: Vec3, look_dir: Vec3, targets: &[Vec3]) -> Option<Vec3> {
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
fn clear_line_of_sight(map: &DungeonMap, from: Vec3, to: Vec3) -> bool {
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
    map: &DungeonMap,
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
fn random_walkable(map: &DungeonMap, rng: &mut SmallRng) -> (usize, usize) {
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
/// arc. Yellow while patrolling, red the moment it's locked onto a target.
fn draw_vision_cones(adversaries: Query<(&Transform, &Adversary)>, mut gizmos: Gizmos) {
    const ARC_SEGMENTS: usize = 12;

    for (transform, adv) in &adversaries {
        let origin = transform.translation + Vec3::Y * CONE_LIFT;
        let color = match adv.mode {
            Mode::Patrol => Color::srgb(1.0, 0.85, 0.2),
            Mode::Chase => Color::srgb(1.0, 0.2, 0.15),
        };

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
