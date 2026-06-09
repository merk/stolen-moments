//! The `Loading` state: hold play back until every GLB scene the world build
//! kicked off has finished streaming in (with its texture dependencies), so the
//! first played frame is fully textured rather than a flash of white/untextured
//! meshes — which matters most on the slower web build.
//!
//! World-build systems register the scene handles they spawn into
//! [`LoadingAssets`] during `OnEnter(Loading)`; [`poll_loading`] then watches
//! those handles each frame, updates a progress readout, and advances to
//! `Playing` once they're all settled (loaded or failed).

use bevy::prelude::*;

use crate::state::{self, GameState, WorldGen};

/// Asset handles whose load must complete before play begins. Cleared at the
/// start of every `Loading` entry, then filled by the world-build systems
/// (dungeon, player, props, adversary) via [`LoadingAssets::track`].
#[derive(Resource, Default)]
pub struct LoadingAssets(Vec<UntypedHandle>);

impl LoadingAssets {
    /// Register `handle` so the loading gate waits for it, returning the handle
    /// so callers can wrap a load inline:
    /// `let h = loading.track(asset_server.load(path));`.
    pub fn track<A: Asset>(&mut self, handle: Handle<A>) -> Handle<A> {
        self.0.push(handle.clone().untyped());
        handle
    }
}

/// Marks the progress text on the loading screen so [`poll_loading`] can update it.
#[derive(Component)]
struct LoadingProgress;

pub struct LoadingPlugin;

impl Plugin for LoadingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LoadingAssets>()
            // Clear last loop's handles before the world build registers new
            // ones (Terrain is the earliest world-build set).
            .add_systems(
                OnEnter(GameState::Loading),
                reset_tracked.before(WorldGen::Terrain),
            )
            .add_systems(OnEnter(GameState::Loading), spawn_loading_screen)
            .add_systems(Update, poll_loading.run_if(in_state(GameState::Loading)));
    }
}

fn reset_tracked(mut loading: ResMut<LoadingAssets>) {
    loading.0.clear();
}

/// Advance to `Playing` once every tracked asset is settled; until then, show
/// how many are ready. A failed load counts as settled (with a warning) so a
/// missing asset can't wedge the game on the loading screen forever.
fn poll_loading(
    asset_server: Res<AssetServer>,
    loading: Res<LoadingAssets>,
    mut next: ResMut<NextState<GameState>>,
    mut progress: Query<&mut Text, With<LoadingProgress>>,
) {
    let total = loading.0.len();
    let mut ready = 0;
    let mut settled = 0;
    for handle in &loading.0 {
        match asset_server.get_load_states(handle.id()) {
            Some((_, _, rec)) if rec.is_loaded() => {
                ready += 1;
                settled += 1;
            }
            Some((load, _, rec)) if load.is_failed() || rec.is_failed() => {
                warn!("Asset failed to load: {:?}", handle.path());
                settled += 1;
            }
            _ => {}
        }
    }

    if let Ok(mut text) = progress.single_mut() {
        text.0 = format!("{ready} / {total}");
    }

    // `total == 0` means nothing registered (e.g. a future headless level) —
    // don't get stuck; advance straight away.
    if settled == total {
        next.set(GameState::Playing);
    }
}

fn spawn_loading_screen(mut commands: Commands) {
    let root = state::menu_overlay(&mut commands, GameState::Loading);
    commands.entity(root).with_children(|p| {
        state::spawn_title(p, "Loading…");
        p.spawn((
            Text::new("0 / 0"),
            TextFont {
                font_size: 22.0,
                ..default()
            },
            TextColor(Color::srgb(0.7, 0.7, 0.75)),
            LoadingProgress,
        ));
    });
}
