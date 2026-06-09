//! Perlin-noise driven dungeon generation and tile spawning.

use bevy::prelude::*;
use noise::{NoiseFn, Perlin};
use std::collections::VecDeque;

use crate::seed::RunSeed;

/// Grid dimensions (in tiles).
pub const MAP_WIDTH: usize = 48;
pub const MAP_HEIGHT: usize = 48;

/// World-space size of a single tile. Kenney mini-dungeon models are 1 unit.
pub const TILE_SIZE: f32 = 1.0;

/// Frequency of the noise sampling. Smaller = larger, smoother caverns.
const NOISE_SCALE: f64 = 0.11;

/// Noise values above this become walkable floor. Higher = less floor.
const FLOOR_THRESHOLD: f64 = -0.05;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tile {
    Floor,
    Wall,
}

/// The generated dungeon grid, stored row-major: `tiles[y * width + x]`.
#[derive(Resource)]
pub struct DungeonMap {
    pub width: usize,
    pub height: usize,
    tiles: Vec<Tile>,
}

impl DungeonMap {
    pub fn get(&self, x: usize, y: usize) -> Tile {
        self.tiles[y * self.width + x]
    }

    fn set(&mut self, x: usize, y: usize, tile: Tile) {
        self.tiles[y * self.width + x] = tile;
    }

    pub fn is_walkable(&self, x: usize, y: usize) -> bool {
        self.get(x, y) == Tile::Floor
    }

    /// Number of walkable floor tiles in the map.
    pub fn floor_count(&self) -> usize {
        self.tiles.iter().filter(|&&t| t == Tile::Floor).count()
    }

    /// Convert tile coordinates to the world-space center of that tile.
    /// The grid is centred on the world origin and laid out on the XZ plane.
    pub fn tile_to_world(&self, x: usize, y: usize) -> Vec3 {
        let wx = (x as f32 - self.width as f32 / 2.0) * TILE_SIZE;
        let wz = (y as f32 - self.height as f32 / 2.0) * TILE_SIZE;
        Vec3::new(wx, 0.0, wz)
    }

    /// Convert a world-space position back to tile coordinates, if in bounds.
    pub fn world_to_tile(&self, pos: Vec3) -> Option<(usize, usize)> {
        let fx = pos.x / TILE_SIZE + self.width as f32 / 2.0;
        let fz = pos.z / TILE_SIZE + self.height as f32 / 2.0;
        if fx < 0.0 || fz < 0.0 {
            return None;
        }
        let (x, y) = (fx.floor() as usize, fz.floor() as usize);
        if x < self.width && y < self.height {
            Some((x, y))
        } else {
            None
        }
    }

    /// True if the world position lands on a walkable tile.
    pub fn is_world_walkable(&self, pos: Vec3) -> bool {
        self.world_to_tile(pos)
            .map(|(x, y)| self.is_walkable(x, y))
            .unwrap_or(false)
    }
}

/// Where the player should start — the most central reachable floor tile.
#[derive(Resource, Clone, Copy)]
pub struct SpawnPoint {
    pub tile: (usize, usize),
    pub world: Vec3,
}

pub struct DungeonPlugin;

impl Plugin for DungeonPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, generate_dungeon);
    }
}

/// Generate the map from Perlin noise, keep only the floor region reachable
/// from the centre, store it as a resource, and spawn the tile meshes.
fn generate_dungeon(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    run_seed: Res<RunSeed>,
) {
    // Truncating to u32 is fine: Perlin only takes a u32 seed, and the derived
    // value is already well-mixed so the low 32 bits are as good as any.
    let seed = run_seed.derive("dungeon") as u32;

    let mut map = build_noise_map(seed);
    // Join every disconnected cavern into one component instead of discarding
    // the unreachable ones, so the whole carved map is usable in a single pass.
    connect_regions(&mut map);
    let spawn_tile = find_central_floor(&map);
    let spawn_world = map.tile_to_world(spawn_tile.0, spawn_tile.1);

    info!(
        "Generated dungeon with seed {seed}: {} floor tiles",
        map.floor_count()
    );

    spawn_tiles(&map, &mut commands, &asset_server);

    commands.insert_resource(SpawnPoint {
        tile: spawn_tile,
        world: spawn_world,
    });
    commands.insert_resource(map);
}

/// Threshold Perlin noise into floor/wall, with a forced solid border.
fn build_noise_map(seed: u32) -> DungeonMap {
    let perlin = Perlin::new(seed);
    let mut tiles = vec![Tile::Wall; MAP_WIDTH * MAP_HEIGHT];

    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            // Keep a 1-tile solid border so the cavern is always enclosed.
            let border = x == 0 || y == 0 || x == MAP_WIDTH - 1 || y == MAP_HEIGHT - 1;
            let n = perlin.get([x as f64 * NOISE_SCALE, y as f64 * NOISE_SCALE]);
            if !border && n > FLOOR_THRESHOLD {
                tiles[y * MAP_WIDTH + x] = Tile::Floor;
            }
        }
    }

    DungeonMap {
        width: MAP_WIDTH,
        height: MAP_HEIGHT,
        tiles,
    }
}

