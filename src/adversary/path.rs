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
}
