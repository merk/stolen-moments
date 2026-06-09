//! The Perlin-noise pass: thresholds noise into floor/wall caverns, plus the
//! small helpers ([`border_mask`], [`find_central_floor`]) shared by the level
//! sources.

use noise::{NoiseFn, Perlin};

use super::map::{LevelMap, MAP_HEIGHT, MAP_WIDTH, Tile};

/// Frequency of the noise sampling. Smaller = larger, smoother caverns.
const NOISE_SCALE: f64 = 0.11;

/// Noise values above this become walkable floor. Higher = less floor.
const FLOOR_THRESHOLD: f64 = -0.05;

/// Threshold Perlin noise into floor/wall, with a forced solid border.
pub(crate) fn build_noise_map(seed: u32) -> LevelMap {
    let perlin = Perlin::new(seed);
    let mut map = LevelMap::filled_with_walls(MAP_WIDTH, MAP_HEIGHT);

    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            // Keep a 1-tile solid border so the cavern is always enclosed.
            let border = x == 0 || y == 0 || x == MAP_WIDTH - 1 || y == MAP_HEIGHT - 1;
            let n = perlin.get([x as f64 * NOISE_SCALE, y as f64 * NOISE_SCALE]);
            if !border && n > FLOOR_THRESHOLD {
                map.set(x, y, Tile::Floor);
            }
        }
    }
    map
}

/// The map's 1-tile solid border, marked immutable so the carver never tunnels
/// out of bounds.
pub(crate) fn border_mask() -> Vec<bool> {
    let mut protected = vec![false; MAP_WIDTH * MAP_HEIGHT];
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            if x == 0 || y == 0 || x == MAP_WIDTH - 1 || y == MAP_HEIGHT - 1 {
                protected[y * MAP_WIDTH + x] = true;
            }
        }
    }
    protected
}

/// Find the walkable tile closest to the grid centre (used by `NoiseSource` and
/// as the hybrid source's spawn fallback).
pub(crate) fn find_central_floor(map: &LevelMap) -> (usize, usize) {
    let (cx, cy) = (map.width as i32 / 2, map.height as i32 / 2);
    let mut best: Option<(usize, usize)> = None;
    let mut best_dist = i32::MAX;

    for y in 0..map.height {
        for x in 0..map.width {
            if map.is_walkable(x, y) {
                let d = (x as i32 - cx).pow(2) + (y as i32 - cy).pow(2);
                if d < best_dist {
                    best_dist = d;
                    best = Some((x, y));
                }
            }
        }
    }
    best.unwrap_or((map.width / 2, map.height / 2))
}
