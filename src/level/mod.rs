//! The level: the world model, its procedural generation, and its rendering.
//!
//! Layered into submodules — [`map`] (the runtime [`LevelMap`] every gameplay
//! system queries), the generation pipeline ([`source`] orchestrating [`noise`],
//! [`rooms`], and [`connect`]), and [`render`] (tiles → meshes). [`LevelPlugin`]
//! builds a level via a [`LevelSource`] on entering `Loading`, inserts the
//! map/spawn resources, and spawns the tile meshes.

mod connect;
pub(crate) mod map;
mod noise;
mod render;
mod rooms;
mod source;

pub use map::{LevelMap, RoomKind, SpawnPoint, TILE_SIZE};

use bevy::prelude::*;

use crate::loading::LoadingAssets;
use crate::seed::RunSeed;
use crate::state::{GameState, WorldGen};
use render::spawn_tiles;
use source::{HybridSource, Level, LevelSource};

pub struct LevelPlugin;

impl Plugin for LevelPlugin {
    fn build(&self, app: &mut App) {
        // Built on entering Loading, before the systems that populate the map.
        app.add_systems(
            OnEnter(GameState::Loading),
            generate_level.in_set(WorldGen::Terrain),
        );
    }
}

/// Build a level from the configured source, store it as resources, and spawn
/// the tile meshes.
fn generate_level(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut loading: ResMut<LoadingAssets>,
    run_seed: Res<RunSeed>,
) {
    let seed = run_seed.derive("level");
    let Level { map, spawn } = HybridSource.build(seed);

    info!(
        "Generated level with seed {seed}: {} floor tiles, {} rooms",
        map.floor_count(),
        map.rooms().len()
    );

    spawn_tiles(&map, &mut commands, &asset_server, &mut loading);

    commands.insert_resource(spawn);
    commands.insert_resource(map);
}
