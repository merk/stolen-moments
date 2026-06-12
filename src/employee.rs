//! The casino-floor employee and the vault-code pickup.
//!
//! The employee is a scripted, **non-reactive** actor (decision C in `PLAN.md`):
//! it paces a short fixed loop of waypoints inside the open Lobby — where
//! straight-line movement never clips a wall, so no pathfinder is needed — and
//! runs on `FixedUpdate` so its route is a pure function of the seed, identical
//! every loop. It exists as a landmark for the **code note**, a pickup dropped
//! on the floor beside it. Walking over the note learns [`Fact::VaultCodeKnown`]
//! (which persists across loops via [`Persistent`]), letting the player open the
//! vault on this or any later run.

use bevy::prelude::*;
use bevy::scene::SceneInstanceReady;
use rand::SeedableRng;
use rand::rngs::SmallRng;
use rand::seq::SliceRandom;

use crate::level::{LevelMap, RoomKind};
use crate::loading::LoadingAssets;
use crate::persistence::{Fact, Persistent};
use crate::player::Player;
use crate::seed::RunSeed;
use crate::state::{GameState, InGame, WorldGen};
use crate::time_loop::LoopReset;

/// Employee pacing speed (world units/sec).
const EMPLOYEE_SPEED: f32 = 1.8;
/// Distance at which the current waypoint counts as reached.
const WAYPOINT_RADIUS: f32 = 0.12;
/// How many waypoints the employee's fixed loop cycles through.
const ROUTE_WAYPOINTS: usize = 3;
/// Player within this distance (world units) of the note picks it up.
const NOTE_PICKUP_RADIUS: f32 = 0.7;
/// A teal "uniform" tint so the employee reads apart from the player and ghosts.
const UNIFORM: Color = Color::srgb(0.18, 0.52, 0.62);

/// A scripted employee pacing a fixed loop of world-space waypoints.
#[derive(Component)]
struct Employee {
    waypoints: Vec<Vec3>,
    /// Index of the waypoint currently being walked toward.
    index: usize,
}

/// The vault-code pickup lying on the floor near the employee.
#[derive(Component)]
struct CodeNote;

/// Which tile the code note sits on, published for other systems (guards) to
/// anchor on — a guard posts nearby to watch the code. Present only when a Lobby
/// (and thus a note) exists; absent for roomless sources.
#[derive(Resource, Clone, Copy)]
pub struct CodeNoteSite {
    pub tile: (usize, usize),
}

pub struct EmployeePlugin;

impl Plugin for EmployeePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(GameState::Loading),
            spawn_employee.in_set(WorldGen::Objectives),
        )
        .add_systems(
            FixedUpdate,
            walk_employee.run_if(in_state(GameState::Playing)),
        )
        .add_systems(Update, pickup_note.run_if(in_state(GameState::Playing)))
        .add_observer(reset_employee)
        .add_observer(reset_note);
    }
}

/// Spawn the employee on a seeded Lobby route plus the code note beside its
/// start. Without a Lobby (e.g. a roomless source) there's nowhere natural to
/// place them, so neither is spawned.
fn spawn_employee(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut loading: ResMut<LoadingAssets>,
    map: Res<LevelMap>,
    run_seed: Res<RunSeed>,
) {
    let Some(lobby) = map.rooms().iter().find(|r| r.kind == RoomKind::Lobby) else {
        return;
    };

    // Pick a handful of distinct Lobby tiles as the route. The Lobby is convex
    // and fully floored, so straight legs between them never cross a wall.
    let mut rng = SmallRng::seed_from_u64(run_seed.derive("employee"));
    let picks: Vec<(usize, usize)> = lobby
        .tiles
        .choose_multiple(&mut rng, ROUTE_WAYPOINTS.max(2))
        .copied()
        .collect();
    if picks.len() < 2 {
        return;
    }
    let waypoints: Vec<Vec3> = picks
        .iter()
        .map(|&(x, y)| map.tile_to_world(x, y))
        .collect();
    let start = waypoints[0];

    let character =
        loading
            .track(asset_server.load(
                GltfAssetLabel::Scene(0).from_asset("Models/GLB format/character-human.glb"),
            ));
    commands
        .spawn((
            SceneRoot(character),
            Transform::from_translation(start),
            Employee {
                waypoints,
                index: 1,
            },
            DespawnOnExit(InGame),
            Name::new("Employee"),
        ))
        .observe(tint_employee);

    // The code note sits on the floor just beside where the employee starts.
    let note = loading.track(
        asset_server.load(GltfAssetLabel::Scene(0).from_asset("Models/GLB format/chest.glb")),
    );
    let note_world = start + Vec3::new(0.8, 0.0, 0.0);
    commands.spawn((
        SceneRoot(note),
        Transform::from_translation(note_world),
        Visibility::Visible,
        CodeNote,
        DespawnOnExit(InGame),
        Name::new("Code note"),
    ));

    // Publish the note's site so a guard can post up and watch it.
    let tile = map.world_to_tile(note_world).unwrap_or(picks[0]);
    commands.insert_resource(CodeNoteSite { tile });
}

