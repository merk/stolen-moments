//! Developer tooling behind an F3 toggle: an on-screen overlay (FPS, seed,
//! state, entity counts) plus visualisation and convenience toggles the
//! tuning-heavy later phases lean on.
//!
//! Keys (all live regardless of game state):
//! - **F3** — show/hide the overlay text panel
//! - **F4** — toggle adversary vision cones
//! - **F5** — toggle the top-down tile floorplan overlay
//! - **F6** — force-close the current loop (same as Shift+R)
//! - **F7** — toggle a free-fly camera (pan with IJKL, raise/lower with U/O)
//!
//! Other gameplay modules stay independent of this one: `camera.rs` and
//! `adversary.rs` read the shared [`DebugSettings`] through `Option<Res<…>>`, so
//! they behave normally whether or not this plugin is present.

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;

use crate::adversary::Adversary;
use crate::camera::IsoCamera;
use crate::coins::Coin;
use crate::dungeon::{DungeonMap, TILE_SIZE};
use crate::seed::RunSeed;
use crate::state::GameState;
use crate::time_loop::{CloseLoop, Ghost};

/// Height above the floor at which the floorplan overlay is drawn.
const MAP_OVERLAY_LIFT: f32 = 0.1;
/// Free-fly camera pan speed (world units/sec).
const FREE_FLY_SPEED: f32 = 14.0;

/// Shared debug visualisation/behaviour flags. Owned by [`DebugPlugin`]; other
/// modules read it via `Option<Res<DebugSettings>>` so they never hard-depend on
/// debug tooling. Defaults match normal play (cones on, everything else off).
#[derive(Resource)]
pub struct DebugSettings {
    /// Whether the overlay text panel is shown.
    pub enabled: bool,
    /// Draw adversary vision cones. On by default — they're gameplay-relevant,
    /// not purely diagnostic — but F4 can hide them to declutter.
    pub vision_cones: bool,
    /// Draw the top-down tile floorplan overlay.
    pub map_overlay: bool,
    /// Detach the camera from the player for free panning.
    pub free_fly: bool,
}

impl Default for DebugSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            vision_cones: true,
            map_overlay: false,
            free_fly: false,
        }
    }
}

/// Marks the overlay text node so [`update_overlay`] can find and refresh it.
#[derive(Component)]
struct DebugOverlay;

pub struct DebugPlugin;

impl Plugin for DebugPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebugSettings>()
            .add_plugins(FrameTimeDiagnosticsPlugin::default())
            .add_systems(Startup, spawn_overlay)
            .add_systems(
                Update,
                (
                    toggle_debug,
                    update_overlay,
                    draw_map_overlay,
                    free_fly_camera,
                ),
            );
    }
}

/// Read the debug hotkeys and flip the matching flags.
fn toggle_debug(
    keys: Res<ButtonInput<KeyCode>>,
    state: Res<State<GameState>>,
    mut settings: ResMut<DebugSettings>,
    mut close: MessageWriter<CloseLoop>,
) {
    if keys.just_pressed(KeyCode::F3) {
        settings.enabled = !settings.enabled;
    }
    if keys.just_pressed(KeyCode::F4) {
        settings.vision_cones = !settings.vision_cones;
    }
    if keys.just_pressed(KeyCode::F5) {
        settings.map_overlay = !settings.map_overlay;
    }
    if keys.just_pressed(KeyCode::F7) {
        settings.free_fly = !settings.free_fly;
    }
    // Only meaningful while a loop is actually running; guard so the message
    // can't sit buffered and close a loop the moment play (re)starts.
    if keys.just_pressed(KeyCode::F6) && *state.get() == GameState::Playing {
        close.write(CloseLoop);
    }
}

fn spawn_overlay(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: 16.0,
            ..default()
        },
        TextColor(Color::srgb(0.55, 1.0, 0.7)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(10.0),
            right: Val::Px(12.0),
            ..default()
        },
        Visibility::Hidden,
        DebugOverlay,
        Name::new("Debug overlay"),
    ));
}

