//! Time looping: record the player each loop, then restart the scene while the
//! previous loops replay as transparent "ghosts" that re-walk their old paths.
//!
//! Press **Shift+R** to close the current loop: the recording is banked, the
//! player returns to the spawn point, coins reset (see [`LoopReset`]), and one
//! ghost is (re)spawned per banked recording to play back on top of you.

use std::sync::Arc;

use bevy::prelude::*;
use bevy::scene::SceneInstanceReady;

use crate::level::SpawnPoint;
use crate::player::Player;
use crate::state::{GameState, InGame};

/// How transparent ghost characters are rendered (0 = invisible, 1 = solid).
const GHOST_ALPHA: f32 = 0.35;

/// Height above the floor at which trails are drawn, to avoid z-fighting.
const TRAIL_HEIGHT: f32 = 0.06;
/// How transparent the trail line is.
const TRAIL_ALPHA: f32 = 0.85;
/// Seconds of path behind the ghost over which the trail dims from full
/// brightness down to [`TRAIL_MIN_BRIGHTNESS`].
const TRAIL_FADE_SECONDS: f32 = 2.5;
/// Brightness floor the trail keeps once fully faded, so the whole route stays
/// visible for the entire trip rather than disappearing behind the ghost.
const TRAIL_MIN_BRIGHTNESS: f32 = 0.3;

/// A single recorded frame of the player's pose, timestamped from loop start.
#[derive(Clone, Copy)]
struct Sample {
    time: f32,
    translation: Vec3,
    rotation: Quat,
}

/// Drives recording and playback for the whole time-loop system.
#[derive(Resource, Default)]
struct LoopState {
    /// Seconds elapsed in the loop currently being played/recorded.
    elapsed: f32,
    /// Samples captured so far this loop.
    current: Vec<Sample>,
    /// Finished recordings from earlier loops, one ghost each.
    banked: Vec<Arc<Vec<Sample>>>,
    /// The character scene reused for every ghost.
    character: Handle<Scene>,
}

/// Marks a replaying ghost and carries the recording it follows plus the loop's
/// assigned colour tint. Public so collectors (coins) can treat ghosts as actors
/// in the world.
#[derive(Component)]
pub struct Ghost {
    recording: Arc<Vec<Sample>>,
    /// Distinct per-loop colour used for both the ghost mesh and its trail.
    color: Color,
    /// Which banked loop this ghost replays. Lower = older; the highest index is
    /// the most recently recorded loop. Used to rank prey for adversaries.
    loop_index: usize,
}

impl Ghost {
    /// Age rank of this ghost: 0 is the oldest banked loop, higher is newer.
    pub fn loop_index(&self) -> usize {
        self.loop_index
    }
}

/// Fired (globally) when a loop restarts, so other systems can reset their
/// per-loop state. Coins listen for this to reappear; see `coins.rs`.
#[derive(Event, Default)]
pub struct LoopReset;

/// Request to restart the current loop. A voluntary close (Shift+R, the debug
/// "force loop" key) banks the run as a ghost; a catch can also request a
/// restart that *discards* the run instead, by clearing `bank`.
#[derive(Message, Clone, Copy)]
pub struct CloseLoop {
    /// Bank the just-played run as a ghost before resetting. `false` throws the
    /// run away — used when the player is caught, so a failed run leaves no echo.
    pub bank: bool,
}

pub struct TimeLoopPlugin;

impl Plugin for TimeLoopPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LoopState>()
            .add_message::<CloseLoop>()
            .add_systems(Startup, load_ghost_scene)
            // A fresh level starts with no banked runs and a zeroed clock.
            .add_systems(OnEnter(GameState::Loading), reset_loop_state)
            .add_systems(
                Update,
                (
                    close_loop_on_shift_r,
                    start_new_loop,
                    tick_and_record,
                    playback_ghosts,
                    draw_ghost_trails,
                )
                    .chain()
                    .run_if(in_state(GameState::Playing)),
            );
    }
}

fn load_ghost_scene(mut state: ResMut<LoopState>, asset_server: Res<AssetServer>) {
    state.character = asset_server
        .load(GltfAssetLabel::Scene(0).from_asset("Models/GLB format/character-human.glb"));
}

