//! Roaming adversaries that hunt the player and any replaying ghosts.
//!
//! Guards come in three [`GuardKind`]s that differ in how they move while
//! unaware — a **static** guard holds a fixed post and only sweeps its cone, a
//! **patrolling** guard walks a fixed seeded route, and a **wandering** guard
//! roams to random tiles. All three share one sensing model: a vision cone
//! sweeps side to side, and a target lingering inside it (within range, with
//! clear line of sight) fills a per-guard **interest** meter. The instant a
//! target banks interest the cone locks onto it and the guard halts to stare it
//! down, so the sighting builds rather than slipping past the sweep. Crossing
//! the interest threshold trips the guard into a chase toward the target's
//! last-known position; brief or distant exposure decays harmlessly. A guard
//! that loses its quarry mid-chase doesn't forget instantly — it drops into an
//! alarmed **search**, sweeping wider and faster while investigating tiles
//! around the last-seen spot and holding a baseline of interest, so a glimpse
//! snaps it straight back into the chase before it eventually stands down.
//! Everything
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
mod overlay;
mod path;
mod spawn;
mod vision;

use bevy::prelude::*;
use rand::rngs::SmallRng;

use crate::state::{GameState, WorldGen};
use behaviour::{reset_adversaries, update_adversaries};
use cone::draw_vision_cones;
use overlay::update_guard_overlays;
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
/// Keep a patroller's waypoints within this tile radius of its spawn, so it walks
/// a coherent local beat instead of crisscrossing the whole map.
const PATROL_RADIUS: i32 = 10;

/// Wander/patrol speed (world units/sec) while unaware.
const PATROL_SPEED: f32 = 2.6;
/// Pursuit speed once a target has been spotted — faster than patrol so a
/// fleeing player can't simply outrun a lock-on at equal speed forever.
const CHASE_SPEED: f32 = 4.2;
/// How quickly the body slerps to face the vision cone's direction.
const TURN_SPEED: f32 = 8.0;
/// Easing time (seconds) for the cone's facing — roughly how long it takes to
/// glide most of the way to a new target direction. A critically-damped
/// `smooth_turn` accelerates from rest and decelerates into place over this time,
/// so glances, lock-ons, and lost-target turns feel like a head turning rather
/// than a cone snapping. Comfortably shorter than the scan step hold so each
/// cadence glance settles before the next.
const CONE_SMOOTH_TIME: f32 = 0.28;

/// Peak swing of the cone away from the heading while scanning at a waypoint
/// (radians). The guard only sweeps while *stopped*; on the move the cone points
/// straight along its heading. Widened alongside the narrower cone so the swept
/// arc still covers comparable ground.
const SWEEP_AMPLITUDE: f32 = 1.1;
/// Cone sweep amplitude while searching — wider than the patrol scan, to sweep
/// more ground hunting for a target that just slipped out of sight.
const SEARCH_SWEEP_AMPLITUDE: f32 = 1.5;

/// The scan look-cadence: discrete offsets (as a fraction of the sweep amplitude)
/// the cone settles on in turn while dwelling, eased between by `smooth_turn`.
/// Reads as a deliberate "glance right… settle… glance left… settle" rather than
/// a metronomic oscillation.
const SCAN_CADENCE: [f32; 6] = [0.5, 1.0, 0.0, -0.5, -1.0, 0.0];
/// Seconds the cone holds on each [`SCAN_CADENCE`] step while patrolling.
const SCAN_STEP_INTERVAL: f32 = 0.9;
/// Faster cadence step while searching — an agitated, hurried sweep.
const SEARCH_SCAN_STEP_INTERVAL: f32 = 0.45;

/// Seconds a guard pauses to scan at each patrol/wander waypoint before moving on.
const DWELL_DURATION: f32 = 2.0;
/// Shorter pause at each search investigate-tile — more urgent than a patrol.
const SEARCH_DWELL_DURATION: f32 = 0.8;
/// Move speed while searching: faster than an unaware patrol, slower than a
/// committed chase.
const SEARCH_SPEED: f32 = 3.4;
/// Seconds a guard hunts a lost target before standing down to patrol.
const SEARCH_DURATION: f32 = 6.0;
/// Baseline interest a guard holds while alarmed/searching, so re-spotting the
/// target trips a chase almost immediately rather than rebuilding from zero.
const SEARCH_INTEREST_FLOOR: f32 = 0.3;
/// Tiles out from the last-seen position a searching guard will investigate.
const SEARCH_RING_RADIUS: i32 = 3;

