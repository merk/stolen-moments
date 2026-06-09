//! Scatters decorative props (barrels, chests, coins, rocks, columns) across
//! the dungeon floor to give the caverns some life.

use std::collections::HashMap;

use bevy::prelude::*;
use rand::rngs::SmallRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};

use crate::coins::{Coin, CoinScore};
use crate::level::{LevelMap, RoomKind, SpawnPoint};
use crate::loading::LoadingAssets;
use crate::seed::RunSeed;
use crate::state::{GameState, InGame, WorldGen};

/// Don't place props within this tile radius of the player's spawn.
const SPAWN_CLEARANCE: i32 = 2;

/// A prop model and exactly how many of it to scatter across the dungeon.
struct PropKind {
    asset: &'static str,
    count: usize,
    /// Coins float and spin; everything else sits on the floor.
    coin: bool,
    /// Which room kind to favour when placing this prop (chips on the game
    /// tables, loot in the vault), falling back to general floor on overflow.
    prefer: Option<RoomKind>,
}

const PROPS: &[PropKind] = &[
    PropKind {
        asset: "coin.glb",
        count: 30,
        coin: true,
        prefer: Some(RoomKind::GameTables),
    },
    PropKind {
        asset: "barrel.glb",
        count: 16,
        coin: false,
        prefer: None,
    },
    PropKind {
        asset: "rocks.glb",
        count: 16,
        coin: false,
        prefer: None,
    },
    PropKind {
        asset: "stones.glb",
        count: 12,
        coin: false,
        prefer: None,
    },
    PropKind {
        asset: "chest.glb",
        count: 6,
        coin: false,
        prefer: Some(RoomKind::Vault),
    },
    PropKind {
        asset: "column.glb",
        count: 4,
        coin: false,
        prefer: None,
    },
];

/// Marks a prop that should slowly spin in place (coins).
#[derive(Component)]
struct Spin;

pub struct PropsPlugin;

impl Plugin for PropsPlugin {
    fn build(&self, app: &mut App) {
        // Scattered during the world build, after the dungeon map exists.
        app.add_systems(
            OnEnter(GameState::Loading),
            scatter_props.in_set(WorldGen::Populate),
        )
        .add_systems(Update, spin_props.run_if(in_state(GameState::Playing)));
    }
}

fn scatter_props(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    map: Res<LevelMap>,
    spawn: Res<SpawnPoint>,
    mut score: ResMut<CoinScore>,
    mut loading: ResMut<LoadingAssets>,
    run_seed: Res<RunSeed>,
) {
    // A fresh level starts with an empty tally; coins re-add to `total` below.
    score.collected = 0;
    score.total = 0;

    let mut rng = SmallRng::seed_from_u64(run_seed.derive("props"));

    // Preload each prop scene once and reuse the handle.
    let handles: Vec<Handle<Scene>> = PROPS
        .iter()
        .map(|p| {
            loading.track(asset_server.load(
                GltfAssetLabel::Scene(0).from_asset(format!("Models/GLB format/{}", p.asset)),
            ))
        })
        .collect();

    // Tiles in a room a prop *prefers* are reserved into their own pool; the
    // rest become the general scatter. Each pool is shuffled and handed out via
    // a unique iterator, so two props never stack on one tile — props overflow
    // from their preferred pool into the general one when the room fills up.
    let reserved: Vec<RoomKind> = PROPS.iter().filter_map(|p| p.prefer).collect();
    let (sx, sy) = (spawn.tile.0 as i32, spawn.tile.1 as i32);
    let mut preferred: HashMap<RoomKind, Vec<(usize, usize)>> = HashMap::new();
    let mut general: Vec<(usize, usize)> = Vec::new();
    for y in 0..map.height {
        for x in 0..map.width {
            if !map.is_walkable(x, y) {
                continue;
            }
            if let Some(kind) = map.room_kind_at(x, y)
                && reserved.contains(&kind)
            {
                preferred.entry(kind).or_default().push((x, y));
                continue;
            }
            // Keep the spawn area clear so the player isn't boxed in.
            if (x as i32 - sx).abs() <= SPAWN_CLEARANCE && (y as i32 - sy).abs() <= SPAWN_CLEARANCE
            {
                continue;
            }
            general.push((x, y));
        }
    }
    general.shuffle(&mut rng);
    let mut pools: HashMap<RoomKind, std::vec::IntoIter<(usize, usize)>> = preferred
        .into_iter()
        .map(|(kind, mut tiles)| {
            tiles.shuffle(&mut rng);
            (kind, tiles.into_iter())
        })
        .collect();
    let mut available = general.into_iter();

    for (kind, handle) in PROPS.iter().zip(&handles) {
        for placed in 0..kind.count {
            let tile = kind
                .prefer
                .and_then(|k| pools.get_mut(&k).and_then(Iterator::next))
                .or_else(|| available.next());
            let Some((x, y)) = tile else {
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
                DespawnOnExit(InGame),
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
