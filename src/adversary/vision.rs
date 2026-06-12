//! Adversary sensing: the vision-cone test, grid line-of-sight, and the small
//! XZ-plane vector helpers both sensing and movement lean on. Pure functions
//! over world-space points and the [`LevelMap`] — no `Adversary` state.

use bevy::prelude::*;

use crate::level::LevelMap;

/// How far the vision cone reaches (world units).
pub(super) const VISION_RANGE: f32 = 9.0;
/// Half-angle of the cone (radians). ~23° each side of centre.
pub(super) const VISION_HALF_ANGLE: f32 = 0.4;

/// Step size used when marching the line-of-sight ray across the grid.
const LOS_STEP: f32 = 0.25;

/// Return the first target (in caller-supplied priority order) that sits inside
/// the cone — within range, within the half-angle, and with clear line of
/// sight — looking from `pos` along `look_dir`.
pub(super) fn first_visible(
    map: &LevelMap,
    pos: Vec3,
    look_dir: Vec3,
    targets: &[Vec3],
) -> Option<Vec3> {
    let min_cos = VISION_HALF_ANGLE.cos();

    for &target in targets {
        let to = horizontal(target - pos);
        let dist = to.length();
        if dist > VISION_RANGE {
            continue;
        }
        // A target right on top of us is trivially "seen".
        if dist > 1e-3 {
            let dir = to / dist;
            if dir.dot(look_dir) < min_cos {
                continue;
            }
            if !clear_line_of_sight(map, pos, target) {
                continue;
            }
        }
        return Some(target);
    }

    None
}

/// March across the grid between two world points; blocked by any wall tile.
fn clear_line_of_sight(map: &LevelMap, from: Vec3, to: Vec3) -> bool {
    let delta = horizontal(to - from);
    let dist = delta.length();
    if dist < 1e-3 {
        return true;
    }
    let steps = (dist / LOS_STEP).ceil() as i32;
    for i in 1..=steps {
        let p = from + delta * (i as f32 / steps as f32);
        if !map.is_world_walkable(p) {
            return false;
        }
    }
    true
}

/// Drop the Y component, keeping a flat XZ-plane vector.
pub(super) fn horizontal(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

/// Rotate a horizontal vector about the Y axis by `angle` radians.
pub(super) fn rotate_y(v: Vec3, angle: f32) -> Vec3 {
    let (s, c) = angle.sin_cos();
    Vec3::new(v.x * c + v.z * s, 0.0, -v.x * s + v.z * c)
}

/// Critically-damped angular easing: ease `current` toward `target` about the Y
/// axis, returning the new horizontal facing. `vel` is the cone's angular
/// velocity (rad/s), carried across frames — it's what makes the turn *accelerate
/// from rest and decelerate into place* rather than snapping to a flat rate, so
/// every lock-on, lost-target turn, and scan glance glides. `smooth_time` is
/// roughly how long it takes to close most of the gap; no overshoot. This is the
/// classic game `SmoothDamp`, applied to the signed angle still to be turned.
pub(super) fn smooth_turn(
    current: Vec3,
    target: Vec3,
    vel: &mut f32,
    smooth_time: f32,
    dt: f32,
) -> Vec3 {
    let current = current.normalize_or_zero();
    let target = target.normalize_or_zero();
    if current == Vec3::ZERO {
        *vel = 0.0;
        return target;
    }
    if target == Vec3::ZERO {
        return current;
    }

    // The signed shortest angle still to rotate to reach the target, matching
    // `rotate_y`'s positive direction (its tangent is `(v.z, -v.x)`). This `gap`
    // is the quantity we ease to zero.
    let tangent = Vec3::new(current.z, 0.0, -current.x);
    let mag = current.dot(target).clamp(-1.0, 1.0).acos();
    let gap = if target.dot(tangent) >= 0.0 {
        mag
    } else {
        -mag
    };

    // SmoothDamp the gap toward zero, then rotate by however much it closed this
    // frame. `vel` tracks the gap's velocity so motion stays continuous even when
    // the target jumps.
    let omega = 2.0 / smooth_time.max(1e-4);
    let x = omega * dt;
    let exp = 1.0 / (1.0 + x + 0.48 * x * x + 0.235 * x * x * x);
    let temp = (*vel + omega * gap) * dt;
    *vel = (*vel - omega * temp) * exp;
    let remaining = (gap + temp) * exp;
    rotate_y(current, gap - remaining)
}
