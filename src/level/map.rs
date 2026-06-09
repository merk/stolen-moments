//! The runtime world model: the tile grid ([`LevelMap`]), its room tags, and
//! tile↔world helpers. This is the vocabulary every gameplay system queries;
//! generation (sibling modules) produces it, rendering turns it into meshes.

use bevy::prelude::*;

/// Grid dimensions (in tiles).
pub const MAP_WIDTH: usize = 48;
pub const MAP_HEIGHT: usize = 48;

/// World-space size of a single tile. Kenney mini-dungeon models are 1 unit.
pub const TILE_SIZE: f32 = 1.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tile {
    Floor,
    Wall,
}

/// Semantic room categories stamped into the grid by the hybrid level source.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RoomKind {
    Start,
    Lobby,
    GameTables,
    Vault,
    Security,
    Service,
}

impl RoomKind {
    /// Sealed rooms get a forced wall ring with deliberate doorways; open rooms
    /// merge their floor with the surrounding cavern.
    pub fn sealed(self) -> bool {
        matches!(self, RoomKind::Vault | RoomKind::Security)
    }
}

/// Index into [`LevelMap::rooms`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RoomId(pub usize);

/// Inclusive tile bounds of a room's footprint.
#[derive(Clone, Copy, Debug)]
pub struct TileRect {
    pub min_x: usize,
    pub min_y: usize,
    pub max_x: usize,
    pub max_y: usize,
}

impl TileRect {
    pub fn center(&self) -> (usize, usize) {
        ((self.min_x + self.max_x) / 2, (self.min_y + self.max_y) / 2)
    }

    /// True if this rect, grown by `pad` on every side, overlaps `other`.
    pub fn intersects_padded(&self, other: &TileRect, pad: usize) -> bool {
        let ax0 = self.min_x.saturating_sub(pad);
        let ay0 = self.min_y.saturating_sub(pad);
        let ax1 = self.max_x + pad;
        let ay1 = self.max_y + pad;
        ax0 <= other.max_x && other.min_x <= ax1 && ay0 <= other.max_y && other.min_y <= ay1
    }

    /// Every tile within the rect (inclusive).
    pub fn all_tiles(&self) -> impl Iterator<Item = (usize, usize)> {
        let (min_x, max_x, min_y, max_y) = (self.min_x, self.max_x, self.min_y, self.max_y);
        (min_y..=max_y).flat_map(move |y| (min_x..=max_x).map(move |x| (x, y)))
    }

    /// The perimeter ring tiles of the rect.
    pub fn perimeter(&self) -> impl Iterator<Item = (usize, usize)> {
        let (min_x, max_x, min_y, max_y) = (self.min_x, self.max_x, self.min_y, self.max_y);
        self.all_tiles()
            .filter(move |&(x, y)| x == min_x || x == max_x || y == min_y || y == max_y)
    }
}

/// A semantic region stamped into the grid.
#[derive(Clone, Debug)]
pub struct Room {
    pub kind: RoomKind,
    pub rect: TileRect,
    /// Floor member tiles tagged into `room_of` (the interior; excludes the
    /// sealed wall ring and its doorway).
    pub tiles: Vec<(usize, usize)>,
}

/// The generated level grid, stored row-major: `tiles[y * width + x]`, with a
/// parallel `room_of` lookup tagging each tile with the room it belongs to.
#[derive(Resource)]
pub struct LevelMap {
    pub width: usize,
    pub height: usize,
    tiles: Vec<Tile>,
    room_of: Vec<Option<RoomId>>,
    rooms: Vec<Room>,
}

impl LevelMap {
    /// A fresh all-wall grid with no rooms — the starting point for generation.
    pub(crate) fn filled_with_walls(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            tiles: vec![Tile::Wall; width * height],
            room_of: vec![None; width * height],
            rooms: Vec::new(),
        }
    }

    pub fn get(&self, x: usize, y: usize) -> Tile {
        self.tiles[y * self.width + x]
    }

    pub(crate) fn set(&mut self, x: usize, y: usize, tile: Tile) {
        self.tiles[y * self.width + x] = tile;
    }

    /// Tag a tile as belonging to a room (used while building `room_of`).
    pub(crate) fn assign_room(&mut self, x: usize, y: usize, id: RoomId) {
        self.room_of[y * self.width + x] = Some(id);
    }

    /// Install the semantic room list once stamping is complete.
    pub(crate) fn set_rooms(&mut self, rooms: Vec<Room>) {
        self.rooms = rooms;
    }

    pub fn is_walkable(&self, x: usize, y: usize) -> bool {
        self.get(x, y) == Tile::Floor
    }

    /// Number of walkable floor tiles in the map.
    pub fn floor_count(&self) -> usize {
        self.tiles.iter().filter(|&&t| t == Tile::Floor).count()
    }

    /// Which room (if any) this tile belongs to.
    pub fn room_at(&self, x: usize, y: usize) -> Option<RoomId> {
        self.room_of[y * self.width + x]
    }

    /// The kind of room this tile belongs to, if any.
    pub fn room_kind_at(&self, x: usize, y: usize) -> Option<RoomKind> {
        self.room_at(x, y).map(|RoomId(i)| self.rooms[i].kind)
    }

    /// All semantic rooms in the level.
    pub fn rooms(&self) -> &[Room] {
        &self.rooms
    }

    /// Convert tile coordinates to the world-space center of that tile.
    /// The grid is centred on the world origin and laid out on the XZ plane.
    pub fn tile_to_world(&self, x: usize, y: usize) -> Vec3 {
        let wx = (x as f32 - self.width as f32 / 2.0) * TILE_SIZE;
        let wz = (y as f32 - self.height as f32 / 2.0) * TILE_SIZE;
        Vec3::new(wx, 0.0, wz)
    }

    /// Convert a world-space position back to tile coordinates, if in bounds.
    pub fn world_to_tile(&self, pos: Vec3) -> Option<(usize, usize)> {
        let fx = pos.x / TILE_SIZE + self.width as f32 / 2.0;
        let fz = pos.z / TILE_SIZE + self.height as f32 / 2.0;
        if fx < 0.0 || fz < 0.0 {
            return None;
        }
        let (x, y) = (fx.floor() as usize, fz.floor() as usize);
        if x < self.width && y < self.height {
            Some((x, y))
        } else {
            None
        }
    }

    /// True if the world position lands on a walkable tile.
    pub fn is_world_walkable(&self, pos: Vec3) -> bool {
        self.world_to_tile(pos)
            .map(|(x, y)| self.is_walkable(x, y))
            .unwrap_or(false)
    }
}

/// Where the player should start — the centre of the `Start` room.
#[derive(Resource, Clone, Copy)]
pub struct SpawnPoint {
    pub tile: (usize, usize),
    pub world: Vec3,
}
