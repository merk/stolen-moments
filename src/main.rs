// Bevy queries and spatial-search tuples are inherently shaped types; clippy's
// type-complexity lint fights the framework more than it helps here.
#![allow(clippy::type_complexity)]

mod adversary;
mod billboard;
mod camera;
mod catch;
mod coins;
mod debug;
mod employee;
mod level;
mod loading;
mod persistence;
mod player;
mod props;
mod seed;
mod state;
mod time_loop;
mod vault;
mod wasm_compat;

use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Stolen Moments".into(),
                        ..default()
                    }),
                    ..default()
                })
                .set(ImagePlugin::default_nearest()),
        )
        .insert_resource(ClearColor(Color::srgb(0.05, 0.05, 0.07)))
        .insert_resource(GlobalAmbientLight {
            color: Color::srgb(0.7, 0.75, 0.9),
            brightness: 220.0,
            ..default()
        })
        .add_plugins((
            state::StatePlugin,
            loading::LoadingPlugin,
            seed::SeedPlugin,
            persistence::PersistencePlugin,
            level::LevelPlugin,
            (camera::IsoCameraPlugin, billboard::BillboardPlugin),
            player::PlayerPlugin,
            props::PropsPlugin,
            coins::CoinsPlugin,
            time_loop::TimeLoopPlugin,
            adversary::AdversaryPlugin,
            catch::CatchPlugin,
            vault::VaultPlugin,
            employee::EmployeePlugin,
            debug::DebugPlugin,
        ))
        .add_systems(Startup, spawn_lighting)
        .run();
}

/// A warm key directional light casting shadows across the dungeon.
fn spawn_lighting(mut commands: Commands) {
    commands.spawn((
        DirectionalLight {
            illuminance: 9000.0,
            shadows_enabled: true,
            color: Color::srgb(1.0, 0.96, 0.88),
            ..default()
        },
        Transform::from_xyz(8.0, 14.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}
