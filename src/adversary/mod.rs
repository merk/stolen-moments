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
//!
//! This file is the feature's header: the [`Adversary`] component, its tuning
//! constants, and the plugin wiring. The implementation lives in submodules —
//! [`spawn`] (placement), [`behaviour`] (the per-tick brain), [`cone`] (the
//! vision gizmo), and the pure helpers [`path`] (grid routing) and [`vision`]
//! (sensing). Those submodules read [`Adversary`]'s private fields directly, as
//! descendants of this module; the rest of the crate sees only an opaque marker.

mod behaviour;
mod cone;
mod path;
mod spawn;
mod vision;

use bevy::prelude::*;
use rand::rngs::SmallRng;

use crate::state::{GameState, WorldGen};
use behaviour::{reset_adversaries, update_adversaries};
use cone::draw_vision_cones;
use spawn::spawn_adversaries;

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
        // Spawn on entering Loading so the dungeon map and spawn point exist.
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
