//! The per-tick guard brain: sense, decide, move. Runs on FixedUpdate so each
//! guard's evolution is a pure function of the seed and tick count, and resets
//! to its exact spawn state on a loop restart so every run replays identically.

use bevy::prelude::*;

use crate::level::LevelMap;
use crate::player::Player;
use crate::time_loop::{Ghost, LoopReset};

use super::path::{bfs_path, random_walkable, search_ring};
use super::vision::{
    VISION_HALF_ANGLE, VISION_RANGE, first_visible, horizontal, rotate_y, smooth_turn,
};
use super::{
    ALERT_DELAY, Adversary, Awareness, CHASE_SPEED, CONE_SMOOTH_TIME, DWELL_DURATION,
    INTEREST_DECAY, INTEREST_EDGE_FACTOR, INTEREST_GAIN, INTEREST_MAX, INTEREST_MIN_FACTOR,
    INTEREST_THRESHOLD, Mode, Navigation, PATROL_SPEED, PatrolRoute, Post, REPATH_INTERVAL,
    SCAN_CADENCE, SCAN_STEP_INTERVAL, SEARCH_DURATION, SEARCH_DWELL_DURATION,
    SEARCH_INTEREST_FLOOR, SEARCH_RING_RADIUS, SEARCH_SCAN_STEP_INTERVAL, SEARCH_SPEED,
    SEARCH_SWEEP_AMPLITUDE, SWEEP_AMPLITUDE, TURN_SPEED, Vision, WAYPOINT_RADIUS, Wander,
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
        vision.look_vel = 0.0;
        vision.sweep_phase = post.sweep_phase;
        awareness.mode = Mode::Patrol;
        awareness.interest = 0.0;
        awareness.last_seen = post.home;
        awareness.search_timer = 0.0;
        awareness.search_points.clear();
        awareness.search_step = 0;
        awareness.alert_timer = 0.0;
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

        // 1. Work out where the cone points this frame. A guard looks straight
        // along its heading while walking, and only sweeps a deliberate look
        // cadence while stopped to scan — searching sweeps a wider, quicker arc.
        // A banked sighting overrides both: the cone locks onto the target.
        let traveling = nav.index < nav.path.len();
        let (sweep_amp, scan_step) = if awareness.mode == Mode::Search {
            (SEARCH_SWEEP_AMPLITUDE, SEARCH_SCAN_STEP_INTERVAL)
        } else {
            (SWEEP_AMPLITUDE, SCAN_STEP_INTERVAL)
        };
        let target_dir = if awareness.mode == Mode::Chase {
            let to = horizontal(awareness.last_seen - pos);
            to.normalize_or(vision.look_dir)
        } else if awareness.interest > resting_interest(awareness.mode) {
            // Eyeing a target: lock the cone onto its last-seen spot while
            // suspicion climbs, rather than sweeping past and dropping it.
            let to = horizontal(awareness.last_seen - pos);
            to.normalize_or(vision.heading)
        } else if traveling {
            // On the move: look where you're going. The scan clock is held so
            // the next dwell starts centred on the heading.
            vision.heading
        } else {
            // Stopped at a waypoint: step the cone through the look cadence,
            // settling on each discrete glance with `smooth_turn` easing between.
            vision.sweep_phase += dt;
            let step = (vision.sweep_phase / scan_step) as usize;
            let offset = SCAN_CADENCE[step % SCAN_CADENCE.len()] * sweep_amp;
            rotate_y(vision.heading, offset)
        };
        // Ease the cone toward this frame's target facing, accelerating from rest
        // and decelerating into place, so every cadence glance, lock-on, and
        // lost-target turn reads like a head turning rather than a cone snapping.
        let current = vision.look_dir;
        let look_dir = smooth_turn(
            current,
            target_dir,
            &mut vision.look_vel,
            CONE_SMOOTH_TIME,
            dt,
        );
        vision.look_dir = look_dir;

        // 2. Scan for the highest-priority visible target inside the cone.
        let spotted = first_visible(&map, pos, look_dir, &target_positions);

        // 3. Update interest and drive the mode transitions.
        match awareness.mode {
            Mode::Patrol => {
                accrue_interest(&mut awareness, spotted, pos, look_dir, dt, 0.0);
                if awareness.interest >= INTEREST_THRESHOLD {
                    // Spotted — hold a brief "spotted you!" beat, then commit.
                    if alert_ready(&mut awareness, dt) {
                        enter_chase(&map, &mut awareness, &mut nav, here);
                    }
                } else {
                    // Sighting lapsed before the beat finished (or never tripped).
                    awareness.alert_timer = 0.0;
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
                accrue_interest(
                    &mut awareness,
                    spotted,
                    pos,
                    look_dir,
                    dt,
                    SEARCH_INTEREST_FLOOR,
                );
                if awareness.interest >= INTEREST_THRESHOLD {
                    if alert_ready(&mut awareness, dt) {
                        enter_chase(&map, &mut awareness, &mut nav, here);
                    }
                } else {
                    awareness.alert_timer = 0.0;
                    if awareness.search_timer <= 0.0 {
                        // Gave up the hunt — stand down to a normal patrol.
                        awareness.mode = Mode::Patrol;
                        awareness.interest = 0.0;
                        nav.path.clear();
                        nav.index = 0;
                    }
                }
            }
        }

        // 4. Move, or dwell-and-scan at a waypoint. A guard eyeing a target
        // (pinned: interest above its mode's baseline, not yet chasing) halts and
        // stares it down while the cone tracks it. Otherwise it advances along its
        // path looking ahead; on reaching the end it pauses to scan, and only once
        // its dwell elapses does it pick the next destination — so patrols read as
        // walk-stop-look-around rather than an endless shuffle.
        let pinned =
            awareness.mode != Mode::Chase && awareness.interest > resting_interest(awareness.mode);
        let speed = match awareness.mode {
            Mode::Chase => CHASE_SPEED,
            Mode::Search => SEARCH_SPEED,
            Mode::Patrol => PATROL_SPEED,
        };
        if !pinned && nav.index < nav.path.len() {
            // Travelling: keep the dwell re-armed so it's full the moment we stop.
            if awareness.mode != Mode::Chase {
                nav.dwell_timer = dwell_len(awareness.mode);
            }
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
        } else if !pinned && awareness.mode != Mode::Chase {
            // Arrived at a waypoint with nothing to chase — scan, then move on.
            nav.dwell_timer -= dt;
            if nav.dwell_timer <= 0.0 {
                if awareness.mode == Mode::Search {
                    next_search_goal(&map, here, &mut awareness, &mut nav);
                } else {
                    next_patrol_goal(&map, here, &mut nav, post, patrol, wander);
                }
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

/// Fold a fresh sighting (or its absence) into the interest meter: a visible
/// target updates `last_seen` and raises interest by [`interest_gain`]; no target
/// decays it toward `floor`.
fn accrue_interest(
    awareness: &mut Awareness,
    spotted: Option<Vec3>,
    pos: Vec3,
    look_dir: Vec3,
    dt: f32,
    floor: f32,
) {
    match spotted {
        Some(target) => {
            awareness.last_seen = target;
            awareness.interest =
                (awareness.interest + interest_gain(pos, target, look_dir) * dt).min(INTEREST_MAX);
        }
        None => {
            awareness.interest = (awareness.interest - INTEREST_DECAY * dt).max(floor);
        }
    }
}

/// Advance the pre-chase "spotted you!" beat. Arms the hold on the first call
/// after a sighting crosses the threshold, counts it down, and returns `true`
/// once it elapses and the chase should commit. Call only while at/over the
/// threshold; reset `alert_timer` to zero when the sighting falls back below it.
fn alert_ready(awareness: &mut Awareness, dt: f32) -> bool {
    if awareness.alert_timer <= 0.0 {
        awareness.alert_timer = ALERT_DELAY;
    }
    awareness.alert_timer -= dt;
    awareness.alert_timer <= 0.0
}

/// How long a guard pauses to scan at each waypoint in the given mode.
fn dwell_len(mode: Mode) -> f32 {
    match mode {
        Mode::Search => SEARCH_DWELL_DURATION,
        _ => DWELL_DURATION,
    }
}

/// Interest gained per second for a target at `target` seen from `pos` along
/// `look_dir`: full rate point-blank and dead-centre, tapering to
/// [`INTEREST_MIN_FACTOR`] at the cone's far edge (distance) and
/// [`INTEREST_EDGE_FACTOR`] at its side edge (angle), so both depth and
/// peripheral position make a target harder to notice.
fn interest_gain(pos: Vec3, target: Vec3, look_dir: Vec3) -> f32 {
    let to = horizontal(target - pos);
    let dist = to.length();
    let closeness = (1.0 - dist / VISION_RANGE).clamp(0.0, 1.0);
    let by_distance = INTEREST_MIN_FACTOR + (1.0 - INTEREST_MIN_FACTOR) * closeness;

    // How far off the cone centre the target sits: 0 dead-centre, 1 at the edge.
    let off = if dist > 1e-3 {
        let cos = (to / dist).dot(look_dir).clamp(-1.0, 1.0);
        (cos.acos() / VISION_HALF_ANGLE).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let by_angle = 1.0 - (1.0 - INTEREST_EDGE_FACTOR) * off;

    INTEREST_GAIN * by_distance * by_angle
}

/// Replace the followed path with a fresh BFS route from `here` to `goal`.
fn repath(map: &LevelMap, nav: &mut Navigation, here: (usize, usize), goal: (usize, usize)) {
    if let Some(path) = bfs_path(map, here, goal) {
        nav.path = path;
        nav.index = 0;
    }
}
