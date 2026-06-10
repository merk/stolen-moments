//! Room placement and stamping: choose non-overlapping footprints for every
//! room kind, then write them into the grid. Open rooms overwrite floor only
//! and merge with the cavern; sealed rooms also force a protected wall ring with
//! a single deliberate doorway.

use bevy::prelude::warn;
use rand::Rng;
use rand::rngs::SmallRng;

use super::map::{LevelMap, MAP_HEIGHT, MAP_WIDTH, Room, RoomKind, Tile, TileRect};

/// Keep at least this many tiles of separation between rooms (so sealed rings
/// don't fuse and a doorway's approach tile clears its neighbours).
const ROOM_PAD: usize = 2;
/// Keep rooms this far off the map border.
const ROOM_MARGIN: usize = 2;

/// A chosen footprint awaiting stamping.
struct RoomSpec {
    kind: RoomKind,
    rect: TileRect,
}

/// The rooms every hybrid level contains, as `(kind, width, height)`. Critical
/// and sealed rooms come first so they win placement when space is tight; the
/// list guarantees exactly one Vault and one Security room.
const ROOM_PLAN: &[(RoomKind, usize, usize)] = &[
    (RoomKind::Vault, 6, 6),
    (RoomKind::Security, 5, 5),
    (RoomKind::Start, 5, 5),
    (RoomKind::Lobby, 9, 7),
    (RoomKind::GameTables, 9, 7),
    (RoomKind::Service, 6, 5),
];

/// Place and stamp every room in [`ROOM_PLAN`], returning their records.
pub(crate) fn stamp_rooms(
    map: &mut LevelMap,
    protected: &mut [bool],
    rng: &mut SmallRng,
) -> Vec<Room> {
    let specs = place_rooms(rng);
    let mut rooms = Vec::with_capacity(specs.len());
    for spec in &specs {
        rooms.push(stamp_room(map, protected, spec, rng));
    }
    rooms
}

/// Pick non-overlapping footprints for every room in [`ROOM_PLAN`].
fn place_rooms(rng: &mut SmallRng) -> Vec<RoomSpec> {
    let mut placed: Vec<RoomSpec> = Vec::with_capacity(ROOM_PLAN.len());
    for &(kind, w, h) in ROOM_PLAN {
        let rect = find_slot(rng, &placed, w, h).unwrap_or_else(|| {
            // No clear slot at all — extremely unlikely at this map size. Force
            // a corner placement so the level still has the required rooms.
            warn!("place_rooms: no free slot for {kind:?}; forcing placement");
            TileRect {
                min_x: ROOM_MARGIN,
                min_y: ROOM_MARGIN,
                max_x: ROOM_MARGIN + w - 1,
                max_y: ROOM_MARGIN + h - 1,
            }
        });
        placed.push(RoomSpec { kind, rect });
    }
    placed
}

/// Find a `w`×`h` footprint that clears all `placed` rooms: random tries first
/// (for variety), then a deterministic scan as a reliable fallback.
fn find_slot(rng: &mut SmallRng, placed: &[RoomSpec], w: usize, h: usize) -> Option<TileRect> {
    const TRIES: usize = 400;
    let max_x0 = MAP_WIDTH - ROOM_MARGIN - w;
    let max_y0 = MAP_HEIGHT - ROOM_MARGIN - h;

    let clear = |rect: &TileRect, placed: &[RoomSpec]| {
        placed
            .iter()
            .all(|p| !p.rect.intersects_padded(rect, ROOM_PAD))
    };

    for _ in 0..TRIES {
        let min_x = rng.gen_range(ROOM_MARGIN..=max_x0);
        let min_y = rng.gen_range(ROOM_MARGIN..=max_y0);
        let rect = TileRect {
            min_x,
            min_y,
            max_x: min_x + w - 1,
            max_y: min_y + h - 1,
        };
        if clear(&rect, placed) {
            return Some(rect);
        }
    }

    for min_y in ROOM_MARGIN..=max_y0 {
        for min_x in ROOM_MARGIN..=max_x0 {
            let rect = TileRect {
                min_x,
                min_y,
                max_x: min_x + w - 1,
                max_y: min_y + h - 1,
            };
            if clear(&rect, placed) {
                return Some(rect);
            }
        }
    }
    None
}

/// Stamp one room into the grid, returning its [`Room`] record. Open rooms write
/// floor only; sealed rooms also force a protected wall ring with one doorway.
fn stamp_room(
    map: &mut LevelMap,
    protected: &mut [bool],
    spec: &RoomSpec,
    rng: &mut SmallRng,
) -> Room {
    let rect = spec.rect;
    let mut tiles = Vec::new();
    let mut doorway = None;

    if spec.kind.sealed() {
        // Interior becomes floor (and the room's tagged tiles).
        for y in rect.min_y + 1..rect.max_y {
            for x in rect.min_x + 1..rect.max_x {
                map.set(x, y, Tile::Floor);
                tiles.push((x, y));
            }
        }
        // Perimeter becomes an immutable wall ring, clipping any cavern at the
        // border so the room truncates the caverns rather than merging.
        for (x, y) in rect.perimeter() {
            map.set(x, y, Tile::Wall);
            protected[y * MAP_WIDTH + x] = true;
        }
        // Carve a single doorway plus the approach tile just outside it, so the
        // sealed room has exactly one floor exit for the carver to connect to.
        let (dx, dy, ax, ay) = pick_doorway(&rect, rng);
        map.set(dx, dy, Tile::Floor);
        protected[dy * MAP_WIDTH + dx] = false;
        map.set(ax, ay, Tile::Floor);
        doorway = Some((dx, dy));
    } else {
        for (x, y) in rect.all_tiles() {
            map.set(x, y, Tile::Floor);
            tiles.push((x, y));
        }
    }

    Room {
        kind: spec.kind,
        rect,
        tiles,
        doorway,
    }
}

/// Choose a doorway on a random side of a sealed room (never a corner) and the
/// approach tile immediately outside it. `ROOM_MARGIN >= 2` guarantees the
/// approach stays inside the map border.
fn pick_doorway(rect: &TileRect, rng: &mut SmallRng) -> (usize, usize, usize, usize) {
    match rng.gen_range(0..4) {
        0 => {
            let x = rng.gen_range(rect.min_x + 1..rect.max_x);
            (x, rect.min_y, x, rect.min_y - 1)
        }
        1 => {
            let x = rng.gen_range(rect.min_x + 1..rect.max_x);
            (x, rect.max_y, x, rect.max_y + 1)
        }
        2 => {
            let y = rng.gen_range(rect.min_y + 1..rect.max_y);
            (rect.min_x, y, rect.min_x - 1, y)
        }
        _ => {
            let y = rng.gen_range(rect.min_y + 1..rect.max_y);
            (rect.max_x, y, rect.max_x + 1, y)
        }
    }
}
