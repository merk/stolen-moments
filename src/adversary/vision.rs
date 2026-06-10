//! Adversary sensing: the vision-cone test, grid line-of-sight, and the small
//! XZ-plane vector helpers both sensing and movement lean on. Pure functions
//! over world-space points and the [`LevelMap`] — no `Adversary` state.

use bevy::prelude::*;

use crate::level::LevelMap;

/// How far the vision cone reaches (world units).
pub(super) const VISION_RANGE: f32 = 9.0;
/// Half-angle of the cone (radians). ~34° each side of centre.
pub(super) const VISION_HALF_ANGLE: f32 = 0.6;

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