/// Wipe banked recordings and the loop clock when a new level is built, so a
/// fresh game (or a return to the menu and back) starts with no ghosts. The
/// reused character handle is left intact.
fn reset_loop_state(mut state: ResMut<LoopState>) {
    state.elapsed = 0.0;
    state.current.clear();
    state.banked.clear();
}

/// Translate the Shift+R keybind into a [`CloseLoop`] request, so the keyboard
/// and tooling (the debug "force loop" key) both drive closure through one path.
fn close_loop_on_shift_r(keys: Res<ButtonInput<KeyCode>>, mut close: MessageWriter<CloseLoop>) {
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if shift && keys.just_pressed(KeyCode::KeyR) {
        close.write(CloseLoop { bank: true });
    }
}

/// On a [`CloseLoop`] request: optionally bank the current loop, reset the player
/// and timer, and respawn a ghost for every banked recording.
fn start_new_loop(
    mut close: MessageReader<CloseLoop>,
    mut commands: Commands,
    mut state: ResMut<LoopState>,
    ghosts: Query<Entity, With<Ghost>>,
    mut player: Query<&mut Transform, With<Player>>,
    spawn: Res<SpawnPoint>,
) {
    // Drain the queue so a buffered request can't re-fire on a later frame; the
    // last request this frame decides whether the run is banked or discarded.
    let Some(request) = close.read().last().copied() else {
        return;
    };

    // Bank the loop we just played, unless this restart discards it (a catch) or
    // there's nothing recorded yet (an empty first frame).
    if request.bank && !state.current.is_empty() {
        let recording = Arc::new(std::mem::take(&mut state.current));
        state.banked.push(recording);
    }
    state.current.clear();
    state.elapsed = 0.0;

    // Clear last loop's ghosts and respawn one per banked recording so each
    // starts its replay cleanly from t = 0.
    for entity in &ghosts {
        commands.entity(entity).despawn();
    }
    let scene = state.character.clone();
    for (index, recording) in state.banked.iter().enumerate() {
        let color = loop_color(index);
        spawn_ghost(
            &mut commands,
            scene.clone(),
            recording.clone(),
            color,
            index,
            spawn.world,
        );
    }

    // Send the player back to the start.
    if let Ok(mut transform) = player.single_mut() {
        transform.translation = spawn.world;
        transform.rotation = Quat::IDENTITY;
    }

    // Let coins (and anything else) reset their per-loop state.
    commands.trigger(LoopReset);

    info!(
        "New loop started — {} ghost(s) replaying",
        state.banked.len()
    );
}

/// Assign each loop a distinct colour by spacing hues with the golden angle, so
/// successive loops are easy to tell apart at a glance.
fn loop_color(index: usize) -> Color {
    let hue = (index as f32 * 137.508).rem_euclid(360.0);
    Color::hsl(hue, 0.85, 0.6)
}

fn spawn_ghost(
    commands: &mut Commands,
    scene: Handle<Scene>,
    recording: Arc<Vec<Sample>>,
    color: Color,
    loop_index: usize,
    start: Vec3,
) {
    commands
        .spawn((
            SceneRoot(scene),
            Transform::from_translation(start),
            Ghost {
                recording,
                color,
                loop_index,
            },
            DespawnOnExit(InGame),
            Name::new("Ghost"),
        ))
        // Once the scene's meshes exist, swap their materials for transparent
        // clones so the ghost reads as a translucent echo of the player.
        .observe(make_ghost_transparent);
}

/// Replace each mesh material under a freshly-spawned ghost with a translucent,
/// loop-tinted clone. We clone per-ghost so the live player and other ghosts keep
/// their original (shared) materials.
fn make_ghost_transparent(
    ready: On<SceneInstanceReady>,
    ghosts: Query<&Ghost>,
    children: Query<&Children>,
    mesh_materials: Query<&MeshMaterial3d<StandardMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let root = ready.event().entity;
    let Ok(ghost) = ghosts.get(root) else {
        return;
    };
    let tint = ghost.color;
    let glow = tint.to_linear();
    for descendant in children.iter_descendants::<Children>(root) {
        let Ok(handle) = mesh_materials.get(descendant) else {
            continue;
        };
        let Some(base) = materials.get(&handle.0) else {
            continue;
        };
        let mut ghost_material = base.clone();
        // Flat loop colour with a soft self-lit glow reads as a translucent,
        // clearly-identifiable echo of that loop.
        ghost_material.base_color = tint.with_alpha(GHOST_ALPHA);
        ghost_material.emissive =
            LinearRgba::new(glow.red * 0.4, glow.green * 0.4, glow.blue * 0.4, 1.0);
        ghost_material.alpha_mode = AlphaMode::Blend;
        let new_handle = materials.add(ghost_material);
        commands
            .entity(descendant)
            .insert(MeshMaterial3d(new_handle));
    }
}

