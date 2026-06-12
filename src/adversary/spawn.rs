//! Guard placement: drop one of each [`GuardKind`] into the level, anchored to
//! the things worth guarding. The patroller paces the **vault door's** frontage
//! so someone is always walking past the front door; the static guard posts up a
//! few tiles from the **code note** and watches it; the wanderer roams a safe
//! distance from the player's spawn. Each objective-anchored placement degrades
//! to a sensible fallback when its landmark is absent (a roomless source, or a
//! source without a sealed vault): the static guard falls back to the Security
//! room, the patroller to a seeded local beat. All choices draw from seeded RNG
//! streams (or deterministic map geometry) so the roster is identical across runs
//! of the same seed.

use bevy::prelude::*;
use rand::rngs::SmallRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};

use crate::billboard::OverlayAssets;
use crate::employee::CodeNoteSite;
use crate::level::{LevelMap, RoomKind, SpawnPoint};
use crate::loading::LoadingAssets;
use crate::seed::RunSeed;
use crate::state::InGame;

use super::overlay::attach_overlays;
use super::path::random_walkable;
use super::vision::horizontal;
use super::{
    Adversary, Awareness, GUARD_KINDS, GuardKind, Mode, Navigation, PATROL_RADIUS,
    PATROL_WAYPOINTS, PatrolRoute, Post, SPAWN_CLEARANCE, VAULT_PATROL_SPAN, Vision,
    WATCH_DISTANCE, Wander,
};

// A spawn system reads a fair few resources to place guards against the level's
// objectives; the params are all distinct ECS fetches, not a refactor smell.
#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_adversaries(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut loading: ResMut<LoadingAssets>,
    overlays: Res<OverlayAssets>,
    map: Res<LevelMap>,
    spawn: Res<SpawnPoint>,
    code: Option<Res<CodeNoteSite>>,
    run_seed: Res<RunSeed>,
) {
    let scene = loading.track(
        asset_server
            .load(GltfAssetLabel::Scene(0).from_asset("Models/GLB format/character-orc.glb")),
    );

    // One RNG seeds placement/initial facing; each guard then carries its own
    // seeded stream so they wander/sweep independently yet reproducibly.
    let mut rng = SmallRng::seed_from_u64(run_seed.derive("adversary.spawn"));
    let (sx, sy) = (spawn.tile.0 as i32, spawn.tile.1 as i32);

    // The vault patroller's beat, derived once from the door's geometry. `None`
    // when the source has no sealed vault — the patroller then falls back below.
    let beat = vault_beat(&map);

    // Static-guard fallback (no code note): post in the Security room, a shuffled
    // pool handing out distinct posts; failing that, a far tile like the wanderer.
    let mut security: Vec<(usize, usize)> = map
        .rooms()
        .iter()
        .filter(|r| r.kind == RoomKind::Security)
        .flat_map(|r| r.tiles.iter().copied())
        .collect();
    security.shuffle(&mut rng);
    let mut security = security.into_iter();

    for (i, &kind) in GUARD_KINDS.iter().enumerate() {
        // Place each guard against the thing it guards: the static guard watches
        // the code note, the patroller posts at the vault door, the wanderer just
        // starts somewhere clear of the player. Each falls back when its landmark
        // is missing.
        let (tile, heading) = match kind {
            GuardKind::Static => match &code {
                Some(site) => watch_post(&map, site.tile),
                None => {
                    let tile = security
                        .next()
                        .unwrap_or_else(|| far_tile(&map, &mut rng, sx, sy));
                    (tile, interesting_facing(&map, tile, spawn.tile))
                }
            },
            GuardKind::Patrolling => match &beat {
                // `(post, heading)` are Copy; the route is cloned below.
                Some(beat) => (beat.0, beat.1),
                None => (far_tile(&map, &mut rng, sx, sy), random_heading(&mut rng)),
            },
            GuardKind::Wandering => (far_tile(&map, &mut rng, sx, sy), random_heading(&mut rng)),
        };

        let world = map.tile_to_world(tile.0, tile.1);
        let sweep_phase = rng.gen_range(0.0..std::f32::consts::TAU);

        let mut entity = commands.spawn((
            Adversary,
            SceneRoot(scene.clone()),
            Transform::from_translation(world)
                .with_scale(Vec3::splat(kind.scale()))
                .looking_to(-heading, Vec3::Y),
            Vision {
                heading,
                look_dir: heading,
                look_vel: 0.0,
                sweep_phase,
            },
            Awareness {
                mode: Mode::Patrol,
                interest: 0.0,
                last_seen: world,
                search_timer: 0.0,
                search_points: Vec::new(),
                search_step: 0,
                alert_timer: 0.0,
            },
            Navigation::default(),
            Post {
                home: world,
                heading,
                sweep_phase,
            },
            DespawnOnExit(InGame),
            Name::new(format!("Adversary {i} ({})", kind.label())),
        ));
        let guard = entity.id();

        // Kind-specific state: only patrolling guards carry a route, only
        // wandering guards carry an RNG. Static guards need neither.
        match kind {
            GuardKind::Patrolling => {
                // The vault beat when there's a vault to work; otherwise a seeded
                // local loop off its own stream, so the fallback is stable across
                // runs and doesn't perturb the placement stream.
                let waypoints = match &beat {
                    Some((_, _, route)) => route.clone(),
                    None => {
                        let mut route_rng =
                            SmallRng::seed_from_u64(run_seed.derive_indexed("adversary.patrol", i));
                        patrol_route(&map, &mut route_rng, tile)
                    }
                };
                entity.insert(PatrolRoute {
                    waypoints,
                    index: 0,
                });
            }
            GuardKind::Wandering => {
                entity.insert(Wander(SmallRng::seed_from_u64(
                    run_seed.derive_indexed("adversary", i),
                )));
            }
            GuardKind::Static => {}
        }

        // `entity`'s borrow of `commands` has ended (last used above), so it's
        // free again to spawn the guard's overlay children.
        attach_overlays(&mut commands, guard, &overlays);
    }
}