/// Show/hide the overlay with `enabled` and, while shown, refresh its contents.
// Reads several resources plus a count query per tracked entity kind; the param
// list is inherent to the readout, not a sign it should be split.
#[allow(clippy::too_many_arguments)]
fn update_overlay(
    settings: Res<DebugSettings>,
    diagnostics: Res<DiagnosticsStore>,
    seed: Res<RunSeed>,
    state: Res<State<GameState>>,
    mut overlay: Query<(&mut Text, &mut Visibility), With<DebugOverlay>>,
    adversaries: Query<(), With<Adversary>>,
    ghosts: Query<(), With<Ghost>>,
    coins: Query<(), With<Coin>>,
) {
    let Ok((mut text, mut visibility)) = overlay.single_mut() else {
        return;
    };

    *visibility = if settings.enabled {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if !settings.enabled {
        return;
    }

    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);

    text.0 = format!(
        "DEBUG  ·  F3 to hide\n\
         fps          {fps:.0}\n\
         seed         {}\n\
         state        {:?}\n\
         adversaries  {}\n\
         ghosts       {}\n\
         coins        {}\n\
         \n\
         F4 cones     {}\n\
         F5 map       {}\n\
         F6 close loop\n\
         F7 free-fly  {}",
        seed.0,
        state.get(),
        adversaries.iter().count(),
        ghosts.iter().count(),
        coins.iter().count(),
        on_off(settings.vision_cones),
        on_off(settings.map_overlay),
        on_off(settings.free_fly),
    );
}

fn on_off(flag: bool) -> &'static str {
    if flag { "on" } else { "off" }
}

/// Draw a flat square over every floor tile — a schematic floorplan that doubles
/// as the future home of the room-tag overlay (P1.1 will colour by room).
fn draw_map_overlay(
    settings: Res<DebugSettings>,
    map: Option<Res<DungeonMap>>,
    mut gizmos: Gizmos,
) {
    if !settings.map_overlay {
        return;
    }
    let Some(map) = map else {
        return;
    };

    // The rect gizmo lies in the XY plane; rotate it flat onto the ground (XZ).
    let flat = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    let size = Vec2::splat(TILE_SIZE * 0.9);
    let color = Color::srgba(0.2, 0.85, 1.0, 0.6);

    for y in 0..map.height {
        for x in 0..map.width {
            if !map.is_walkable(x, y) {
                continue;
            }
            let centre = map.tile_to_world(x, y) + Vec3::Y * MAP_OVERLAY_LIFT;
            gizmos.rect(Isometry3d::new(centre, flat), size, color);
        }
    }
}

/// While free-fly is on, pan the camera with IJKL (ground plane) and U/O (up /
/// down). `camera::follow_target` yields to this by reading the same flag.
fn free_fly_camera(
    settings: Res<DebugSettings>,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut camera: Query<&mut Transform, With<IsoCamera>>,
) {
    if !settings.free_fly {
        return;
    }
    let Ok(mut cam) = camera.single_mut() else {
        return;
    };

    // Pan relative to the fixed iso view: forward is the camera's horizontal
    // heading, right is that rotated 90°. Mirrors the player's input mapping.
    let offset = crate::camera::CAMERA_OFFSET;
    let forward = Vec3::new(-offset.x, 0.0, -offset.z).normalize();
    let right = Vec3::new(-forward.z, 0.0, forward.x);

    let mut dir = Vec3::ZERO;
    if keys.pressed(KeyCode::KeyI) {
        dir += forward;
    }
    if keys.pressed(KeyCode::KeyK) {
        dir -= forward;
    }
    if keys.pressed(KeyCode::KeyL) {
        dir += right;
    }
    if keys.pressed(KeyCode::KeyJ) {
        dir -= right;
    }
    if keys.pressed(KeyCode::KeyU) {
        dir += Vec3::Y;
    }
    if keys.pressed(KeyCode::KeyO) {
        dir -= Vec3::Y;
    }

    if dir != Vec3::ZERO {
        cam.translation += dir.normalize() * FREE_FLY_SPEED * time.delta_secs();
    }
}
