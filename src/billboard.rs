//! Camera-facing world overlays: small textured **emote** sprites and floating
//! **progress bars** drawn above entities, so the player reads game state at a
//! glance from clear glyphs and bar lengths rather than subtle colour shifts.
//!
//! The iso camera's orientation never changes — only its position follows — so a
//! billboard just copies the camera's fixed rotation. An overlay parented to a
//! *turning* entity (a guard yawing to face its movement) counter-rotates by the
//! parent's rotation so it stays square to the screen.
//!
//! This module owns only the rendering primitives: the [`Billboard`] component
//! and the [`OverlayAssets`] (the emote atlas material, one quad mesh per glyph,
//! and the track/fill quads + materials shared by every bar). The gameplay
//! modules ([`crate::adversary`], [`crate::catch`]) spawn these as child overlays
//! and drive them from their own state, so this module stays free of game logic.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use crate::camera::IsoCamera;

/// The Kenney emotes atlas (pixel style 1) and its pixel dimensions, used to
/// turn each glyph's cell into UV coordinates.
const EMOTE_ATLAS: &str = "kenney_emotes-pack/Spritesheets/pixel_style1.png";
const ATLAS_W: f32 = 80.0;
const ATLAS_H: f32 = 96.0;
/// Side of one atlas cell in pixels.
const CELL: f32 = 16.0;

/// World size of an emote sprite quad (square).
const EMOTE_SIZE: f32 = 0.6;

/// World dimensions of a progress bar (the full track).
pub const BAR_WIDTH: f32 = 0.9;
pub const BAR_HEIGHT: f32 = 0.13;

/// One emote glyph. Each maps to a single 16×16 cell of the atlas (coordinates
/// taken from the pack's `pixel_style1.xml`).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Emote {
    /// `?` — a guard has noticed something and is growing suspicious.
    Question,
    /// `!` — a guard has locked on and is giving chase.
    Exclamation,
    /// Animated thinking dots (cycle these three while a guard searches).
    Dots1,
    Dots2,
    Dots3,
    /// A lightbulb — marks the vault code, the one idea worth stealing.
    Idea,
}

impl Emote {
    /// Top-left pixel of this glyph's cell in the atlas.
    fn cell(self) -> (f32, f32) {
        match self {
            Emote::Question => (0.0, 80.0),
            Emote::Exclamation => (32.0, 64.0),
            Emote::Dots1 => (48.0, 48.0),
            Emote::Dots2 => (48.0, 32.0),
            Emote::Dots3 => (48.0, 16.0),
            Emote::Idea => (16.0, 32.0),
        }
    }

    /// Every glyph, for building the mesh table up front.
    const ALL: [Emote; 6] = [
        Emote::Question,
        Emote::Exclamation,
        Emote::Dots1,
        Emote::Dots2,
        Emote::Dots3,
        Emote::Idea,
    ];
}

/// Shared meshes and materials for every world overlay, built once at startup.
///
/// Bars are assembled from two unit quads scaled by `Transform`: a centred
/// `bar_track` and a left-anchored `bar_fill` (its X scale is the fill fraction,
/// so it grows rightward from the bar's left edge). Emote sprites share one
/// atlas material and swap the per-glyph mesh to change icon.
#[derive(Resource)]
pub struct OverlayAssets {
    /// Unlit, alpha-blended material sampling the emote atlas.
    pub emote_material: Handle<StandardMaterial>,
    emote_meshes: HashMap<Emote, Handle<Mesh>>,
    /// Centred unit quad for a bar's background track.
    pub bar_track_mesh: Handle<Mesh>,
    /// Left-anchored unit quad for a bar's fill (X scale = fill fraction).
    pub bar_fill_mesh: Handle<Mesh>,
    /// Dark translucent backing for a bar.
    pub bar_track_material: Handle<StandardMaterial>,
    /// Amber fill — a rising warning (guard attention).
    pub bar_warn_material: Handle<StandardMaterial>,
    /// Red fill — imminent danger (the player's grab meter).
    pub bar_danger_material: Handle<StandardMaterial>,
}

impl OverlayAssets {
    /// The quad mesh for a given glyph.
    pub fn emote_mesh(&self, emote: Emote) -> Handle<Mesh> {
        self.emote_meshes[&emote].clone()
    }
}

