//! The guard's swept vision cone, drawn as a gizmo and tinted like a light: a
//! warm yellow-white while the guard is calm, warming through to red as a
//! sighting alarms it. The cone's *colour* is now only this coarse calm↔alert
//! signal — the precise state (suspicious / searching / chasing) is read off the
//! guard's floating emote and attention bar (see [`super::overlay`]).

use bevy::prelude::*;

use super::vision::{VISION_HALF_ANGLE, VISION_RANGE, rotate_y};
use super::{AdversaryGizmos, Awareness, CONE_LIFT, INTEREST_THRESHOLD, Mode, Vision};

/// Draw each guard's swept vision cone: two edge rays plus the far arc, coloured
/// by how alarmed the guard is.
pub(super) fn draw_vision_cones(
    gizmos_cfg: Res<AdversaryGizmos>,
    adversaries: Query<(&Transform, &Vision, &Awareness)>,
    mut gizmos: Gizmos,
) {
    if !gizmos_cfg.vision_cones {
        return;
    }
    const ARC_SEGMENTS: usize = 12;

    for (transform, vision, awareness) in &adversaries {
        let origin = transform.translation + Vec3::Y * CONE_LIFT;
        let color = cone_color(awareness);

        let mut prev: Option<Vec3> = None;
        for i in 0..=ARC_SEGMENTS {
            let t = i as f32 / ARC_SEGMENTS as f32;
            let angle = -VISION_HALF_ANGLE + t * (2.0 * VISION_HALF_ANGLE);
            let point = origin + rotate_y(vision.look_dir, angle) * VISION_RANGE;

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

/// Cone tint as a "light": a warm yellow-white at rest, lerping to red as the
/// guard grows alarmed. Patrol interest drives the blend up to the chase point;
/// an alarmed search or an active chase pins it fully red.
fn cone_color(awareness: &Awareness) -> Color {
    const CALM: Vec3 = Vec3::new(1.0, 0.95, 0.6);
    const ALERT: Vec3 = Vec3::new(1.0, 0.2, 0.12);
    let alarm = match awareness.mode {
        Mode::Chase | Mode::Search => 1.0,
        Mode::Patrol => (awareness.interest / INTEREST_THRESHOLD).clamp(0.0, 1.0),
    };
    let c = CALM.lerp(ALERT, alarm);
    Color::srgb(c.x, c.y, c.z)
}
