//! Isometric orthographic camera that smoothly follows its target.

use bevy::camera::ScalingMode;
use bevy::prelude::*;

use crate::state::GameState;

/// Offset of the camera from its follow target. The (equal X/Z, larger Y)
/// vector gives the classic 3/4 isometric viewing angle.
pub const CAMERA_OFFSET: Vec3 = Vec3::new(12.0, 16.0, 12.0);

/// How many world units tall the viewport shows. Smaller = more zoomed in.
const VIEW_HEIGHT: f32 = 18.0;

/// Higher = snappier follow.
const FOLLOW_SPEED: f32 = 6.0;

#[derive(Component)]
pub struct IsoCamera;

/// Marker for the entity the camera should follow. Whichever module owns the
/// focal entity (the player today) adds this; the camera stays decoupled from it.
/// Following needs a position, so a `Transform` is required.
#[derive(Component)]
#[require(Transform)]
pub struct CameraTarget;

pub struct IsoCameraPlugin;

impl Plugin for IsoCameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_camera)
            .add_systems(Update, follow_target.run_if(in_state(GameState::Playing)));
    }
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Projection::from(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: VIEW_HEIGHT,
            },
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_translation(CAMERA_OFFSET).looking_at(Vec3::ZERO, Vec3::Y),
        IsoCamera,
    ));
}

fn follow_target(
    time: Res<Time>,
    debug: Option<Res<crate::debug::DebugSettings>>,
    target: Query<&Transform, (With<CameraTarget>, Without<IsoCamera>)>,
    mut camera: Query<&mut Transform, With<IsoCamera>>,
) {
    // The debug free-fly camera drives the transform itself; don't fight it.
    if debug.is_some_and(|d| d.free_fly) {
        return;
    }
    let Ok(target) = target.single() else {
        return;
    };
    let Ok(mut cam) = camera.single_mut() else {
        return;
    };

    // Only translate to follow — the rotation stays fixed at the iso angle set
    // on spawn. Re-aiming each frame while the position lerps causes wobble.
    let target = target.translation + CAMERA_OFFSET;
    let t = (FOLLOW_SPEED * time.delta_secs()).min(1.0);
    cam.translation = cam.translation.lerp(target, t);
}
