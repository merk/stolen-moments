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
//! This file is the feature's header: the guard components, their tuning
//! constants, and the plugin wiring. A guard is an [`Adversary`] marker plus
//! small single-purpose components — [`Vision`] (the swept cone), [`Awareness`]
//! (suspicion/chase state), [`Navigation`] (route progress), and [`Post`] (the
//! spawn snapshot it resets to) — with [`PatrolRoute`] or [`Wander`] added only
//! for the kinds that need them, so a guard's *kind* is expressed by which
//! components it carries. The implementation lives in submodules: [`spawn`]
//! (placement), [`behaviour`] (the per-tick brain), [`cone`] (the vision gizmo),
//! and the pure helpers [`path`] (grid routing) and [`vision`] (sensing).

mod behaviour;
mod cone;
mod path;
mod spawn;
mod vision;

use bevy::prelude::*;
use rand::rngs::SmallRng;

use crate::state::{GameState, WorldGen};
use behaviour::{reset_adversaries, update_adversaries};
use cone::{draw_attention_meters, draw_vision_cones};
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

/// How a guard behaves while it hasn't yet locked onto a target. Used only at
/// spawn to drive appearance and which behaviour components to attach; the
/// running guard's kind is then encoded by those components.
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

/// Patrol vs chase — a guard's top-level state.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Unaware: moving per the guard's kind, cone sweeping side to side.
    Patrol,
    /// Locked onto a target, pathing toward its last-seen position.
    Chase,
}

/// Marker for guard entities. The only adversary component other modules see;
/// all behaviour state lives in the sibling components below.
#[derive(Component)]
pub struct Adversary;

/// Where a guard is looking: the swept vision cone.
#[derive(Component)]
struct Vision {
    /// Base facing (normalised, horizontal). While patrolling the cone sweeps
    /// around this; it tracks the current movement direction.
    heading: Vec3,
    /// The actual cone-centre direction this frame (heading + sweep, or the
    /// bearing to the target while chasing). Cached for the gizmo pass.
    look_dir: Vec3,
    /// Phase of the patrol sweep oscillation.
    sweep_phase: f32,
}

/// A guard's suspicion: whether it's patrolling or chasing, how much interest a
/// visible target has banked, and where that target was last seen.
///
/// Public as a component so sibling concerns can read a guard's alertness (the
/// catch system asks [`Awareness::is_chasing`] to know when a guard has the
/// player in its grasp); the fields stay private so its state machine is only
/// driven from within this module.
#[derive(Component)]
pub struct Awareness {
    mode: Mode,
    /// Interest accrued from time-in-cone; a chase begins once it crosses
    /// [`INTEREST_THRESHOLD`].
    interest: f32,
    /// Where the target was last seen; the chase destination.
    last_seen: Vec3,
}

impl Awareness {
    /// True while this guard is locked onto a target and pursuing it.
    pub fn is_chasing(&self) -> bool {
        matches!(self.mode, Mode::Chase)
    }
}

/// Progress along the current BFS route.
#[derive(Component, Default)]
struct Navigation {
    /// Tile waypoints currently being followed.
    path: Vec<(usize, usize)>,
    index: usize,
    /// Counts down to the next chase re-path.
    repath_timer: f32,
}

/// A guard's spawn snapshot: the post it returns to and the facing/sweep it
/// restores on every loop reset, so each run replays identically.
#[derive(Component)]
struct Post {
    home: Vec3,
    heading: Vec3,
    sweep_phase: f32,
}

/// A patrolling guard's fixed route (absent on static/wandering guards).
#[derive(Component)]
struct PatrolRoute {
    waypoints: Vec<(usize, usize)>,
    index: usize,
}

/// A wandering guard's private RNG for picking roam targets (absent on others).
#[derive(Component)]
struct Wander(SmallRng);

/// This module's dev-control slice: which guard overlays are drawn. Owned and
/// read here (by the gizmo systems); the `debug` plugin is the only writer, so
/// the adversary module never depends on the debug tooling. Both default on —
/// they're gameplay-relevant tells, not just diagnostics.
#[derive(Resource)]
pub struct AdversaryGizmos {
    /// Draw each guard's swept vision cone.
    pub vision_cones: bool,
    /// Draw each guard's attention meter (interest → chase).
    pub attention_meters: bool,
}

impl Default for AdversaryGizmos {
    fn default() -> Self {
        Self {
            vision_cones: true,
            attention_meters: true,
        }
    }
}

pub struct AdversaryPlugin;

impl Plugin for AdversaryPlugin {
    fn build(&self, app: &mut App) {
        // Spawn on entering Loading so the dungeon map and spawn point exist.
        // Sensing/decision/movement runs on FixedUpdate so behaviour is a pure
        // function of the seed and tick count, independent of frame rate — the
        // basis for guard routes repeating identically across loops. The cone
        // gizmo draws every frame in Update off the cached `look_dir`.
        app.init_resource::<AdversaryGizmos>()
            .add_systems(
                OnEnter(GameState::Loading),
                spawn_adversaries.in_set(WorldGen::Populate),
            )
            .add_systems(
                FixedUpdate,
                update_adversaries.run_if(in_state(GameState::Playing)),
            )
            .add_systems(
                Update,
                (draw_vision_cones, draw_attention_meters).run_if(in_state(GameState::Playing)),
            )
            .add_observer(reset_adversaries);
    }
}
