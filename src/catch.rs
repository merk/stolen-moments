//! Getting caught: a guard that has locked onto the live player and closed to
//! contact fills a **grab meter**; hold the player in its grasp long enough and
//! the loop is broken. Breaking line/distance drains the meter (faster than it
//! fills), so a player who keeps moving can shake a chaser off — the player
//! outpaces a chase ([`crate::player`] `MOVE_SPEED` > [`CHASE_SPEED`]).
//!
//! What a catch *costs* is selectable for tuning via [`CatchConfig`] (cycled
//! with F8, see `debug.rs`): discard the run, bank it as a ghost anyway, or end
//! the game. The grab meter draws as a red progress bar floating over the player.
//!
//! [`CatchConfig`] is this module's slice of dev control: it owns the knobs,
//! reads them itself, and the `debug` plugin is the only writer — so this
//! gameplay module never depends on the debug tooling.

use bevy::prelude::*;

use crate::adversary::{Adversary, Awareness};
use crate::billboard::{BAR_HEIGHT, BAR_WIDTH, Billboard, OverlayAssets};
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

/// Height above the player at which the grab bar floats.
const BAR_LIFT: f32 = 2.05;

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
            // Attach the grab bar to the player as soon as it spawns (any state).
            .add_systems(Update, attach_grab_bar)
            .add_systems(
                Update,
                (track_grab, update_grab_bar).run_if(in_state(GameState::Playing)),
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

/// The grab bar's fill quad, X-scaled to the grab progress each frame. Its
/// parent (the bar root) is toggled whole to show/hide the meter.
#[derive(Component)]
struct GrabBar;
#[derive(Component)]
struct GrabFill;

/// Attach a floating grab bar to the player the moment it spawns: a billboard
/// root holding a dark track and a red fill, hidden until the meter rises.
fn attach_grab_bar(
    mut commands: Commands,
    assets: Option<Res<OverlayAssets>>,
    players: Query<Entity, Added<Player>>,
) {
    let Some(assets) = assets else {
        return;
    };
    for player in &players {
        let bar = commands
            .spawn((
                GrabBar,
                Billboard,
                Transform::from_xyz(0.0, BAR_LIFT, 0.0),
                Visibility::Hidden,
                ChildOf(player),
                Name::new("Grab bar"),
            ))
            .id();
        commands.spawn((
            Mesh3d(assets.bar_track_mesh.clone()),
            MeshMaterial3d(assets.bar_track_material.clone()),
            Transform::from_scale(Vec3::new(BAR_WIDTH, BAR_HEIGHT, 1.0)),
            ChildOf(bar),
        ));
        commands.spawn((
            GrabFill,
            Mesh3d(assets.bar_fill_mesh.clone()),
            MeshMaterial3d(assets.bar_danger_material.clone()),
            Transform {
                translation: Vec3::new(-BAR_WIDTH * 0.5, 0.0, 0.01),
                scale: Vec3::new(0.0, BAR_HEIGHT, 1.0),
                ..default()
            },
            ChildOf(bar),
        ));
    }
}

/// Show the grab bar while the meter is filling and scale its fill to progress;
/// hide it when empty or switched off.
fn update_grab_bar(
    config: Res<CatchConfig>,
    caught: Res<Caught>,
    mut bars: Query<&mut Visibility, With<GrabBar>>,
    mut fills: Query<&mut Transform, With<GrabFill>>,
) {
    let show = config.show_grab_meter && caught.progress > 0.0;
    for mut vis in &mut bars {
        *vis = if show {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    for mut transform in &mut fills {
        transform.scale.x = BAR_WIDTH * caught.progress;
    }
}

/// Horizontal (XZ) distance between two world points.
fn horizontal_dist(a: Vec3, b: Vec3) -> f32 {
    Vec2::new(a.x - b.x, a.z - b.z).length()
}
