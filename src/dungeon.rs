//! Perlin-noise driven dungeon generation and tile spawning.

use bevy::prelude::*;
use noise::{NoiseFn, Perlin};
use std::collections::VecDeque;

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
fn generate_dungeon(mut commands: Commands, asset_server: Res<AssetServer>) {
    let seed: u32 = rand::random();
    info!("Generating dungeon with seed {seed}");

    let mut map = build_noise_map(seed);
    let spawn_tile = keep_central_region(&mut map);
    let spawn_world = map.tile_to_world(spawn_tile.0, spawn_tile.1);

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

/// Flood-fill from the most central floor tile and turn every floor tile that
/// isn't reachable into a wall, guaranteeing a single connected dungeon.
fn keep_central_region(map: &mut DungeonMap) -> (usize, usize) {
    let start = find_central_floor(map);

    let mut reachable = vec![false; map.width * map.height];
    let mut queue = VecDeque::new();
    queue.push_back(start);
    reachable[start.1 * map.width + start.0] = true;

    while let Some((x, y)) = queue.pop_front() {
        for (nx, ny) in neighbours(x, y, map.width, map.height) {
            let idx = ny * map.width + nx;
            if !reachable[idx] && map.is_walkable(nx, ny) {
                reachable[idx] = true;
                queue.push_back((nx, ny));
            }
        }
    }

    for y in 0..map.height {
        for x in 0..map.width {
            if map.is_walkable(x, y) && !reachable[y * map.width + x] {
                map.set(x, y, Tile::Wall);
            }
        }
    }

    start
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
