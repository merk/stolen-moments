//! Top-level game flow: the [`GameState`] machine (boot → menu → loading →
//! play → pause), the menu/pause UI, and the world-build lifecycle the gameplay
//! plugins hang off.
//!
//! The level is generated on `OnEnter(GameState::Loading)` and lives for as long
//! as the computed [`InGame`] state is active (loading, playing, paused). World
//! entities tag themselves `DespawnOnExit(InGame)`, so leaving to the menu tears
//! the level down automatically and a fresh `Loading` rebuilds it cleanly.

use bevy::app::AppExit;
use bevy::prelude::*;

/// The top-level application state.
#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameState {
    /// One-frame bootstrap that immediately advances to the main menu. Gives
    /// the rest of the app a settled first frame before any UI appears.
    #[default]
    Boot,
    MainMenu,
    /// The level is built here and asset loads kick off. Held by `loading.rs`
    /// until every tracked GLB scene has finished loading (with dependencies),
    /// so the first played frame is fully textured.
    Loading,
    Playing,
    Paused,
    // Entered by the win/lose conditions added in a later phase (P3+); declared
    // now so the state machine is complete, but not yet reachable or wired to UI.
    #[allow(dead_code)]
    GameOver,
    #[allow(dead_code)]
    Win,
}

/// Active whenever a level is loaded — spanning loading, play, and pause. World
/// entities marked `DespawnOnExit(InGame)` live exactly this long, so returning
/// to the menu disposes of the level and re-entering builds a fresh one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InGame;

impl ComputedStates for InGame {
    type SourceStates = GameState;

    fn compute(source: GameState) -> Option<Self> {
        matches!(
            source,
            GameState::Loading | GameState::Playing | GameState::Paused
        )
        .then_some(Self)
    }
}

/// Ordering for the world build on `OnEnter(GameState::Loading)`: terrain first
/// (it creates `LevelMap`/`SpawnPoint`), then everything that populates it.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorldGen {
    Terrain,
    Populate,
}

pub struct StatePlugin;

impl Plugin for StatePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<GameState>()
            .add_computed_state::<InGame>()
            .configure_sets(
                OnEnter(GameState::Loading),
                (WorldGen::Terrain, WorldGen::Populate).chain(),
            )
            .add_systems(OnEnter(GameState::Boot), enter_main_menu)
            .add_systems(OnEnter(GameState::MainMenu), spawn_main_menu)
            .add_systems(OnEnter(GameState::Paused), spawn_pause_menu)
            .add_systems(
                Update,
                (
                    main_menu_keys.run_if(in_state(GameState::MainMenu)),
                    pause_on_esc.run_if(in_state(GameState::Playing)),
                    resume_on_esc.run_if(in_state(GameState::Paused)),
                    menu_actions,
                    menu_hover,
                ),
            );
    }
}

/// `Boot` exists only to give the app one settled frame; advance straight on.
fn enter_main_menu(mut next: ResMut<NextState<GameState>>) {
    next.set(GameState::MainMenu);
}

fn pause_on_esc(keys: Res<ButtonInput<KeyCode>>, mut next: ResMut<NextState<GameState>>) {
    if keys.just_pressed(KeyCode::Escape) {
        next.set(GameState::Paused);
    }
}

fn resume_on_esc(keys: Res<ButtonInput<KeyCode>>, mut next: ResMut<NextState<GameState>>) {
    if keys.just_pressed(KeyCode::Escape) {
        next.set(GameState::Playing);
    }
}

fn main_menu_keys(keys: Res<ButtonInput<KeyCode>>, mut next: ResMut<NextState<GameState>>) {
    if keys.any_just_pressed([KeyCode::Enter, KeyCode::Space]) {
        next.set(GameState::Loading);
    }
}

// --- Menu UI -------------------------------------------------------------

/// What a clicked menu button does.
#[derive(Component, Clone, Copy)]
enum MenuAction {
    StartGame,
    Resume,
    QuitToMenu,
    QuitApp,
}

