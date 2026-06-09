//! Player character: spawning, input-driven movement and wall collision.

use bevy::prelude::*;

use crate::camera::CameraTarget;
use crate::dungeon::{DungeonMap, SpawnPoint};
use crate::state::{GameState, InGame, WorldGen};

const MOVE_SPEED: f32 = 5.0;

/// Collision radius used to keep the player from clipping into walls.
const PLAYER_RADIUS: f32 = 0.3;

/// How quickly the character turns to face the movement direction.
const TURN_SPEED: f32 = 12.0;

#[derive(Component)]
pub struct Player;

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        // Spawned during the world build, after the dungeon (and SpawnPoint).
        app.add_systems(
            OnEnter(GameState::Loading),
            spawn_player.in_set(WorldGen::Populate),
        )
        .add_systems(Update, move_player.run_if(in_state(GameState::Playing)));
    }
}

fn spawn_player(mut commands: Commands, asset_server: Res<AssetServer>, spawn: Res<SpawnPoint>) {
    let scene = asset_server
        .load(GltfAssetLabel::Scene(0).from_asset("Models/GLB format/character-human.glb"));

    commands.spawn((
        SceneRoot(scene),
        Transform::from_translation(spawn.world),
        Player,
        CameraTarget,
        DespawnOnExit(InGame),
        Name::new("Player"),
    ));
}

fn move_player(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    map: Res<DungeonMap>,
    mut player: Query<&mut Transform, With<Player>>,
) {
    let Ok(mut transform) = player.single_mut() else {
        return;
    };

    // Gather raw WASD / arrow input.
    let mut input = Vec2::ZERO;
    if keys.any_pressed([KeyCode::KeyW, KeyCode::ArrowUp]) {
        input.y += 1.0;
    }
    if keys.any_pressed([KeyCode::KeyS, KeyCode::ArrowDown]) {
        input.y -= 1.0;
    }
    if keys.any_pressed([KeyCode::KeyD, KeyCode::ArrowRight]) {
        input.x += 1.0;
    }
    if keys.any_pressed([KeyCode::KeyA, KeyCode::ArrowLeft]) {
        input.x -= 1.0;
    }

    if input == Vec2::ZERO {
        return;
    }
    input = input.normalize();

    // Map input onto the camera-relative ground plane so "up" is up-screen.
    // `crate::camera::CAMERA_OFFSET` defines the view direction; its horizontal
    // component gives the camera's forward, and forward rotated -90° gives right.
    let offset = crate::camera::CAMERA_OFFSET;
    let forward = Vec3::new(-offset.x, 0.0, -offset.z).normalize();
    let right = Vec3::new(-forward.z, 0.0, forward.x);
    let move_dir = (forward * input.y + right * input.x).normalize_or_zero();

    let distance = MOVE_SPEED * time.delta_secs();
    let pos = transform.translation;

    // Resolve each axis independently so the player slides along walls.
    let mut new_pos = pos;
    let step_x = Vec3::new(move_dir.x * distance, 0.0, 0.0);
    if step_x != Vec3::ZERO {
        let probe = new_pos + step_x + Vec3::new(move_dir.x.signum() * PLAYER_RADIUS, 0.0, 0.0);
        if map.is_world_walkable(probe) {
            new_pos += step_x;
        }
    }
    let step_z = Vec3::new(0.0, 0.0, move_dir.z * distance);
    if step_z != Vec3::ZERO {
        let probe = new_pos + step_z + Vec3::new(0.0, 0.0, move_dir.z.signum() * PLAYER_RADIUS);
        if map.is_world_walkable(probe) {
            new_pos += step_z;
        }
    }

    transform.translation = new_pos;

    // Smoothly rotate to face the movement direction. The Kenney character's
    // front is +Z, but `looking_to` aligns -Z, so we negate to face forward.
    if move_dir != Vec3::ZERO {
        let target = Transform::from_translation(new_pos)
            .looking_to(-move_dir, Vec3::Y)
            .rotation;
        transform.rotation = transform
            .rotation
            .slerp(target, (TURN_SPEED * time.delta_secs()).min(1.0));
    }
}