/// Recolour the employee's meshes with the flat uniform tint, so it's clearly
/// neither the player nor a ghost. Mirrors `time_loop`'s ghost-tinting pass.
fn tint_employee(
    ready: On<SceneInstanceReady>,
    employees: Query<(), With<Employee>>,
    children: Query<&Children>,
    mesh_materials: Query<&MeshMaterial3d<StandardMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let root = ready.event().entity;
    if employees.get(root).is_err() {
        return;
    }
    let glow = UNIFORM.to_linear();
    for descendant in children.iter_descendants::<Children>(root) {
        let Ok(handle) = mesh_materials.get(descendant) else {
            continue;
        };
        let Some(base) = materials.get(&handle.0) else {
            continue;
        };
        let mut tinted = base.clone();
        tinted.base_color = UNIFORM;
        tinted.emissive =
            LinearRgba::new(glow.red * 0.15, glow.green * 0.15, glow.blue * 0.15, 1.0);
        let new_handle = materials.add(tinted);
        commands
            .entity(descendant)
            .insert(MeshMaterial3d(new_handle));
    }
}

/// Walk each employee toward its current waypoint, advancing (and looping) the
/// route on arrival. Deterministic on `FixedUpdate`.
fn walk_employee(time: Res<Time>, mut employees: Query<(&mut Transform, &mut Employee)>) {
    let step = EMPLOYEE_SPEED * time.delta_secs();
    for (mut transform, mut employee) in &mut employees {
        let target = employee.waypoints[employee.index];
        let to = target - transform.translation;
        let dist = to.length();
        if dist <= WAYPOINT_RADIUS {
            let len = employee.waypoints.len();
            employee.index = (employee.index + 1) % len;
            continue;
        }
        let dir = to / dist;
        transform.translation += dir * step.min(dist);
        // Face the direction of travel (model front is +Z, looking_to aligns -Z).
        transform.rotation = Transform::IDENTITY.looking_to(-dir, Vec3::Y).rotation;
    }
}

/// On a loop restart, send each employee back to the start of its route so the
/// new run replays identically.
fn reset_employee(_reset: On<LoopReset>, mut employees: Query<(&mut Transform, &mut Employee)>) {
    for (mut transform, mut employee) in &mut employees {
        transform.translation = employee.waypoints[0];
        employee.index = 1 % employee.waypoints.len();
    }
}

/// Pick up the code note when the player walks over it: learn the (persistent)
/// vault code and hide the note for the rest of the session.
fn pickup_note(
    mut persistent: ResMut<Persistent>,
    player: Query<&Transform, With<Player>>,
    mut notes: Query<(&Transform, &mut Visibility), With<CodeNote>>,
) {
    let Ok(player) = player.single() else {
        return;
    };
    for (transform, mut visibility) in &mut notes {
        if *visibility == Visibility::Hidden {
            continue;
        }
        let (dx, dz) = (
            transform.translation.x - player.translation.x,
            transform.translation.z - player.translation.z,
        );
        if dx * dx + dz * dz <= NOTE_PICKUP_RADIUS * NOTE_PICKUP_RADIUS
            && persistent.learn(Fact::VaultCodeKnown)
        {
            *visibility = Visibility::Hidden;
            info!("Vault code learned!");
        }
    }
}

/// On a loop restart, restore the note only while the code is still unknown.
/// Once learned the fact persists, so the note stays gone across loops.
fn reset_note(
    _reset: On<LoopReset>,
    persistent: Res<Persistent>,
    mut notes: Query<&mut Visibility, With<CodeNote>>,
) {
    let known = persistent.knows(Fact::VaultCodeKnown);
    for mut visibility in &mut notes {
        *visibility = if known {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
    }
}