/// Carve corridors until every floor tile is part of one connected component.
///
/// We flood-fill to label all disconnected regions, seed the "connected" set
/// with the largest, then repeatedly attach whichever remaining region has the
/// closest pair of tiles to the connected set via an L-shaped tunnel.
fn connect_regions(map: &mut DungeonMap) {
    let mut regions = find_regions(map);
    if regions.len() <= 1 {
        return;
    }

    // Seed the connected set with the largest region.
    let largest = regions
        .iter()
        .enumerate()
        .max_by_key(|(_, r)| r.len())
        .map(|(i, _)| i)
        .expect("regions is non-empty");
    let mut connected: Vec<(usize, usize)> = regions.swap_remove(largest);

    while !regions.is_empty() {
        // Find the closest pair of tiles between any unconnected region and the
        // connected set, then carve a corridor across that gap.
        let mut best: Option<(usize, (usize, usize), (usize, usize), i32)> = None;
        for (ri, region) in regions.iter().enumerate() {
            for &(ax, ay) in &connected {
                for &(bx, by) in region {
                    let d = (ax as i32 - bx as i32).pow(2) + (ay as i32 - by as i32).pow(2);
                    if best.is_none_or(|(.., bd)| d < bd) {
                        best = Some((ri, (ax, ay), (bx, by), d));
                    }
                }
            }
        }

        let (ri, from, to, _) = best.expect("at least one region remains");
        carve_corridor(map, from, to);
        // The freshly attached region (and the corridor endpoints) join the
        // connected set so later regions can tunnel to either.
        connected.append(&mut regions.swap_remove(ri));
    }
}

/// Flood-fill every walkable tile into its connected component.
fn find_regions(map: &DungeonMap) -> Vec<Vec<(usize, usize)>> {
    let mut visited = vec![false; map.width * map.height];
    let mut regions = Vec::new();

    for y in 0..map.height {
        for x in 0..map.width {
            if !map.is_walkable(x, y) || visited[y * map.width + x] {
                continue;
            }

            let mut region = Vec::new();
            let mut queue = VecDeque::new();
            queue.push_back((x, y));
            visited[y * map.width + x] = true;

            while let Some((cx, cy)) = queue.pop_front() {
                region.push((cx, cy));
                for (nx, ny) in neighbours(cx, cy, map.width, map.height) {
                    let idx = ny * map.width + nx;
                    if !visited[idx] && map.is_walkable(nx, ny) {
                        visited[idx] = true;
                        queue.push_back((nx, ny));
                    }
                }
            }
            regions.push(region);
        }
    }

    regions
}

/// Carve a 1-wide L-shaped floor corridor from `from` to `to`: a horizontal
/// leg then a vertical leg. Both endpoints are interior floor tiles, so the
/// path never touches the forced solid border.
fn carve_corridor(map: &mut DungeonMap, from: (usize, usize), to: (usize, usize)) {
    let (mut x, y0) = from;
    while x != to.0 {
        map.set(x, y0, Tile::Floor);
        x = if x < to.0 { x + 1 } else { x - 1 };
    }
    let mut y = y0;
    while y != to.1 {
        map.set(to.0, y, Tile::Floor);
        y = if y < to.1 { y + 1 } else { y - 1 };
    }
    map.set(to.0, to.1, Tile::Floor);
}

/// Find the walkable tile closest to the grid centre (a good spawn point).
fn find_central_floor(map: &DungeonMap) -> (usize, usize) {
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

    // Fall back to the centre if the map somehow has no floor at all.
    best.unwrap_or((map.width / 2, map.height / 2))
}

fn neighbours(x: usize, y: usize, w: usize, h: usize) -> impl Iterator<Item = (usize, usize)> {
    let mut out = Vec::with_capacity(4);
    if x > 0 {
        out.push((x - 1, y));
    }
    if x + 1 < w {
        out.push((x + 1, y));
    }
    if y > 0 {
        out.push((x, y - 1));
    }
    if y + 1 < h {
        out.push((x, y + 1));
    }
    out.into_iter()
}

/// Spawn floor tiles for walkable cells, and the textured Kenney wall block for
/// any wall cell that borders the cavern (so we skip thousands of hidden cells).
fn spawn_tiles(map: &DungeonMap, commands: &mut Commands, asset_server: &AssetServer) {
    let floor_scene =
        asset_server.load(GltfAssetLabel::Scene(0).from_asset("Models/GLB format/floor.glb"));
    let wall_scene =
        asset_server.load(GltfAssetLabel::Scene(0).from_asset("Models/GLB format/wall.glb"));

    for y in 0..map.height {
        for x in 0..map.width {
            let pos = map.tile_to_world(x, y);
            match map.get(x, y) {
                Tile::Floor => {
                    commands.spawn((
                        SceneRoot(floor_scene.clone()),
                        Transform::from_translation(pos),
                        Name::new(format!("Floor ({x},{y})")),
                    ));
                }
                Tile::Wall if borders_floor(map, x, y) => {
                    commands.spawn((
                        SceneRoot(wall_scene.clone()),
                        Transform::from_translation(pos),
                        Name::new(format!("Wall ({x},{y})")),
                    ));
                }
                Tile::Wall => {}
            }
        }
    }
}

/// True if any of the 8 surrounding cells is walkable floor.
fn borders_floor(map: &DungeonMap, x: usize, y: usize) -> bool {
    for dy in -1i32..=1 {
        for dx in -1i32..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let (nx, ny) = (x as i32 + dx, y as i32 + dy);
            if nx >= 0
                && ny >= 0
                && (nx as usize) < map.width
                && (ny as usize) < map.height
                && map.is_walkable(nx as usize, ny as usize)
            {
                return true;
            }
        }
    }
    false
}