const BUTTON_NORMAL: Color = Color::srgb(0.18, 0.18, 0.22);
const BUTTON_HOVERED: Color = Color::srgb(0.28, 0.28, 0.34);
const BUTTON_PRESSED: Color = Color::srgb(0.10, 0.10, 0.13);

fn spawn_main_menu(mut commands: Commands) {
    let root = menu_overlay(&mut commands, GameState::MainMenu);
    commands.entity(root).with_children(|p| {
        spawn_title(p, "STOLEN MOMENTS");
        spawn_hint(p, "WASD move · Shift+R closes the loop · Esc pauses");
        spawn_button(p, "Start", MenuAction::StartGame);
        // The web build has no process to exit, so only offer Quit natively.
        #[cfg(not(target_arch = "wasm32"))]
        spawn_button(p, "Quit", MenuAction::QuitApp);
    });
}

fn spawn_pause_menu(mut commands: Commands) {
    let root = menu_overlay(&mut commands, GameState::Paused);
    commands.entity(root).with_children(|p| {
        spawn_title(p, "PAUSED");
        spawn_button(p, "Resume", MenuAction::Resume);
        spawn_button(p, "Main Menu", MenuAction::QuitToMenu);
    });
}

/// Apply click results: drive state transitions or quit the app.
fn menu_actions(
    interactions: Query<(&Interaction, &MenuAction), Changed<Interaction>>,
    mut next: ResMut<NextState<GameState>>,
    mut exit: MessageWriter<AppExit>,
) {
    for (interaction, action) in &interactions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match action {
            MenuAction::StartGame => next.set(GameState::Loading),
            MenuAction::Resume => next.set(GameState::Playing),
            MenuAction::QuitToMenu => next.set(GameState::MainMenu),
            MenuAction::QuitApp => {
                exit.write(AppExit::Success);
            }
        }
    }
}

/// Tint buttons on hover/press for a bit of feedback.
fn menu_hover(
    mut buttons: Query<(&Interaction, &mut BackgroundColor), (Changed<Interaction>, With<Button>)>,
) {
    for (interaction, mut bg) in &mut buttons {
        bg.0 = match interaction {
            Interaction::Pressed => BUTTON_PRESSED,
            Interaction::Hovered => BUTTON_HOVERED,
            Interaction::None => BUTTON_NORMAL,
        };
    }
}

/// A full-screen, centred, dimmed overlay scoped to the given menu state.
pub(crate) fn menu_overlay(commands: &mut Commands, scope: GameState) -> Entity {
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: Val::Px(14.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            DespawnOnExit(scope),
            Name::new("Menu overlay"),
        ))
        .id()
}

pub(crate) fn spawn_title(parent: &mut ChildSpawnerCommands, text: &str) {
    parent.spawn((
        Text::new(text),
        TextFont {
            font_size: 56.0,
            ..default()
        },
        TextColor(Color::srgb(1.0, 0.85, 0.3)),
        Node {
            margin: UiRect::bottom(Val::Px(20.0)),
            ..default()
        },
    ));
}

fn spawn_hint(parent: &mut ChildSpawnerCommands, text: &str) {
    parent.spawn((
        Text::new(text),
        TextFont {
            font_size: 18.0,
            ..default()
        },
        TextColor(Color::srgb(0.7, 0.7, 0.75)),
        Node {
            margin: UiRect::bottom(Val::Px(10.0)),
            ..default()
        },
    ));
}

fn spawn_button(parent: &mut ChildSpawnerCommands, label: &str, action: MenuAction) {
    parent
        .spawn((
            Button,
            Node {
                width: Val::Px(220.0),
                height: Val::Px(54.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(BUTTON_NORMAL),
            action,
        ))
        .with_children(|b| {
            b.spawn((
                Text::new(label),
                TextFont {
                    font_size: 26.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });
}
