//! Getting caught: a guard that has locked onto the live player and closed to
//! contact fills a **grab meter**; hold the player in its grasp long enough and
//! the loop is broken. Breaking line/distance drains the meter (faster than it
//! fills), so a player who keeps moving can shake a chaser off — the player
//! outpaces a chase ([`crate::player`] `MOVE_SPEED` > [`CHASE_SPEED`]).
//!
//! What a catch *costs* is selectable for tuning via [`CatchConfig`] (cycled
//! with F8, see `debug.rs`): discard the run, bank it as a ghost anyway, or end
//! the game. The grab meter draws as a filling red ring at the feet.
//!
//! [`CatchConfig`] is this module's slice of dev control: it owns the knobs,
//! reads them itself, and the `debug` plugin is the only writer — so this
//! gameplay module never depends on the debug tooling.

use std::f32::consts::TAU;

use bevy::prelude::*;

use crate::adversary::{Adversary, Awareness};
use crate::player::Player;
use crate::state::GameState;
use crate::time_loop::{CloseLoop, LoopReset};

/// How close (world units, horizontal) a chasing guard must be to grip the
/// player. Player collision radius is ~0.3 and a guard's foot ring ~0.42, so
/// this triggers only on a genuine overlap.
const CONTACT_RADIUS: f32 = 0.6;
/// Seconds of unbroken contact needed to fill the meter and break the loop.
const GRAB_FILL_TIME: f32 = 1.2;
/// Seconds to drain a full meter once out of contact — shorter than the fill
/// time, so putting distance between you and the guard is rewarded.
const GRAB_DRAIN_TIME: f32 = 0.6;

/// Feet-ring radius for the grab meter gizmo, and its lift off the floor.
const METER_RADIUS: f32 = 0.55;
const METER_LIFT: f32 = 0.07;
const METER_SEGMENTS: usize = 32;

/// The live player's grab meter, `0.0` (free) to `1.0` (caught). Public so the
/// debug overlay can read it out.
#[derive(Resource, Default)]
pub struct Caught {
    pub progress: f32,
}

/// What happens when the grab meter fills. Selectable at runtime for tuning.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum CatchMode {
    /// Snap to spawn and reset coins, but bank **no** ghost — the run is wasted.
    #[default]
    Discard,
    /// Treat the catch like a voluntary close: bank the run as a ghost, reset.
    Bank,
    /// End the game: transition to the lose screen.
    GameOver,
}

impl CatchMode {
    /// Short overlay label for the active mode.
    pub fn label(self) -> &'static str {
        match self {
            CatchMode::Discard => "discard run",
            CatchMode::Bank => "bank ghost",
            CatchMode::GameOver => "game over",
        }
    }

    /// The next mode in the cycle, for the debug toggle.
    pub fn next(self) -> Self {
        match self {
            CatchMode::Discard => CatchMode::Bank,
            CatchMode::Bank => CatchMode::GameOver,
            CatchMode::GameOver => CatchMode::Discard,
        }
    }
}

/// This module's dev-control slice: what a catch does, plus whether the grab
/// meter is drawn. Owned and read here; written only by the `debug` plugin.
#[derive(Resource)]
pub struct CatchConfig {
    /// What happens to the run on a catch.
    pub mode: CatchMode,
    /// Draw the grab meter while it's filling.
    pub show_grab_meter: bool,
}

impl Default for CatchConfig {
    fn default() -> Self {
        Self {
            mode: CatchMode::default(),
            show_grab_meter: true,
        }
    }
}

pub struct CatchPlugin;

impl Plugin for CatchPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Caught>()
            .init_resource::<CatchConfig>()
            // A freshly built level starts the player free.
            .add_systems(OnEnter(GameState::Loading), reset_caught)
            .add_systems(
                Update,
                (track_grab, draw_grab_meter).run_if(in_state(GameState::Playing)),
            )
            // A loop restart (whatever caused it) frees the player again.
            .add_observer(clear_caught_on_loop);
    }
}

/// Zero the meter on a fresh level build.
fn reset_caught(mut caught: ResMut<Caught>) {
    caught.progress = 0.0;
}

/// Zero the meter whenever a loop restarts.
fn clear_caught_on_loop(_reset: On<LoopReset>, mut caught: ResMut<Caught>) {
    caught.progress = 0.0;
}

/// Fill the grab meter while a chasing guard holds the player in contact, drain
/// it otherwise, and fire the configured consequence once it tops out.
fn track_grab(
    time: Res<Time>,
    config: Res<CatchConfig>,
    mut caught: ResMut<Caught>,
    player: Query<&Transform, With<Player>>,
    adversaries: Query<(&Transform, &Awareness), With<Adversary>>,
    mut close: MessageWriter<CloseLoop>,
    mut next: ResMut<NextState<GameState>>,
) {
    let Ok(player_t) = player.single() else {
        return;
    };
    let here = player_t.translation;

    // Only a guard actively chasing can grab; a patrolling brush-past doesn't.
    let gripped = adversaries
        .iter()
        .any(|(t, a)| a.is_chasing() && horizontal_dist(t.translation, here) <= CONTACT_RADIUS);

    let dt = time.delta_secs();
    if gripped {
        caught.progress = (caught.progress + dt / GRAB_FILL_TIME).min(1.0);
    } else {
        caught.progress = (caught.progress - dt / GRAB_DRAIN_TIME).max(0.0);
    }

    if caught.progress >= 1.0 {
        caught.progress = 0.0;
        match config.mode {
            CatchMode::Discard => {
                close.write(CloseLoop { bank: false });
            }
            CatchMode::Bank => {
                close.write(CloseLoop { bank: true });
            }
            CatchMode::GameOver => {
                next.set(GameState::GameOver);
            }
        }
    }
}

/// Draw the grab meter as a red ring at the player's feet, filling clockwise
/// from the top as the meter rises. Hidden while the meter is empty.
fn draw_grab_meter(
    config: Res<CatchConfig>,
    caught: Res<Caught>,
    player: Query<&Transform, With<Player>>,
    mut gizmos: Gizmos,
) {
    if !config.show_grab_meter || caught.progress <= 0.0 {
        return;
    }
    let Ok(player_t) = player.single() else {
        return;
    };
    let centre = player_t.translation + Vec3::Y * METER_LIFT;

    // Faint full ring as the track, then the filled portion in bright red.
    draw_arc(
        &mut gizmos,
        centre,
        TAU,
        Color::srgba(0.5, 0.1, 0.1, 0.4),
        METER_SEGMENTS,
    );
    let segs = ((METER_SEGMENTS as f32) * caught.progress).ceil().max(1.0) as usize;
    draw_arc(
        &mut gizmos,
        centre,
        TAU * caught.progress,
        Color::srgb(1.0, 0.15, 0.1),
        segs,
    );
}

/// Polyline an arc of `sweep` radians around `centre` on the ground plane,
/// starting at the top (−Z) and going clockwise.
fn draw_arc(gizmos: &mut Gizmos, centre: Vec3, sweep: f32, color: Color, segments: usize) {
    let mut prev: Option<Vec3> = None;
    for i in 0..=segments {
        let angle = -std::f32::consts::FRAC_PI_2 + sweep * (i as f32 / segments as f32);
        let point = centre + Vec3::new(angle.cos(), 0.0, angle.sin()) * METER_RADIUS;
        if let Some(previous) = prev {
            gizmos.line(previous, point, color);
        }
        prev = Some(point);
    }
}

/// Horizontal (XZ) distance between two world points.
fn horizontal_dist(a: Vec3, b: Vec3) -> f32 {
    Vec2::new(a.x - b.x, a.z - b.z).length()
}
