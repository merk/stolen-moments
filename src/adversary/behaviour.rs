//! The per-tick guard brain: sense, decide, move. Runs on FixedUpdate so each
//! guard's evolution is a pure function of the seed and tick count, and resets
//! to its exact spawn state on a loop restart so every run replays identically.

use bevy::prelude::*;

use crate::level::LevelMap;
use crate::player::Player;
use crate::time_loop::{Ghost, LoopReset};

use super::path::{bfs_path, random_walkable};
use super::vision::{VISION_RANGE, first_visible, horizontal, rotate_y};
use super::{
    Adversary, CHASE_SPEED, GuardKind, INTEREST_DECAY, INTEREST_GAIN, INTEREST_MAX,
    INTEREST_MIN_FACTOR, INTEREST_THRESHOLD, Mode, PATROL_SPEED, REPATH_INTERVAL, SWEEP_AMPLITUDE,
    SWEEP_SPEED, TURN_SPEED, WAYPOINT_RADIUS,
};

/// On a loop restart, send every guard back to its post and restore its full
/// initial state (facing, sweep phase, interest) so the new run is identical.
pub(super) fn reset_adversaries(
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
pub(super) fn update_adversaries(
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