/// Marks a world overlay that should always face the camera. A standalone
/// billboard takes the camera rotation directly; one parented to a rotating
/// entity cancels the parent's rotation first, so it stays screen-aligned.
#[derive(Component)]
pub struct Billboard;

pub struct BillboardPlugin;

impl Plugin for BillboardPlugin {
    fn build(&self, app: &mut App) {
        // In Update (not PostUpdate) so the local rotation written here is baked
        // into GlobalTransform by PostUpdate propagation this same frame; reading
        // the parent's GlobalTransform a frame stale is imperceptible.
        app.add_systems(Startup, build_overlay_assets)
            .add_systems(Update, face_camera);
    }
}

/// Build the shared overlay meshes/materials once the asset server is up.
fn build_overlay_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let atlas = asset_server.load(EMOTE_ATLAS);
    let emote_material = materials.add(StandardMaterial {
        base_color_texture: Some(atlas),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default()
    });

    let emote_meshes = Emote::ALL
        .into_iter()
        .map(|e| (e, meshes.add(emote_quad(e))))
        .collect();

    // Inserted as a resource so any module can spawn overlays from it.
    commands.insert_resource(OverlayAssets {
        emote_material,
        emote_meshes,
        bar_track_mesh: meshes.add(unit_quad(0.0)),
        bar_fill_mesh: meshes.add(unit_quad(0.5)),
        bar_track_material: materials.add(bar_material(Color::srgba(0.04, 0.04, 0.05, 0.75))),
        bar_warn_material: materials.add(bar_material(Color::srgb(1.0, 0.62, 0.08))),
        bar_danger_material: materials.add(bar_material(Color::srgb(1.0, 0.18, 0.12))),
    });
}

/// A flat, unlit material tinted `color`, blended so the dark track reads as a
/// translucent backing.
fn bar_material(color: Color) -> StandardMaterial {
    StandardMaterial {
        base_color: color,
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default()
    }
}

/// Orient every billboard to face the camera. Parented overlays cancel their
/// parent's rotation; standalone ones copy the camera rotation outright.
fn face_camera(
    camera: Query<&GlobalTransform, With<IsoCamera>>,
    parents: Query<&GlobalTransform, Without<Billboard>>,
    mut billboards: Query<(&mut Transform, Option<&ChildOf>), With<Billboard>>,
) {
    let Ok(cam) = camera.single() else {
        return;
    };
    let cam_rot = cam.rotation();
    for (mut transform, child_of) in &mut billboards {
        transform.rotation = match child_of.and_then(|c| parents.get(c.parent()).ok()) {
            Some(parent) => parent.rotation().inverse() * cam_rot,
            None => cam_rot,
        };
    }
}

/// Build the quad for one emote glyph: an [`EMOTE_SIZE`] square in the XY plane
/// (normal +Z), UV-mapped to that glyph's atlas cell.
fn emote_quad(emote: Emote) -> Mesh {
    let (cx, cy) = emote.cell();
    let (u0, u1) = (cx / ATLAS_W, (cx + CELL) / ATLAS_W);
    // v grows downward in texture space, so the cell's top edge is the smaller v.
    let (v_top, v_bottom) = (cy / ATLAS_H, (cy + CELL) / ATLAS_H);
    let h = EMOTE_SIZE * 0.5;
    quad(-h, h, -h, h, [u0, u1, v_top, v_bottom])
}

/// Build a 1×1 unit quad in the XY plane for a progress-bar segment. `left_x`
/// places its left edge: `0.0` centres it (the track), `0.5` left-anchors it so
/// scaling X grows the quad rightward from a fixed left edge (the fill).
fn unit_quad(left_x: f32) -> Mesh {
    quad(-left_x, 1.0 - left_x, -0.5, 0.5, [0.0, 1.0, 0.0, 1.0])
}

/// Two-triangle quad spanning `x0..x1`, `y0..y1` at z=0 (normal +Z, the side the
/// camera faces once billboarded), with the UV rect `[u0, u1, v_top, v_bottom]`.
fn quad(x0: f32, x1: f32, y0: f32, y1: f32, [u0, u1, v_top, v_bottom]: [f32; 4]) -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[x0, y0, 0.0], [x1, y0, 0.0], [x1, y1, 0.0], [x0, y1, 0.0]],
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 4])
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[u0, v_bottom], [u1, v_bottom], [u1, v_top], [u0, v_top]],
    )
    .with_inserted_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]))
}
