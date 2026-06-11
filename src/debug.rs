//! Developer tooling behind an F3 toggle: an on-screen overlay (FPS, seed,
//! state, entity counts) plus visualisation and convenience toggles the
//! tuning-heavy later phases lean on.
//!
//! Keys (all live regardless of game state):
//! - **F3** — show/hide the overlay text panel
//! - **F4** — toggle adversary vision cones
//! - **F5** — toggle the top-down tile floorplan overlay
//! - **F6** — force-close the current loop (same as Shift+R)
//! - **F7** — toggle adversary attention meters
//! - **F8** — cycle what a catch does (discard run / bank ghost / game over)
//! - **F9** — toggle the player's grab meter
//!
//! This plugin only ever *writes* the dev-control flags that gameplay reads —
//! [`AdversaryGizmos`] and [`CatchConfig`] are owned by the modules that act on
//! them, so those modules never depend on this tooling. Pull `DebugPlugin` and
//! the game still runs on the flags' defaults.

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;

use crate::adversary::{Adversary, AdversaryGizmos};
use crate::catch::{CatchConfig, Caught};
use crate::coins::Coin;
use crate::level::{LevelMap, RoomKind, TILE_SIZE};
use crate::seed::RunSeed;
use crate::state::GameState;
use crate::time_loop::{CloseLoop, Ghost};

/// Height above the floor at which the floorplan overlay is drawn.
const MAP_OVERLAY_LIFT: f32 = 0.1;

/// This plugin's own flags: the overlay panel and the debug-only floorplan it
/// draws itself. Flags that *gameplay* reads live with the modules that own them
/// ([`AdversaryGizmos`], [`CatchConfig`]); this plugin writes those, it doesn't
/// store them. Defaults: overlay panel off, map overlay on.
#[derive(Resource)]
pub struct DebugSettings {
    /// Whether the overlay text panel is shown.
    pub enabled: bool,
    /// Draw the top-down tile floorplan overlay. On by default; F5 hides it.
    pub map_overlay: bool,
}

impl Default for DebugSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            map_overlay: true,
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
            .add_systems(Update, (toggle_debug, update_overlay, draw_map_overlay));
    }
}

/// Read the debug hotkeys and flip the matching flags. The gizmo/catch flags
/// live on the gameplay modules' resources; this is the only writer of them.
#[allow(clippy::too_many_arguments)]
fn toggle_debug(
    keys: Res<ButtonInput<KeyCode>>,
    state: Res<State<GameState>>,
    mut settings: ResMut<DebugSettings>,
    mut gizmos: ResMut<AdversaryGizmos>,
    mut catch: ResMut<CatchConfig>,
    mut close: MessageWriter<CloseLoop>,
) {
    if keys.just_pressed(KeyCode::F3) {
        settings.enabled = !settings.enabled;
    }
    if keys.just_pressed(KeyCode::F4) {
        gizmos.vision_cones = !gizmos.vision_cones;
    }
    if keys.just_pressed(KeyCode::F5) {
        settings.map_overlay = !settings.map_overlay;
    }
    // Only meaningful while a loop is actually running; guard so the message
    // can't sit buffered and close a loop the moment play (re)starts.
    if keys.just_pressed(KeyCode::F6) && *state.get() == GameState::Playing {
        close.write(CloseLoop { bank: true });
    }
    if keys.just_pressed(KeyCode::F7) {
        gizmos.attention_meters = !gizmos.attention_meters;
    }
    if keys.just_pressed(KeyCode::F8) {
        catch.mode = catch.mode.next();
    }
    if keys.just_pressed(KeyCode::F9) {
        catch.show_grab_meter = !catch.show_grab_meter;
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
    gizmos: Res<AdversaryGizmos>,
    catch: Res<CatchConfig>,
    caught: Res<Caught>,
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
         grab         {:.0}%\n\
         \n\
         F4 cones     {}\n\
         F5 map       {}\n\
         F6 close loop\n\
         F7 attention {}\n\
         F8 on catch  {}\n\
         F9 grab bar  {}",
        seed.0,
        state.get(),
        adversaries.iter().count(),
        ghosts.iter().count(),
        coins.iter().count(),
        caught.progress * 100.0,
        on_off(gizmos.vision_cones),
        on_off(settings.map_overlay),
        on_off(gizmos.attention_meters),
        catch.mode.label(),
        on_off(catch.show_grab_meter),
    );
}

fn on_off(flag: bool) -> &'static str {
    if flag { "on" } else { "off" }
}

/// Draw a flat square over every floor tile — a schematic floorplan, coloured by
/// the tile's semantic room kind (cyan for un-roomed cavern floor).
fn draw_map_overlay(settings: Res<DebugSettings>, map: Option<Res<LevelMap>>, mut gizmos: Gizmos) {
    if !settings.map_overlay {
        return;
    }
    let Some(map) = map else {
        return;
    };

    // The rect gizmo lies in the XY plane; rotate it flat onto the ground (XZ).
    let flat = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    let size = Vec2::splat(TILE_SIZE * 0.9);

    for y in 0..map.height {
        for x in 0..map.width {
            if !map.is_walkable(x, y) {
                continue;
            }
            let centre = map.tile_to_world(x, y) + Vec3::Y * MAP_OVERLAY_LIFT;
            gizmos.rect(
                Isometry3d::new(centre, flat),
                size,
                room_color(map.room_kind_at(x, y)),
            );
        }
    }
}

/// Floorplan tint for a tile by its room kind; un-roomed cavern floor stays cyan.
fn room_color(kind: Option<RoomKind>) -> Color {
    match kind {
        None => Color::srgba(0.2, 0.85, 1.0, 0.35),
        Some(RoomKind::Start) => Color::srgba(0.3, 1.0, 0.4, 0.7),
        Some(RoomKind::Lobby) => Color::srgba(0.6, 0.8, 1.0, 0.7),
        Some(RoomKind::GameTables) => Color::srgba(1.0, 0.85, 0.2, 0.7),
        Some(RoomKind::Vault) => Color::srgba(1.0, 0.55, 0.1, 0.8),
        Some(RoomKind::Security) => Color::srgba(1.0, 0.25, 0.2, 0.8),
        Some(RoomKind::Service) => Color::srgba(0.7, 0.7, 0.75, 0.7),
    }
}
