//! The vision-cone gizmo: each guard's cone drawn on the floor, tinted by its
//! awareness so the player can read patrols at a glance.

use bevy::prelude::*;

use super::vision::{VISION_HALF_ANGLE, VISION_RANGE, rotate_y};
use super::{Adversary, CONE_LIFT, INTEREST_THRESHOLD, Mode};

/// Draw each adversary's vision cone on the floor: two edge rays plus the far
/// arc. Yellow at rest, warming through orange as interest builds, red once it's
/// locked onto a target.
pub(super) fn draw_vision_cones(
    debug: Option<Res<crate::debug::DebugSettings>>,
    adversaries: Query<(&Transform, &Adversary)>,
    mut gizmos: Gizmos,
) {
    if debug.is_some_and(|d| !d.vision_cones) {
        return;
    }
    const ARC_SEGMENTS: usize = 12;

    for (transform, adv) in &adversaries {
        let origin = transform.translation + Vec3::Y * CONE_LIFT;
        let color = cone_color(adv);

        let mut prev: Option<Vec3> = None;
        for i in 0..=ARC_SEGMENTS {
            let t = i as f32 / ARC_SEGMENTS as f32;
            let angle = -VISION_HALF_ANGLE + t * (2.0 * VISION_HALF_ANGLE);
            let point = origin + rotate_y(adv.look_dir, angle) * VISION_RANGE;

            // The first and last spokes are the cone's straight edges.
            if i == 0 || i == ARC_SEGMENTS {
                gizmos.line(origin, point, color);
            }
            if let Some(previous) = prev {
                gizmos.line(previous, point, color);
            }
            prev = Some(point);
        }
    }
}

/// Cone tint: red while chasing, otherwise yellow→orange as interest builds.
fn cone_color(adv: &Adversary) -> Color {
    match adv.mode {
        Mode::Chase => Color::srgb(1.0, 0.2, 0.15),
        Mode::Patrol => {
            let t = (adv.interest / INTEREST_THRESHOLD).clamp(0.0, 1.0);
            Color::srgb(1.0, 0.85 - 0.45 * t, 0.2 - 0.05 * t)
        }
    }
}
