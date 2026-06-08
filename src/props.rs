//! Scatters decorative props (barrels, chests, coins, rocks, columns) across
//! the dungeon floor to give the caverns some life.

use bevy::prelude::*;
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};

use crate::coins::{Coin, CoinScore};
use crate::dungeon::{DungeonMap, SpawnPoint};

/// Chance that any given eligible floor tile receives a prop.
const PROP_DENSITY: f64 = 0.06;

/// Don't place props within this tile radius of the player's spawn.
const SPAWN_CLEARANCE: i32 = 2;

/// A prop model and how often it should appear (relative weight).
struct PropKind {
    asset: &'static str,
    weight: u32,
    /// Coins float and spin; everything else sits on the floor.
    coin: bool,
}

const PROPS: &[PropKind] = &[
    PropKind { asset: "coin.glb", weight: 5, coin: true },
    PropKind { asset: "barrel.glb", weight: 4, coin: false },
    PropKind { asset: "rocks.glb", weight: 4, coin: false },
    PropKind { asset: "stones.glb", weight: 3, coin: false },
    PropKind { asset: "chest.glb", weight: 2, coin: false },
    PropKind { asset: "column.glb", weight: 1, coin: false },
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
) {
    let mut rng = SmallRng::from_entropy();
    let total_weight: u32 = PROPS.iter().map(|p| p.weight).sum();

    // Preload each prop scene once and reuse the handle.
    let handles: Vec<Handle<Scene>> = PROPS
        .iter()
        .map(|p| {
            asset_server.load(
                GltfAssetLabel::Scene(0).from_asset(format!("Models/GLB format/{}", p.asset)),
            )
        })
        .collect();

    let (sx, sy) = (spawn.tile.0 as i32, spawn.tile.1 as i32);

    for y in 0..map.height {
        for x in 0..map.width {
            if !map.is_walkable(x, y) {
                continue;
            }
            // Keep the spawn area clear so the player isn't boxed in.
            if (x as i32 - sx).abs() <= SPAWN_CLEARANCE && (y as i32 - sy).abs() <= SPAWN_CLEARANCE {
                continue;
            }
            if !rng.gen_bool(PROP_DENSITY) {
                continue;
            }

            // Weighted pick of which prop to place.
            let mut roll = rng.gen_range(0..total_weight);
            let idx = PROPS
                .iter()
                .position(|p| {
                    if roll < p.weight {
                        true
                    } else {
                        roll -= p.weight;
                        false
                    }
                })
                .unwrap_or(0);
            let kind = &PROPS[idx];

            let base = map.tile_to_world(x, y);
            let yaw = rng.gen_range(0.0..std::f32::consts::TAU);

            let mut entity = commands.spawn((
                SceneRoot(handles[idx].clone()),
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
                entity.insert(Transform::from_translation(base).with_rotation(
                    Quat::from_rotation_y(yaw),
                ));
            }
        }
    }
}

fn spin_props(time: Res<Time>, mut spinners: Query<&mut Transform, With<Spin>>) {
    for mut transform in &mut spinners {
        transform.rotate_y(2.0 * time.delta_secs());
    }
}
