//! The guard overlays: each guard's swept vision cone (tinted by its awareness)
//! plus a coloured ground ring marking its kind, so the player can read both a
//! guard's alertness and its behaviour at a glance.

use bevy::prelude::*;

use super::vision::{VISION_HALF_ANGLE, VISION_RANGE, rotate_y};
use super::{Awareness, CONE_LIFT, INTEREST_THRESHOLD, Mode, PatrolRoute, Vision, Wander};

/// Radius of the kind ring drawn at a guard's feet (world units; tile is 1).
const RING_RADIUS: f32 = 0.42;
/// Segments in the ring polygon.
const RING_SEGMENTS: usize = 20;

/// Draw each guard's overlays: the vision cone, then the kind ring.
///
/// Cone — two edge rays plus the far arc, yellow at rest, warming through orange
/// as interest builds, red once locked onto a target. Ring — a flat circle at
/// the feet coloured by kind (cool tones, distinct from the warm cone) so a
/// patrolling guard reads apart from a wandering or static one.
pub(super) fn draw_vision_cones(
    debug: Option<Res<crate::debug::DebugSettings>>,
    adversaries: Query<(
        &Transform,
        &Vision,
        &Awareness,
        Option<&PatrolRoute>,
        Option<&Wander>,
    )>,
    mut gizmos: Gizmos,
) {
    if debug.is_some_and(|d| !d.vision_cones) {
        return;
    }
    const ARC_SEGMENTS: usize = 12;

    for (transform, vision, awareness, patrol, wander) in &adversaries {
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

        draw_kind_ring(&mut gizmos, origin, kind_color(patrol, wander));
    }
}

/// Cone tint: red while chasing, otherwise yellow→orange as interest builds.
fn cone_color(awareness: &Awareness) -> Color {
    match awareness.mode {
        Mode::Chase => Color::srgb(1.0, 0.2, 0.15),
        Mode::Patrol => {
            let t = (awareness.interest / INTEREST_THRESHOLD).clamp(0.0, 1.0);
            Color::srgb(1.0, 0.85 - 0.45 * t, 0.2 - 0.05 * t)
        }
    }
}

/// Ring tint by kind, told apart by which behaviour component the guard carries:
/// patrolling (green), wandering (violet), or static (blue).
fn kind_color(patrol: Option<&PatrolRoute>, wander: Option<&Wander>) -> Color {
    if patrol.is_some() {
        Color::srgb(0.2, 1.0, 0.4)
    } else if wander.is_some() {
        Color::srgb(0.7, 0.3, 1.0)
    } else {
        Color::srgb(0.2, 0.6, 1.0)
    }
}

/// Draw a flat ring on the floor around `origin` as a line-segment polygon.
fn draw_kind_ring(gizmos: &mut Gizmos, origin: Vec3, color: Color) {
    let mut prev: Option<Vec3> = None;
    for i in 0..=RING_SEGMENTS {
        let angle = i as f32 / RING_SEGMENTS as f32 * std::f32::consts::TAU;
        let point = origin + Vec3::new(angle.cos(), 0.0, angle.sin()) * RING_RADIUS;
        if let Some(previous) = prev {
            gizmos.line(previous, point, color);
        }
        prev = Some(point);
    }
}
