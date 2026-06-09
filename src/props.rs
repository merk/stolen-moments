//! Scatters decorative props (barrels, chests, coins, rocks, columns) across
//! the dungeon floor to give the caverns some life.

use bevy::prelude::*;
use rand::rngs::SmallRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};

use crate::coins::{Coin, CoinScore};
use crate::dungeon::{DungeonMap, SpawnPoint};
use crate::seed::RunSeed;

/// Don't place props within this tile radius of the player's spawn.
const SPAWN_CLEARANCE: i32 = 2;

/// A prop model and exactly how many of it to scatter across the dungeon.
struct PropKind {
    asset: &'static str,
    count: usize,
    /// Coins float and spin; everything else sits on the floor.
    coin: bool,
}

const PROPS: &[PropKind] = &[
    PropKind {
        asset: "coin.glb",
        count: 30,
        coin: true,
    },
    PropKind {
        asset: "barrel.glb",
        count: 16,
        coin: false,
    },
    PropKind {
        asset: "rocks.glb",
        count: 16,
        coin: false,
    },
    PropKind {
        asset: "stones.glb",
        count: 12,
        coin: false,
    },
    PropKind {
        asset: "chest.glb",
        count: 6,
        coin: false,
    },
    PropKind {
        asset: "column.glb",
        count: 4,
        coin: false,
    },
];

/// Marks a prop that should slowly spin in place (coins).
#[derive(Component)]
struct Spin;

pub struct PropsPlugin;

impl Plugin for PropsPlugin {
    fn build(&self, app: &mut App) {
        // PostStartup so the dungeon map and spawn point already exist.
        app.add_systems(PostStartup, scatter_props)
            .add_systems(Update, spin_props);
    }
}

fn scatter_props(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    map: Res<DungeonMap>,
    spawn: Res<SpawnPoint>,
    mut score: ResMut<CoinScore>,
    run_seed: Res<RunSeed>,
) {
    let mut rng = SmallRng::seed_from_u64(run_seed.derive("props"));

    // Preload each prop scene once and reuse the handle.
    let handles: Vec<Handle<Scene>> = PROPS
        .iter()
        .map(|p| {
            asset_server
                .load(GltfAssetLabel::Scene(0).from_asset(format!("Models/GLB format/{}", p.asset)))
        })
        .collect();

    // Collect every eligible floor tile, then shuffle so we can hand out unique
    // tiles to each prop type without ever stacking two props on one tile.
    let (sx, sy) = (spawn.tile.0 as i32, spawn.tile.1 as i32);
    let mut tiles: Vec<(usize, usize)> = Vec::new();
    for y in 0..map.height {
        for x in 0..map.width {
            if !map.is_walkable(x, y) {
                continue;
            }
            // Keep the spawn area clear so the player isn't boxed in.
            if (x as i32 - sx).abs() <= SPAWN_CLEARANCE && (y as i32 - sy).abs() <= SPAWN_CLEARANCE
            {
                continue;
            }
            tiles.push((x, y));
        }
    }
    tiles.shuffle(&mut rng);
    let mut available = tiles.into_iter();

    for (kind, handle) in PROPS.iter().zip(&handles) {
        for (placed, _) in (0..kind.count).enumerate() {
            let Some((x, y)) = available.next() else {
                warn!(
                    "Ran out of floor tiles placing props; {} got {placed}/{}",
                    kind.asset, kind.count
                );
                break;
            };

            let base = map.tile_to_world(x, y);
            let yaw = rng.gen_range(0.0..std::f32::consts::TAU);

            let mut entity = commands.spawn((
                SceneRoot(handle.clone()),
                Name::new(format!("{} ({x},{y})", kind.asset)),
            ));

            if kind.coin {
                entity.insert((
                    Transform::from_translation(base + Vec3::Y * 0.35)
                        .with_rotation(Quat::from_rotation_y(yaw)),
                    Spin,
                    Coin,
                ));
                score.total += 1;
            } else {
                entity.insert(
                    Transform::from_translation(base).with_rotation(Quat::from_rotation_y(yaw)),
                );
            }
        }
    }
}

fn spin_props(time: Res<Time>, mut spinners: Query<&mut Transform, With<Spin>>) {
    for mut transform in &mut spinners {
        transform.rotate_y(2.0 * time.delta_secs());
    }
}
