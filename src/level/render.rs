//! Turns a built [`LevelMap`] into tile meshes: a floor model for every
//! walkable cell and the Kenney wall block for any wall that borders the cavern
//! (so the thousands of fully-buried cells are skipped).

use bevy::prelude::*;

use super::map::{LevelMap, Tile};
use crate::loading::LoadingAssets;
use crate::state::InGame;

/// Spawn floor tiles for walkable cells, and a wall block for any wall cell that
/// borders the cavern.
pub(crate) fn spawn_tiles(
    map: &LevelMap,
    commands: &mut Commands,
    asset_server: &AssetServer,
    loading: &mut LoadingAssets,
) {
    let floor_scene = loading.track(
        asset_server.load(GltfAssetLabel::Scene(0).from_asset("Models/GLB format/floor.glb")),
    );
    let wall_scene = loading.track(
        asset_server.load(GltfAssetLabel::Scene(0).from_asset("Models/GLB format/wall.glb")),
    );

    for y in 0..map.height {
        for x in 0..map.width {
            let pos = map.tile_to_world(x, y);
            match map.get(x, y) {
                Tile::Floor => {
                    commands.spawn((
                        SceneRoot(floor_scene.clone()),
                        Transform::from_translation(pos),
                        DespawnOnExit(InGame),
                        Name::new(format!("Floor ({x},{y})")),
                    ));
                }
                Tile::Wall if borders_floor(map, x, y) => {
                    commands.spawn((
                        SceneRoot(wall_scene.clone()),
                        Transform::from_translation(pos),
                        DespawnOnExit(InGame),
                        Name::new(format!("Wall ({x},{y})")),
                    ));
                }
                Tile::Wall => {}
            }
        }
    }
}

/// True if any of the 8 surrounding cells is walkable floor.
fn borders_floor(map: &LevelMap, x: usize, y: usize) -> bool {
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
