//! Collectible coins: marker component, score tracking, pickup, and HUD.

use bevy::prelude::*;

use crate::player::Player;
use crate::state::{GameState, InGame};
use crate::time_loop::{Ghost, LoopReset};

/// Marks a coin prop the player can pick up.
#[derive(Component)]
pub struct Coin;

/// Tags a coin already taken this loop, so it's skipped until the loop resets.
#[derive(Component)]
struct Collected;

/// Running tally of coins collected versus how many were placed.
#[derive(Resource, Default)]
pub struct CoinScore {
    pub collected: u32,
    pub total: u32,
}

/// Player within this distance (world units) of a coin picks it up.
const PICKUP_RADIUS: f32 = 0.6;

/// Marks the HUD text node that displays the coin tally.
#[derive(Component)]
struct CoinCounterText;

pub struct CoinsPlugin;

impl Plugin for CoinsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CoinScore>()
            .add_systems(OnEnter(GameState::Loading), spawn_hud)
            .add_systems(
                Update,
                (collect_coins, update_hud).run_if(in_state(GameState::Playing)),
            )
            .add_observer(reset_coins);
    }
}

/// Hide any coin a collector (the player or a replaying ghost) is standing on
/// and bump the collected count. Coins are hidden rather than despawned so the
/// next loop can bring them back instantly.
fn collect_coins(
    mut commands: Commands,
    mut score: ResMut<CoinScore>,
    players: Query<&Transform, With<Player>>,
    ghosts: Query<&Transform, With<Ghost>>,
    mut coins: Query<(Entity, &Transform, &mut Visibility), (With<Coin>, Without<Collected>)>,
) {
    // Ghosts re-walk their recorded paths, so they pick up coins exactly as
    // they did when those frames were the live player.
    let collectors: Vec<Vec3> = players
        .iter()
        .chain(ghosts.iter())
        .map(|t| t.translation)
        .collect();
    if collectors.is_empty() {
        return;
    }

    for (entity, transform, mut visibility) in &mut coins {
        // Coins float above the floor, so compare on the ground (XZ) plane only.
        let taken = collectors.iter().any(|pos| {
            let dx = transform.translation.x - pos.x;
            let dz = transform.translation.z - pos.z;
            dx * dx + dz * dz <= PICKUP_RADIUS * PICKUP_RADIUS
        });
        if taken {
            *visibility = Visibility::Hidden;
            commands.entity(entity).insert(Collected);
            score.collected += 1;
        }
    }
}

/// On a loop restart, bring every coin back and zero the collected tally.
fn reset_coins(
    _reset: On<LoopReset>,
    mut commands: Commands,
    mut score: ResMut<CoinScore>,
    mut coins: Query<(Entity, &mut Visibility), With<Coin>>,
) {
    for (entity, mut visibility) in &mut coins {
        *visibility = Visibility::Inherited;
        commands.entity(entity).remove::<Collected>();
    }
    score.collected = 0;
}

fn spawn_hud(mut commands: Commands) {
    commands.spawn((
        Text::new("Coins: 0"),
        TextFont {
            font_size: 28.0,
            ..default()
        },
        TextColor(Color::srgb(1.0, 0.85, 0.3)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(12.0),
            left: Val::Px(14.0),
            ..default()
        },
        DespawnOnExit(InGame),
        CoinCounterText,
    ));
}

/// Refresh the HUD text whenever the score (collected or total) changes.
fn update_hud(score: Res<CoinScore>, mut text: Query<&mut Text, With<CoinCounterText>>) {
    if !score.is_changed() {
        return;
    }
    let Ok(mut text) = text.single_mut() else {
        return;
    };
    text.0 = format!("Coins: {} / {}", score.collected, score.total);
}