/// Advance the loop clock and record the player's current pose.
fn tick_and_record(
    time: Res<Time>,
    mut state: ResMut<LoopState>,
    player: Query<&Transform, With<Player>>,
) {
    if let Ok(transform) = player.single() {
        let sample = Sample {
            time: state.elapsed,
            translation: transform.translation,
            rotation: transform.rotation,
        };
        state.current.push(sample);
    }
    state.elapsed += time.delta_secs();
}

/// Move every ghost to where the player was at the current loop time.
fn playback_ghosts(state: Res<LoopState>, mut ghosts: Query<(&Ghost, &mut Transform)>) {
    for (ghost, mut transform) in &mut ghosts {
        if let Some((translation, rotation)) = sample_at(&ghost.recording, state.elapsed) {
            transform.translation = translation;
            transform.rotation = rotation;
        }
    }
}

/// Draw a fading trail along the path each ghost has walked so far this loop.
///
/// The trail is brightest right at the ghost and dims with age over
/// [`TRAIL_FADE_SECONDS`], then holds at [`TRAIL_MIN_BRIGHTNESS`] so the entire
/// route stays faintly visible for the whole trip.
fn draw_ghost_trails(state: Res<LoopState>, ghosts: Query<&Ghost>, mut gizmos: Gizmos) {
    let head_time = state.elapsed;
    let lift = Vec3::Y * TRAIL_HEIGHT;

    for ghost in &ghosts {
        let samples = &ghost.recording;
        if samples.len() < 2 {
            continue;
        }

        for window in samples.windows(2) {
            let (a, b) = (&window[0], &window[1]);
            let start = a.translation + lift;

            if b.time <= head_time {
                // Whole segment is behind the ghost; dim it by the newer end's age.
                let color = trail_color(ghost.color, head_time - b.time);
                gizmos.line(start, b.translation + lift, color);
            } else {
                // The ghost is partway along this segment — draw up to its head and
                // stop; everything after is in the ghost's future.
                let span = (b.time - a.time).max(1e-6);
                let f = ((head_time - a.time) / span).clamp(0.0, 1.0);
                let head = a.translation.lerp(b.translation, f) + lift;
                gizmos.line(start, head, trail_color(ghost.color, 0.0));
                break;
            }
        }
    }
}

/// Tint `color` for a trail segment whose newer end is `age` seconds behind the
/// ghost: full brightness at the head, fading to [`TRAIL_MIN_BRIGHTNESS`].
fn trail_color(color: Color, age: f32) -> Color {
    let f = (age / TRAIL_FADE_SECONDS).clamp(0.0, 1.0);
    let brightness = 1.0 - f * (1.0 - TRAIL_MIN_BRIGHTNESS);
    let c = color.to_linear();
    LinearRgba::new(
        c.red * brightness,
        c.green * brightness,
        c.blue * brightness,
        TRAIL_ALPHA,
    )
    .into()
}

/// Interpolate a recording at time `t`, clamping to the ends. Returns `None`
/// only for an empty recording.
fn sample_at(samples: &[Sample], t: f32) -> Option<(Vec3, Quat)> {
    let first = samples.first()?;
    if t <= first.time {
        return Some((first.translation, first.rotation));
    }
    let last = samples.last()?;
    if t >= last.time {
        return Some((last.translation, last.rotation));
    }

    // `partition_point` gives the first index whose time is strictly past `t`,
    // so the bracketing samples are [i - 1, i].
    let i = samples.partition_point(|s| s.time <= t);
    let a = &samples[i - 1];
    let b = &samples[i];
    let span = (b.time - a.time).max(1e-6);
    let f = ((t - a.time) / span).clamp(0.0, 1.0);
    Some((
        a.translation.lerp(b.translation, f),
        a.rotation.slerp(b.rotation, f),
    ))
}
