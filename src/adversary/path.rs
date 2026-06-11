//! Grid pathfinding for adversaries: shortest routes and random goals over the
//! walkable tiles of a [`LevelMap`]. Pure functions — no ECS, no `Adversary`
//! state — so they're unit-testable on a hand-built map.

use std::collections::VecDeque;

use rand::Rng;
use rand::rngs::SmallRng;

use crate::level::LevelMap;

/// Breadth-first shortest path over walkable tiles, returning the waypoints
/// after `start` up to and including `goal`. Empty when already at the goal,
/// `None` when no route exists.
pub(super) fn bfs_path(
    map: &LevelMap,
    start: (usize, usize),
    goal: (usize, usize),
) -> Option<Vec<(usize, usize)>> {
    if start == goal {
        return Some(Vec::new());
    }
    let (w, h) = (map.width, map.height);
    let mut came: Vec<Option<(usize, usize)>> = vec![None; w * h];
    let mut visited = vec![false; w * h];
    let mut queue = VecDeque::new();

    visited[start.1 * w + start.0] = true;
    queue.push_back(start);

    while let Some((cx, cy)) = queue.pop_front() {
        if (cx, cy) == goal {
            let mut path = Vec::new();
            let mut cur = goal;
            while cur != start {
                path.push(cur);
                cur = came[cur.1 * w + cur.0].expect("reconstruct reaches start");
            }
            path.reverse();
            return Some(path);
        }
        for (nx, ny) in neighbours(cx, cy, w, h) {
            let idx = ny * w + nx;
            if !visited[idx] && map.is_walkable(nx, ny) {
                visited[idx] = true;
                came[idx] = Some((cx, cy));
                queue.push_back((nx, ny));
            }
        }
    }
    None
}

/// Tiles to investigate around `origin` while searching: walkable tiles within
/// Chebyshev `radius` reachable from `origin` without leaving that radius,
/// ordered ring-by-ring then by a fixed bearing. The ordering is a total order
/// over distinct tiles, so the result is deterministic (independent of the
/// flood's visit order) and a guard's search pattern replays identically.
/// Excludes `origin` itself.
pub(super) fn search_ring(
    map: &LevelMap,
    origin: (usize, usize),
    radius: i32,
) -> Vec<(usize, usize)> {
    let chebyshev = |t: (usize, usize)| {
        let dx = (t.0 as i32 - origin.0 as i32).abs();
        let dy = (t.1 as i32 - origin.1 as i32).abs();
        dx.max(dy)
    };

    // Bounded BFS flood: only tiles within `radius` of origin, so the cost stays
    // local rather than flooding the whole connected component.
    let mut reachable = vec![false; map.width * map.height];
    let mut queue = VecDeque::new();
    reachable[origin.1 * map.width + origin.0] = true;
    queue.push_back(origin);
    while let Some((cx, cy)) = queue.pop_front() {
        for (nx, ny) in neighbours(cx, cy, map.width, map.height) {
            let idx = ny * map.width + nx;
            if !reachable[idx] && chebyshev((nx, ny)) <= radius && map.is_walkable(nx, ny) {
                reachable[idx] = true;
                queue.push_back((nx, ny));
            }
        }
    }

    let mut tiles: Vec<(usize, usize)> = (0..map.width * map.height)
        .filter(|&i| reachable[i])
        .map(|i| (i % map.width, i / map.width))
        .filter(|&t| t != origin)
        .collect();
    let bearing = |t: (usize, usize)| {
        ((t.1 as i32 - origin.1 as i32) as f32).atan2((t.0 as i32 - origin.0 as i32) as f32)
    };
    tiles.sort_by(|&a, &b| {
        chebyshev(a).cmp(&chebyshev(b)).then(
            bearing(a)
                .partial_cmp(&bearing(b))
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });
    tiles
}

/// Pick a uniformly random walkable tile. The connected map guarantees one
/// exists, so this always terminates.
pub(super) fn random_walkable(map: &LevelMap, rng: &mut SmallRng) -> (usize, usize) {
    loop {
        let x = rng.gen_range(0..map.width);
        let y = rng.gen_range(0..map.height);
        if map.is_walkable(x, y) {
            return (x, y);
        }
    }
}

fn neighbours(x: usize, y: usize, w: usize, h: usize) -> impl Iterator<Item = (usize, usize)> {
    let mut out = Vec::with_capacity(4);
    if x > 0 {
        out.push((x - 1, y));
    }
    if x + 1 < w {
        out.push((x + 1, y));
    }
    if y > 0 {
        out.push((x, y - 1));
    }
    if y + 1 < h {
        out.push((x, y + 1));
    }
    out.into_iter()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::map::Tile;

    /// Carve an L-shaped corridor and check BFS follows it around the wall.
    #[test]
    fn bfs_follows_an_l_corridor() {
        let mut map = LevelMap::filled_with_walls(5, 5);
        // Horizontal leg along y=1, vertical leg up x=3.
        for x in 1..=3 {
            map.set(x, 1, Tile::Floor);
        }
        for y in 1..=3 {
            map.set(3, y, Tile::Floor);
        }

        let path = bfs_path(&map, (1, 1), (3, 3)).expect("route exists");
        // 4 steps along the L (excludes the start tile).
        assert_eq!(path, vec![(2, 1), (3, 1), (3, 2), (3, 3)]);
    }

    #[test]
    fn bfs_returns_none_when_walled_off() {
        let mut map = LevelMap::filled_with_walls(3, 3);
        map.set(0, 0, Tile::Floor);
        map.set(2, 2, Tile::Floor);
        assert!(bfs_path(&map, (0, 0), (2, 2)).is_none());
    }

    #[test]
    fn bfs_to_self_is_empty() {
        let mut map = LevelMap::filled_with_walls(3, 3);
        map.set(1, 1, Tile::Floor);
        assert_eq!(bfs_path(&map, (1, 1), (1, 1)), Some(Vec::new()));
    }

    /// On an open field the ring is every tile within the radius, excluding the
    /// origin, and ordered ring-by-ring (nearest tiles first).
    #[test]
    fn search_ring_covers_radius_in_order() {
        let mut map = LevelMap::filled_with_walls(7, 7);
        for y in 0..7 {
            for x in 0..7 {
                map.set(x, y, Tile::Floor);
            }
        }
        let ring = search_ring(&map, (3, 3), 2);

        // 5x5 block minus the origin tile.
        assert_eq!(ring.len(), 24);
        assert!(!ring.contains(&(3, 3)));
        // Ring 1 (the 8 immediate neighbours) all come before ring 2.
        let cheby = |&(x, y): &(usize, usize)| (x as i32 - 3).abs().max((y as i32 - 3).abs());
        let first_ring2 = ring.iter().position(|t| cheby(t) == 2).unwrap();
        assert!(ring[..first_ring2].iter().all(|t| cheby(t) == 1));
        assert_eq!(first_ring2, 8);
    }

    /// Tiles walled off from the origin (or beyond the radius) are excluded.
    #[test]
    fn search_ring_skips_unreachable_and_distant() {
        // A 1-wide horizontal corridor: only tiles reachable along it count.
        let mut map = LevelMap::filled_with_walls(7, 3);
        for x in 1..6 {
            map.set(x, 1, Tile::Floor);
        }
        let ring = search_ring(&map, (3, 1), 3);
        // Reachable floor within radius 3 along the corridor: x in {1,2,4,5},
        // ordered ring-by-ring (|dx| 1 then 2) and +x (bearing 0) before −x (π).
        assert_eq!(ring, vec![(4, 1), (2, 1), (5, 1), (1, 1)]);
    }
}