/// Build a fixed patrol loop: the spawn tile plus a handful of random reachable
/// tiles within [`PATROL_RADIUS`] of it, ordered nearest-neighbour from the start
/// so the guard walks a coherent local beat rather than crisscrossing the map.
/// The connected map guarantees BFS links each leg at runtime.
fn patrol_route(map: &LevelMap, rng: &mut SmallRng, start: (usize, usize)) -> Vec<(usize, usize)> {
    let mut pool: Vec<(usize, usize)> = Vec::with_capacity(PATROL_WAYPOINTS - 1);
    while pool.len() < PATROL_WAYPOINTS - 1 {
        pool.push(near_tile(map, rng, start));
    }

    // Greedily chain the nearest unvisited tile from where we are, starting at
    // `start`, so consecutive legs stay short and the loop doesn't zigzag.
    let mut route = Vec::with_capacity(PATROL_WAYPOINTS);
    route.push(start);
    let mut cur = start;
    while !pool.is_empty() {
        let (i, &next) = pool
            .iter()
            .enumerate()
            .min_by_key(|&(_, &t)| chebyshev(cur, t))
            .expect("pool is non-empty");
        route.push(next);
        cur = next;
        pool.swap_remove(i);
    }
    route
}

/// Pick a random walkable tile within [`PATROL_RADIUS`] (Chebyshev) of `origin`.
/// The origin tile itself is walkable, so this always terminates.
fn near_tile(map: &LevelMap, rng: &mut SmallRng, origin: (usize, usize)) -> (usize, usize) {
    loop {
        let t = random_walkable(map, rng);
        if chebyshev(origin, t) <= PATROL_RADIUS {
            return t;
        }
    }
}

/// Chebyshev (king-move) tile distance.
fn chebyshev(a: (usize, usize), b: (usize, usize)) -> i32 {
    let dx = (a.0 as i32 - b.0 as i32).abs();
    let dy = (a.1 as i32 - b.1 as i32).abs();
    dx.max(dy)
}

/// A facing for a posted static guard: toward the doorway of the room it stands
/// in, else the nearest doorway anywhere, else the player's spawn — so it always
/// watches a meaningful approach rather than a random wall. Falls back to a fixed
/// heading only on a doorless, spawnless map.
fn interesting_facing(map: &LevelMap, tile: (usize, usize), spawn: (usize, usize)) -> Vec3 {
    let here = map.tile_to_world(tile.0, tile.1);
    let toward = |target: (usize, usize)| {
        let dir = horizontal(map.tile_to_world(target.0, target.1) - here);
        dir.try_normalize()
    };

    // The doorway of the room this guard stands in, if any.
    let own_door = map
        .rooms()
        .iter()
        .find(|r| r.doorway.is_some() && r.tiles.contains(&tile))
        .and_then(|r| r.doorway);
    // Otherwise the closest doorway on the whole map.
    let nearest_door = map
        .rooms()
        .iter()
        .filter_map(|r| r.doorway)
        .min_by_key(|&d| chebyshev(tile, d));

    own_door
        .or(nearest_door)
        .and_then(toward)
        .or_else(|| toward(spawn))
        .unwrap_or(Vec3::Z)
}

