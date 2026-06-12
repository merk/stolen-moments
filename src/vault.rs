//! The locked vault: the heist objective. The vault is a sealed room whose loot
//! (coins scattered there by `props.rs`) is unreachable until its door is
//! opened. The door sits in the room's single doorway and, while locked, turns
//! that tile solid in the [`LevelMap`] so the player *and* guard pathing both
//! treat it as a wall.
//!
//! Opening requires the vault code — once the player has learned
//! [`Fact::VaultCodeKnown`] (see `employee.rs`/`persistence.rs`), pressing **E**
//! at the door opens it. The door re-locks on every [`LoopReset`]
//! ([`PersistPolicy::ResetEachLoop`]); because the code *persists* across loops,
//! re-opening on a later run is instant — the heist's worked example of
//! knowledge outlasting a reset.

use bevy::prelude::*;

use crate::adversary::Surveillance;
use crate::level::{LevelMap, RoomKind, Tile};
use crate::loading::LoadingAssets;
use crate::persistence::{Fact, PersistPolicy, Persistent};
use crate::player::Player;
use crate::state::{GameState, InGame, WorldGen};
use crate::time_loop::LoopReset;

/// How close (world units, on the floor plane) the player must be to a door to
/// interact with or be prompted by it.
const INTERACT_RANGE: f32 = 1.4;

/// The vault door entity: which tile it plugs and whether it's currently open.
#[derive(Component)]
struct VaultDoor {
    tile: (usize, usize),
    open: bool,
}

/// Marks the contextual interaction prompt text.
#[derive(Component)]
struct VaultPromptText;

pub struct VaultPlugin;

impl Plugin for VaultPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(GameState::Loading),
            (spawn_vault_door.in_set(WorldGen::Populate), spawn_prompt),
        )
        .add_systems(
            Update,
            (open_vault, update_prompt).run_if(in_state(GameState::Playing)),
        )
        .add_observer(relock_vault);
    }
}

/// Spawn the vault door at the Vault room's doorway and lock it (the doorway tile
/// becomes solid). Runs in `Populate`, so the map already exists.
fn spawn_vault_door(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut loading: ResMut<LoadingAssets>,
    mut map: ResMut<LevelMap>,
) {
    let Some((tile, rect)) = map
        .rooms()
        .iter()
        .find(|r| r.kind == RoomKind::Vault)
        .and_then(|r| r.doorway.map(|d| (d, r.rect)))
    else {
        return;
    };

    let scene = loading.track(
        asset_server.load(GltfAssetLabel::Scene(0).from_asset("Models/GLB format/gate.glb")),
    );

    // Align the gate with the wall it plugs: doorways on a left/right edge need a
    // quarter turn so the gate spans the gap rather than facing across it.
    let on_vertical_edge = tile.0 == rect.min_x || tile.0 == rect.max_x;
    let rotation = if on_vertical_edge {
        Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)
    } else {
        Quat::IDENTITY
    };

    commands.spawn((
        SceneRoot(scene),
        Transform::from_translation(map.tile_to_world(tile.0, tile.1)).with_rotation(rotation),
        Visibility::Visible,
        VaultDoor { tile, open: false },
        PersistPolicy::ResetEachLoop,
        DespawnOnExit(InGame),
        Name::new("Vault door"),
    ));

    // Locked: the doorway tile reads as solid to movement and pathing.
    map.set(tile.0, tile.1, Tile::Wall);
}

/// With the code known, **E** near a closed door opens it: the doorway tile
/// becomes walkable again and the gate hides. The crack only works unobserved —
/// a guard whose cone covers the door foils the attempt, so the player must time
/// it for a gap in the watch.
fn open_vault(
    keys: Res<ButtonInput<KeyCode>>,
    persistent: Res<Persistent>,
    surveillance: Surveillance,
    mut map: ResMut<LevelMap>,
    player: Query<&Transform, With<Player>>,
    mut doors: Query<(&mut VaultDoor, &mut Visibility, &Transform)>,
) {
    if !keys.just_pressed(KeyCode::KeyE) || !persistent.knows(Fact::VaultCodeKnown) {
        return;
    }
    let Ok(player) = player.single() else {
        return;
    };

    for (mut door, mut visibility, transform) in &mut doors {
        if door.open
            || !within_range(player.translation, transform.translation)
            || surveillance.is_watched(&map, transform.translation)
        {
            continue;
        }
        door.open = true;
        map.set(door.tile.0, door.tile.1, Tile::Floor);
        *visibility = Visibility::Hidden;
        info!("Vault opened");
    }
}

/// On a loop restart, re-lock every door (`ResetEachLoop`): close it, make the
/// doorway solid again, and show the gate.
fn relock_vault(
    _reset: On<LoopReset>,
    mut map: ResMut<LevelMap>,
    mut doors: Query<(&mut VaultDoor, &mut Visibility)>,
) {
    for (mut door, mut visibility) in &mut doors {
        door.open = false;
        map.set(door.tile.0, door.tile.1, Tile::Wall);
        *visibility = Visibility::Visible;
    }
}

/// Show a contextual prompt when the player stands at a closed vault door:
/// how to open it once the code is known, or a nudge to find the code first.
fn update_prompt(
    persistent: Res<Persistent>,
    surveillance: Surveillance,
    map: Res<LevelMap>,
    player: Query<&Transform, With<Player>>,
    doors: Query<(&VaultDoor, &Transform)>,
    mut prompt: Query<(&mut Text, &mut Visibility), With<VaultPromptText>>,
) {
    let Ok((mut text, mut visibility)) = prompt.single_mut() else {
        return;
    };
    let near = player.single().ok().and_then(|player| {
        doors
            .iter()
            .find(|(door, t)| !door.open && within_range(player.translation, t.translation))
    });

    if let Some((_, door_transform)) = near {
        text.0 = if !persistent.knows(Fact::VaultCodeKnown) {
            "Vault locked — find the code".into()
        } else if surveillance.is_watched(&map, door_transform.translation) {
            "Vault watched — wait for the guard to look away".into()
        } else {
            "Press E to open vault".into()
        };
        *visibility = Visibility::Inherited;
    } else {
        *visibility = Visibility::Hidden;
    }
}

/// Horizontal (XZ) distance test against [`INTERACT_RANGE`].
fn within_range(a: Vec3, b: Vec3) -> bool {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    dx * dx + dz * dz <= INTERACT_RANGE * INTERACT_RANGE
}

fn spawn_prompt(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: 24.0,
            ..default()
        },
        TextColor(Color::srgb(0.95, 0.85, 0.4)),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(48.0),
            align_self: AlignSelf::Center,
            justify_self: JustifySelf::Center,
            ..default()
        },
        Visibility::Hidden,
        DespawnOnExit(InGame),
        VaultPromptText,
    ));
}