/// Interest needed to trip a guard from patrol into a chase. Low, because the
/// cone now locks onto a target the instant it banks any interest (see the
/// patrol look-direction in `behaviour`), so a target caught in the cone is
/// tracked and roused into a chase quickly rather than slipping past the sweep.
const INTEREST_THRESHOLD: f32 = 0.5;
/// Interest gained per second with a target dead-centre and point-blank; scaled
/// down with distance by [`INTEREST_MIN_FACTOR`] so far sightings build slowly.
const INTEREST_GAIN: f32 = 1.6;
/// Floor on the distance-scaled gain — a target at the cone's far edge still
/// raises interest at this fraction of [`INTEREST_GAIN`].
const INTEREST_MIN_FACTOR: f32 = 0.4;
/// Floor on the angle-scaled gain — a target at the cone's *side* edge raises
/// interest at this fraction of a dead-centre sighting, so skirting the
/// periphery of a guard's view is genuinely safer than walking through it.
const INTEREST_EDGE_FACTOR: f32 = 0.35;
/// Interest lost per second while no target is visible.
const INTEREST_DECAY: f32 = 0.7;
/// Cap so interest can't bank arbitrarily high before a chase begins.
const INTEREST_MAX: f32 = 1.25;
/// Brief "spotted you!" hold once a sighting crosses [`INTEREST_THRESHOLD`]: the
/// guard locks on and flashes its alert before bursting into the chase, so a
/// clean break of line of sight within the beat still escapes.
const ALERT_DELAY: f32 = 0.4;

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

/// A guard's top-level alertness state.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Unaware: moving per the guard's kind, cone sweeping side to side.
    Patrol,
    /// Locked onto a target, pathing toward its last-seen position.
    Chase,
    /// Alarmed: lost the target but still hunting — sweeping a wider, faster
    /// cone while investigating tiles around where it was last seen, holding a
    /// baseline of interest so a re-sighting snaps straight back into a chase.
    Search,
}

/// Marker for guard entities. The only adversary component other modules see;
/// all behaviour state lives in the sibling components below.
#[derive(Component)]
pub struct Adversary;

/// Where a guard is looking: the swept vision cone.
#[derive(Component)]
struct Vision {
    /// Base facing (normalised, horizontal): the guard's movement direction,
    /// tracked as it walks. The cone points straight along it while travelling
    /// and sweeps *around* it while stopped to scan.
    heading: Vec3,
    /// The actual cone-centre direction this frame (heading while moving, a
    /// scan offset while dwelling, or the bearing to the target while chasing),
    /// eased toward its goal each tick. Cached for the gizmo pass.
    look_dir: Vec3,
    /// The cone's angular velocity (rad/s), carried across frames so `smooth_turn`
    /// can accelerate and decelerate the facing rather than turning at a flat rate.
    look_vel: f32,
    /// Clock advanced only while scanning, stepping through the look cadence.
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
    /// [`INTEREST_THRESHOLD`]. While searching it holds at
    /// [`SEARCH_INTEREST_FLOOR`] rather than draining to zero.
    interest: f32,
    /// Where the target was last seen; the chase destination and the centre of
    /// the search.
    last_seen: Vec3,
    /// Counts down while searching; on reaching zero the guard stands down to
    /// patrol. Unused outside [`Mode::Search`].
    search_timer: f32,
    /// Deterministic ring of tiles to investigate around `last_seen` while
    /// searching, with `search_step` the next index into it. Recomputed on
    /// entering a search so the pattern replays identically across loops.
    search_points: Vec<(usize, usize)>,
    search_step: usize,
    /// Counts down during the pre-chase "spotted you!" beat once a sighting
    /// crosses the threshold; the chase commits when it reaches zero. Zero
    /// whenever the guard isn't mid-double-take.
    alert_timer: f32,
}

impl Awareness {
    /// True while this guard is locked onto a target and pursuing it.
    pub fn is_chasing(&self) -> bool {
        matches!(self.mode, Mode::Chase)
    }

    /// True during the pre-chase "spotted you!" beat — locked on, flashing its
    /// alert, about to commit to a chase.
    fn is_alerting(&self) -> bool {
        self.alert_timer > 0.0
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
    /// Counts down while stopped at a waypoint, scanning, before the guard picks
    /// its next destination. Re-armed each tick while travelling, so it's full
    /// the moment the guard arrives.
    dwell_timer: f32,
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
    /// Show each guard's floating state overlays — the emote (`?`/dots/`!`) and
    /// the attention bar that fills as a sighting builds toward a chase.
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
                (draw_vision_cones, update_guard_overlays).run_if(in_state(GameState::Playing)),
            )
            .add_observer(reset_adversaries);
    }
}