/// Pick a random walkable tile at least [`SPAWN_CLEARANCE`] tiles from the
/// player's spawn on at least one axis.
fn far_tile(map: &LevelMap, rng: &mut SmallRng, sx: i32, sy: i32) -> (usize, usize) {
    loop {
        let t = random_walkable(map, rng);
        let far = (t.0 as i32 - sx).abs() >= SPAWN_CLEARANCE
            || (t.1 as i32 - sy).abs() >= SPAWN_CLEARANCE;
        if far {
            return t;
        }
    }
}

/// A seeded random horizontal heading, for guards with no landmark to face.
fn random_heading(rng: &mut SmallRng) -> Vec3 {
    let angle = rng.gen_range(0.0..std::f32::consts::TAU);
    Vec3::new(angle.cos(), 0.0, angle.sin())
}

/// The vault patroller's beat: a post one tile outside the vault door, a heading
/// looking at the door, and a route pacing the door's frontage to either side so
/// the guard keeps walking past the front door. `None` when the source has no
/// sealed vault, or the door has no open tile in front of it to stand on.
///
/// Everything is anchored to the door tile and the *outside* of the wall, so it's
/// independent of whether the door itself is currently locked (solid) — the gate
/// plugs the doorway tile, which this never steps on.
#[allow(clippy::type_complexity)]
fn vault_beat(map: &LevelMap) -> Option<((usize, usize), Vec3, Vec<(usize, usize)>)> {
    let vault = map
        .rooms()
        .iter()
        .find(|r| r.kind == RoomKind::Vault && r.doorway.is_some())?;
    let door = vault.doorway?;
    let (cx, cy) = vault.rect.center();

    // Outward step from the room centre through the door: the door sits on a
    // perimeter edge, so this points from inside to outside along that edge.
    let out = (
        (door.0 as i32 - cx as i32).signum(),
        (door.1 as i32 - cy as i32).signum(),
    );
    // The post: one tile outside the door. Abort if it isn't open floor.
    let front = offset(map, door, out)?;
    if !map.is_walkable(front.0, front.1) {
        return None;
    }

    // Pace along the wall face (perpendicular to `out`), extending to each side as
    // far as the frontage stays walkable, up to the span. The endpoints flank the
    // door, so cycling between them walks back and forth across it.
    let perp = (-out.1, out.0);
    let left = walk_face(map, front, perp, VAULT_PATROL_SPAN);
    let right = walk_face(map, front, (-perp.0, -perp.1), VAULT_PATROL_SPAN);
    let mut route = vec![left, right];
    route.dedup();
    if route.len() < 2 {
        // No room to pace — just hold the door front and sweep.
        route = vec![front];
    }

    let heading = face_toward(map, front, door);
    Some((front, heading, route))
}

/// A static guard's post for watching the code note: a walkable vantage roughly
/// [`WATCH_DISTANCE`] tiles off the note, facing it. Tries cardinals then
/// diagonals for the first clear standoff; falls back to the note's own tile if
/// the surroundings are tight.
fn watch_post(map: &LevelMap, note: (usize, usize)) -> ((usize, usize), Vec3) {
    const DIRS: [(i32, i32); 8] = [
        (1, 0),
        (-1, 0),
        (0, 1),
        (0, -1),
        (1, 1),
        (1, -1),
        (-1, 1),
        (-1, -1),
    ];
    for (dx, dy) in DIRS {
        if let Some(t) = offset(map, note, (dx * WATCH_DISTANCE, dy * WATCH_DISTANCE))
            && map.is_walkable(t.0, t.1)
        {
            return (t, face_toward(map, t, note));
        }
    }
    (note, Vec3::Z)
}

/// A normalised horizontal heading from one tile toward another (Z if they
/// coincide).
fn face_toward(map: &LevelMap, from: (usize, usize), to: (usize, usize)) -> Vec3 {
    let dir = horizontal(map.tile_to_world(to.0, to.1) - map.tile_to_world(from.0, from.1));
    dir.try_normalize().unwrap_or(Vec3::Z)
}

/// `tile` shifted by a signed delta, or `None` if it leaves the grid.
fn offset(map: &LevelMap, tile: (usize, usize), delta: (i32, i32)) -> Option<(usize, usize)> {
    let x = tile.0 as i32 + delta.0;
    let y = tile.1 as i32 + delta.1;
    if x < 0 || y < 0 || x as usize >= map.width || y as usize >= map.height {
        None
    } else {
        Some((x as usize, y as usize))
    }
}

