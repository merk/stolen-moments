//! The per-tick guard brain: sense, decide, move. Runs on FixedUpdate so each
//! guard's evolution is a pure function of the seed and tick count, and resets
//! to its exact spawn state on a loop restart so every run replays identically.

use bevy::prelude::*;

use crate::level::LevelMap;
use crate::player::Player;
use crate::time_loop::{Ghost, LoopReset};

use super::path::{bfs_path, random_walkable, search_ring};
use super::vision::{VISION_RANGE, first_visible, horizontal, rotate_y};
use super::{
    Adversary, Awareness, CHASE_SPEED, INTEREST_DECAY, INTEREST_GAIN, INTEREST_MAX,
    INTEREST_MIN_FACTOR, INTEREST_THRESHOLD, Mode, Navigation, PATROL_SPEED, PatrolRoute, Post,
    REPATH_INTERVAL, SEARCH_DURATION, SEARCH_INTEREST_FLOOR, SEARCH_RING_RADIUS, SEARCH_SPEED,
    SEARCH_SWEEP_AMPLITUDE, SEARCH_SWEEP_SPEED, SWEEP_AMPLITUDE, SWEEP_SPEED, TURN_SPEED, Vision,
    WAYPOINT_RADIUS, Wander,
};

/// On a loop restart, send every guard back to its post and restore its full
/// initial state (facing, sweep phase, interest) so the new run is identical.
pub(super) fn reset_adversaries(
    _reset: On<LoopReset>,
    mut adversaries: Query<(
        &mut Transform,
        &mut Vision,
        &mut Awareness,
        &mut Navigation,
        &Post,
        Option<&mut PatrolRoute>,
    )>,
) {
    for (mut transform, mut vision, mut awareness, mut nav, post, route) in &mut adversaries {
        transform.translation = post.home;
        transform.rotation = Transform::IDENTITY
            .looking_to(-post.heading, Vec3::Y)
            .rotation;
        vision.heading = post.heading;
        vision.look_dir = post.heading;
        vision.sweep_phase = post.sweep_phase;
        awareness.mode = Mode::Patrol;
        awareness.interest = 0.0;
        awareness.last_seen = post.home;
        awareness.search_timer = 0.0;
        awareness.search_points.clear();
        awareness.search_step = 0;
        *nav = Navigation::default();
        if let Some(mut route) = route {
            route.index = 0;
        }
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
    mut adversaries: Query<
        (
            &mut Transform,
            &mut Vision,
            &mut Awareness,
            &mut Navigation,
            &Post,
            Option<&mut PatrolRoute>,
            Option<&mut Wander>,
        ),
        With<Adversary>,
    >,
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

    for (mut transform, mut vision, mut awareness, mut nav, post, patrol, wander) in
        &mut adversaries
    {
        let pos = transform.translation;
        let Some(here) = map.world_to_tile(pos) else {
            continue;
        };

        // 1. Work out where the cone is pointing this frame. While searching the
        // sweep is wider and quicker, to cover more ground hunting a lost target.
        let (sweep_amp, sweep_speed) = if awareness.mode == Mode::Search {
            (SEARCH_SWEEP_AMPLITUDE, SEARCH_SWEEP_SPEED)
        } else {
            (SWEEP_AMPLITUDE, SWEEP_SPEED)
        };
        vision.sweep_phase += sweep_speed * dt;
        let look_dir = match awareness.mode {
            Mode::Chase => {
                let to = horizontal(awareness.last_seen - pos);
                to.normalize_or(vision.look_dir)
            }
            // Patrol or Search. The moment interest rises above the mode's
            // resting baseline the guard is actively eyeing a target, so lock
            // the cone onto its last-seen position to track it while suspicion
            // climbs — rather than sweeping past and dropping the sighting. The
            // sweep phase keeps advancing underneath, so once interest settles
            // back the cone resumes its deterministic side-to-side sweep.
            _ if awareness.interest > resting_interest(awareness.mode) => {
                let to = horizontal(awareness.last_seen - pos);
                let swing = vision.sweep_phase.sin() * sweep_amp;
                to.normalize_or(rotate_y(vision.heading, swing))
            }
            _ => {
                let swing = vision.sweep_phase.sin() * sweep_amp;
                rotate_y(vision.heading, swing)
            }
        };
        vision.look_dir = look_dir;

        // 2. Scan for the highest-priority visible target inside the cone.
        let spotted = first_visible(&map, pos, look_dir, &target_positions);

        // 3. Update interest, state, and this frame's destination.
        match awareness.mode {
            Mode::Patrol => {
                if let Some(target) = spotted {
                    awareness.last_seen = target;
                    awareness.interest =
                        (awareness.interest + interest_gain(pos, target) * dt).min(INTEREST_MAX);
                } else {
                    awareness.interest = (awareness.interest - INTEREST_DECAY * dt).max(0.0);
                }

                if spotted.is_some() && awareness.interest >= INTEREST_THRESHOLD {
                    enter_chase(&map, &mut awareness, &mut nav, here);
                } else if awareness.interest <= 0.0 && nav.index >= nav.path.len() {
                    // Idle (reached the current goal) and not eyeing anyone —
                    // pick the next destination per kind. While interest is
                    // banked the guard holds instead (see the movement step).
                    next_patrol_goal(&map, here, &mut nav, post, patrol, wander);
                }
            }
            Mode::Chase => {
                if let Some(target) = spotted {
                    awareness.last_seen = target;
                }
                let arrived = nav.index >= nav.path.len();
                if spotted.is_none() && arrived {
                    // Lost them and reached where they were — don't forget
                    // instantly; drop into an alarmed search around the spot.
                    enter_search(&map, &mut awareness, &mut nav, here);
                } else {
                    nav.repath_timer -= dt;
                    if nav.repath_timer <= 0.0 {
                        if let Some(goal) = map.world_to_tile(awareness.last_seen) {
                            repath(&map, &mut nav, here, goal);
                        }
                        nav.repath_timer = REPATH_INTERVAL;
                    }
                }
            }
            Mode::Search => {
                awareness.search_timer -= dt;
                if let Some(target) = spotted {
                    awareness.last_seen = target;
                    awareness.interest =
                        (awareness.interest + interest_gain(pos, target) * dt).min(INTEREST_MAX);
                } else {
                    // Hold the alarmed baseline rather than draining to zero, so
                    // a glimpse re-trips a chase almost immediately.
                    awareness.interest =
                        (awareness.interest - INTEREST_DECAY * dt).max(SEARCH_INTEREST_FLOOR);
                }

                if spotted.is_some() && awareness.interest >= INTEREST_THRESHOLD {
                    enter_chase(&map, &mut awareness, &mut nav, here);
                } else if awareness.search_timer <= 0.0 {
                    // Gave up the hunt — stand down to a normal patrol.
                    awareness.mode = Mode::Patrol;
                    awareness.interest = 0.0;
                    nav.path.clear();
                    nav.index = 0;
                } else if spotted.is_none() && nav.index >= nav.path.len() {
                    // Reached an investigate tile — move on to the next.
                    next_search_goal(&map, here, &mut awareness, &mut nav);
                }
            }
        }

        // 4. Advance along the current path — unless pinned. A guard actively
        // eyeing a target (interest above its mode's baseline, while not yet
        // committed to a chase) halts and stares it down while the cone tracks
        // it (step 1) and suspicion climbs — for any kind — rather than walking
        // its route or search back out of range and dropping the sighting. Once
        // interest settles it resumes movement from where it froze.
        let pinned =
            awareness.mode != Mode::Chase && awareness.interest > resting_interest(awareness.mode);
        let speed = match awareness.mode {
            Mode::Chase => CHASE_SPEED,
            Mode::Search => SEARCH_SPEED,
            Mode::Patrol => PATROL_SPEED,
        };
        if !pinned && nav.index < nav.path.len() {
            let (tx, ty) = nav.path[nav.index];
            let waypoint = map.tile_to_world(tx, ty);
            let to = horizontal(waypoint - pos);
            let dist = to.length();
            if dist <= WAYPOINT_RADIUS {
                nav.index += 1;
            } else {
                let dir = to / dist;
                let step = (speed * dt).min(dist);
                transform.translation = pos + dir * step;
                vision.heading = dir;
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

/// Interest a guard idles at in the given mode: zero while patrolling, the
/// alarmed floor while searching. Interest above this means it's actively eyeing
/// a target (used to pin the cone and halt movement); at or below it the guard
/// sweeps and moves normally.
fn resting_interest(mode: Mode) -> f32 {
    match mode {
        Mode::Search => SEARCH_INTEREST_FLOOR,
        _ => 0.0,
    }
}

/// Begin chasing toward the current `last_seen` position.
fn enter_chase(
    map: &LevelMap,
    awareness: &mut Awareness,
    nav: &mut Navigation,
    here: (usize, usize),
) {
    awareness.mode = Mode::Chase;
    nav.repath_timer = REPATH_INTERVAL;
    if let Some(goal) = map.world_to_tile(awareness.last_seen) {
        repath(map, nav, here, goal);
    }
}

/// Drop into an alarmed search centred on `last_seen`: hold a baseline of
/// interest, arm the give-up timer, build the deterministic ring of tiles to
/// investigate, and head for the first one.
fn enter_search(
    map: &LevelMap,
    awareness: &mut Awareness,
    nav: &mut Navigation,
    here: (usize, usize),
) {
    awareness.mode = Mode::Search;
    awareness.interest = SEARCH_INTEREST_FLOOR;
    awareness.search_timer = SEARCH_DURATION;
    awareness.search_step = 0;
    awareness.search_points = match map.world_to_tile(awareness.last_seen) {
        Some(origin) => search_ring(map, origin, SEARCH_RING_RADIUS),
        None => Vec::new(),
    };
    next_search_goal(map, here, awareness, nav);
}

/// Path to the next tile in the search ring, advancing the step counter. Wraps
/// around the ring so a guard keeps investigating until its timer runs out.
fn next_search_goal(
    map: &LevelMap,
    here: (usize, usize),
    awareness: &mut Awareness,
    nav: &mut Navigation,
) {
    if awareness.search_points.is_empty() {
        return;
    }
    let goal = awareness.search_points[awareness.search_step % awareness.search_points.len()];
    awareness.search_step += 1;
    repath(map, nav, here, goal);
}

/// Choose the next destination for an idle guard, dispatched by which behaviour
/// component it carries: a patrolling guard advances along its route, a wandering
/// guard picks a fresh random tile, and a static guard (neither) returns to post.
fn next_patrol_goal(
    map: &LevelMap,
    here: (usize, usize),
    nav: &mut Navigation,
    post: &Post,
    patrol: Option<Mut<PatrolRoute>>,
    wander: Option<Mut<Wander>>,
) {
    if let Some(mut route) = patrol {
        if route.waypoints.is_empty() {
            return;
        }
        route.index = (route.index + 1) % route.waypoints.len();
        let goal = route.waypoints[route.index];
        repath(map, nav, here, goal);
    } else if let Some(mut wander) = wander {
        let dest = random_walkable(map, &mut wander.0);
        repath(map, nav, here, dest);
    } else if let Some(post_tile) = map.world_to_tile(post.home)
        && here != post_tile
    {
        // Static guard displaced (e.g. after a chase) — walk back to the post.
        repath(map, nav, here, post_tile);
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
fn repath(map: &LevelMap, nav: &mut Navigation, here: (usize, usize), goal: (usize, usize)) {
    if let Some(path) = bfs_path(map, here, goal) {
        nav.path = path;
        nav.index = 0;
    }
}
