//! A guard's floating state overlays: an **emote** that names what it's doing
//! (`?` suspicious, animated dots while searching, `!` chasing) plus an
//! **attention bar** that fills as a sighting builds toward a chase. Both are
//! camera-facing billboards (see [`crate::billboard`]) parented to the guard, so
//! they track it and read clearly without leaning on the cone's colour.

use bevy::prelude::*;

use crate::billboard::{BAR_HEIGHT, BAR_WIDTH, Billboard, Emote, OverlayAssets};

use super::{AdversaryGizmos, Awareness, INTEREST_THRESHOLD, Mode};

/// Heights above a guard's origin for its bar and emote (world units).
const BAR_LIFT: f32 = 1.5;
const EMOTE_LIFT: f32 = 2.05;
/// Seconds each searching dot frame holds before cycling to the next.
const DOTS_PERIOD: f32 = 0.35;
/// Interest below this reads as "not actually eyeing anyone" — no `?`.
const SUSPICION_EPSILON: f32 = 0.03;

/// Links a guard to its spawned overlay child entities so one system can drive
/// them: the emote sprite, the attention-bar root (toggled whole), and the bar's
/// fill quad (X-scaled to the fraction).
#[derive(Component)]
pub(super) struct GuardOverlay {
    emote: Entity,
    bar: Entity,
    fill: Entity,
}

/// Spawn a guard's emote + attention-bar overlays as billboard children and
/// record them in a [`GuardOverlay`] on the guard.
pub(super) fn attach_overlays(commands: &mut Commands, guard: Entity, assets: &OverlayAssets) {
    let emote = commands
        .spawn((
            Billboard,
            Mesh3d(assets.emote_mesh(Emote::Question)),
            MeshMaterial3d(assets.emote_material.clone()),
            Transform::from_xyz(0.0, EMOTE_LIFT, 0.0),
            Visibility::Hidden,
            ChildOf(guard),
            Name::new("Guard emote"),
        ))
        .id();

    // The bar root is the billboard; its track + fill are children, so they
    // inherit its camera-facing rotation and a horizontal offset stays
    // screen-aligned rather than swinging with the guard's facing.
    let bar = commands
        .spawn((
            Billboard,
            Transform::from_xyz(0.0, BAR_LIFT, 0.0),
            Visibility::Hidden,
            ChildOf(guard),
            Name::new("Guard attention bar"),
        ))
        .id();
    commands.spawn((
        Mesh3d(assets.bar_track_mesh.clone()),
        MeshMaterial3d(assets.bar_track_material.clone()),
        Transform::from_scale(Vec3::new(BAR_WIDTH, BAR_HEIGHT, 1.0)),
        ChildOf(bar),
    ));
    let fill = commands
        .spawn((
            Mesh3d(assets.bar_fill_mesh.clone()),
            MeshMaterial3d(assets.bar_warn_material.clone()),
            // Left edge pinned to the track's left; nudged toward the camera so
            // it sits over the track. X scale is set each frame to the fraction.
            Transform {
                translation: Vec3::new(-BAR_WIDTH * 0.5, 0.0, 0.01),
                scale: Vec3::new(0.0, BAR_HEIGHT, 1.0),
                ..default()
            },
            ChildOf(bar),
        ))
        .id();

    commands
        .entity(guard)
        .insert(GuardOverlay { emote, bar, fill });
}

/// Drive every guard's overlays from its awareness: swap/show the emote, toggle
/// the bar, and scale its fill. Honours the `attention_meters` dev toggle.
pub(super) fn update_guard_overlays(
    cfg: Res<AdversaryGizmos>,
    time: Res<Time>,
    assets: Res<OverlayAssets>,
    guards: Query<(&Awareness, &GuardOverlay)>,
    mut overlays: Query<(&mut Visibility, &mut Transform, Option<&mut Mesh3d>)>,
) {
    let elapsed = time.elapsed_secs();
    for (awareness, ov) in &guards {
        // Emote: which glyph (if any), or hidden when the overlays are off.
        if let Ok((mut vis, _, mesh)) = overlays.get_mut(ov.emote) {
            match cfg.attention_meters.then(|| emote_for(awareness, elapsed)) {
                Some(Some(emote)) => {
                    if let Some(mut mesh) = mesh {
                        mesh.0 = assets.emote_mesh(emote);
                    }
                    *vis = Visibility::Visible;
                }
                _ => *vis = Visibility::Hidden,
            }
        }

        // Attention bar: shown while a sighting builds, hidden once chasing (the
        // `!` and red cone carry that) or when calm.
        let fraction = attention_fraction(awareness);
        let show_bar = cfg.attention_meters && awareness.mode != Mode::Chase && fraction > 0.0;
        if let Ok((mut vis, _, _)) = overlays.get_mut(ov.bar) {
            *vis = if show_bar {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };
        }
        if let Ok((_, mut transform, _)) = overlays.get_mut(ov.fill) {
            transform.scale.x = BAR_WIDTH * fraction;
        }
    }
}

/// The emote a guard shows for its current state, or `None` when calm.
fn emote_for(awareness: &Awareness, elapsed: f32) -> Option<Emote> {
    // The pre-chase "spotted you!" beat flashes the alert before the guard moves,
    // whichever mode the sighting tripped from.
    if awareness.is_alerting() {
        return Some(Emote::Exclamation);
    }
    match awareness.mode {
        Mode::Chase => Some(Emote::Exclamation),
        Mode::Search => Some(searching_dots(elapsed)),
        Mode::Patrol => (awareness.interest > SUSPICION_EPSILON).then_some(Emote::Question),
    }
}

/// Cycle the three thinking-dots frames on a fixed period for a searching guard.
fn searching_dots(elapsed: f32) -> Emote {
    match ((elapsed / DOTS_PERIOD) as i64).rem_euclid(3) {
        0 => Emote::Dots1,
        1 => Emote::Dots2,
        _ => Emote::Dots3,
    }
}

/// How full the attention bar reads: saturated while chasing, otherwise the
/// banked interest as a fraction of the chase threshold.
fn attention_fraction(awareness: &Awareness) -> f32 {
    match awareness.mode {
        Mode::Chase => 1.0,
        Mode::Patrol | Mode::Search => (awareness.interest / INTEREST_THRESHOLD).clamp(0.0, 1.0),
    }
}