/// Step out from `from` along `dir`, returning the furthest contiguously walkable
/// tile reached within `max` steps (`from` itself if the first step is blocked).
fn walk_face(map: &LevelMap, from: (usize, usize), dir: (i32, i32), max: i32) -> (usize, usize) {
    let mut last = from;
    for step in 1..=max {
        match offset(map, from, (dir.0 * step, dir.1 * step)) {
            Some(t) if map.is_walkable(t.0, t.1) => last = t,
            _ => break,
        }
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::map::{Room, Tile, TileRect};

    /// An all-floor square map for exercising placement helpers.
    fn open_map(size: usize) -> LevelMap {
        let mut map = LevelMap::filled_with_walls(size, size);
        for y in 0..size {
            for x in 0..size {
                map.set(x, y, Tile::Floor);
            }
        }
        map
    }

    #[test]
    fn patrol_route_starts_at_spawn_and_stays_local() {
        let map = open_map(40);
        let start = (20, 20);
        let mut rng = SmallRng::seed_from_u64(7);
        let route = patrol_route(&map, &mut rng, start);

        assert_eq!(route.len(), PATROL_WAYPOINTS);
        assert_eq!(route[0], start, "route begins at the spawn tile");
        for &wp in &route[1..] {
            assert!(
                chebyshev(start, wp) <= PATROL_RADIUS,
                "waypoint {wp:?} is within the local beat radius of {start:?}",
            );
        }
    }

    #[test]
    fn static_guard_faces_its_room_doorway() {
        let mut map = open_map(11);
        // A room the guard stands in, with its doorway off to the +x side.
        let guard = (5, 5);
        let door = (8, 5);
        map.set_rooms(vec![Room {
            kind: RoomKind::Security,
            rect: TileRect {
                min_x: 3,
                min_y: 3,
                max_x: 7,
                max_y: 7,
            },
            tiles: vec![guard],
            doorway: Some(door),
        }]);

        let h = interesting_facing(&map, guard, (0, 0));
        assert!(h.x > 0.9, "faces toward the +x doorway: {h:?}");
        assert!(h.z.abs() < 0.1, "no sideways lean: {h:?}");
    }

    #[test]
    fn static_guard_without_doorways_faces_spawn() {
        let map = open_map(11); // no rooms recorded
        let guard = (5, 5);
        let spawn = (5, 9); // straight along +z from the guard
        let h = interesting_facing(&map, guard, spawn);
        assert!(h.z > 0.9, "faces toward the spawn: {h:?}");
        assert!(h.x.abs() < 0.1, "no sideways lean: {h:?}");
    }

    #[test]
    fn vault_beat_posts_outside_the_door_and_paces_its_face() {
        let mut map = open_map(11);
        // A vault whose single doorway is on its top (−y) edge.
        let door = (5, 3);
        map.set_rooms(vec![Room {
            kind: RoomKind::Vault,
            rect: TileRect {
                min_x: 3,
                min_y: 3,
                max_x: 7,
                max_y: 7,
            },
            tiles: vec![(5, 5)],
            doorway: Some(door),
        }]);

        let (post, heading, route) = vault_beat(&map).expect("vault has a beat");

        // Posts one tile outside the door (centre at (5,5), so outward is −y).
        assert_eq!(post, (5, 2), "stands just outside the door");
        // Looks back at the door (+z, since +y maps to +z in world space).
        assert!(heading.z > 0.9, "watches the door: {heading:?}");
        // Paces the frontage: two endpoints flanking the door along its edge (x).
        assert_eq!(route.len(), 2);
        assert!(
            route.iter().all(|&(_, y)| y == post.1),
            "endpoints sit on the door's frontage line: {route:?}",
        );
        assert!(
            route.iter().any(|&(x, _)| x < 5) && route.iter().any(|&(x, _)| x > 5),
            "endpoints flank the door on both sides: {route:?}",
        );
    }

    #[test]
    fn vault_beat_is_none_without_a_vault() {
        let map = open_map(11); // no rooms recorded
        assert!(vault_beat(&map).is_none());
    }

    #[test]
    fn watch_post_stands_off_the_code_and_faces_it() {
        let map = open_map(11);
        let note = (5, 5);
        let (post, heading) = watch_post(&map, note);

        assert_eq!(
            chebyshev(post, note),
            WATCH_DISTANCE,
            "stands the watch distance off the note: {post:?}",
        );
        // The first clear direction tried is +x, so it looks back along −x.
        assert!(heading.x < -0.9, "watches the note: {heading:?}");
    }
}
