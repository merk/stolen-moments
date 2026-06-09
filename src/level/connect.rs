//! Protected-aware connectivity: carve corridors until the whole level is one
//! connected component, crossing only *carvable* walls. Sealed room rings and
//! the map border are protected, so sealed rooms stay reachable solely through
//! their doorways while every floor tile is reachable from the spawn.

use std::collections::VecDeque;

use bevy::prelude::warn;

use super::map::{LevelMap, Tile};

/// Carve non-protected walls until every floor tile is connected to `start`.
///
/// Repeatedly: label the floor components; if more than one remains, run a 0-1
/// BFS out of the `start` component through *carvable* walls (cost 1) and floor
/// (cost 0), treating `protected` tiles as impassable, until it reaches another
/// component's floor. The cheapest such route is carved, merging that component
/// in. Protected tiles — sealed rings and the border — are never crossed.
pub(crate) fn carve_connect(map: &mut LevelMap, protected: &[bool], start: (usize, usize)) {
    let (w, h) = (map.width, map.height);
    loop {
        let labels = label_components(map);
        let Some(main) = labels[start.1 * w + start.0] else {
            return; // start isn't floor — nothing sensible to connect.
        };
        let has_other = labels.iter().any(|&l| matches!(l, Some(l) if l != main));
        if !has_other {
            return;
        }

        let mut dist = vec![u32::MAX; w * h];
        let mut came: Vec<Option<usize>> = vec![None; w * h];
        let mut dq: VecDeque<usize> = VecDeque::new();
        for (i, &label) in labels.iter().enumerate() {
            if label == Some(main) {
                dist[i] = 0;
                dq.push_back(i);
            }
        }

        let mut target: Option<usize> = None;
        while let Some(ci) = dq.pop_front() {
            // First floor tile reached that belongs to another component wins.
            if matches!(labels[ci], Some(l) if l != main) {
                target = Some(ci);
                break;
            }
            let (cx, cy) = (ci % w, ci / w);
            let d = dist[ci];
            for (nx, ny) in neighbours4(cx, cy, w, h) {
                let ni = ny * w + nx;
                if protected[ni] {
                    continue;
                }
                let step = if map.get(nx, ny) == Tile::Wall { 1 } else { 0 };
                let nd = d + step;
                if nd < dist[ni] {
                    dist[ni] = nd;
                    came[ni] = Some(ci);
                    if step == 0 {
                        dq.push_front(ni);
                    } else {
                        dq.push_back(ni);
                    }
                }
            }
        }

        let Some(t) = target else {
            warn!("carve_connect: some floor is boxed in by protected walls; leaving disconnected");
            return;
        };

        // Carve every wall tile along the route back to the start component.
        let mut cur = t;
        while let Some(prev) = came[cur] {
            let (cx, cy) = (cur % w, cur / w);
            if map.get(cx, cy) == Tile::Wall {
                map.set(cx, cy, Tile::Floor);
            }
            cur = prev;
        }
    }
}

/// Flood-fill every walkable tile into a connected-component label.
pub(crate) fn label_components(map: &LevelMap) -> Vec<Option<usize>> {
    let (w, h) = (map.width, map.height);
    let mut labels = vec![None; w * h];
    let mut next = 0usize;

    for y in 0..h {
        for x in 0..w {
            if !map.is_walkable(x, y) || labels[y * w + x].is_some() {
                continue;
            }
            let mut queue = VecDeque::new();
            queue.push_back((x, y));
            labels[y * w + x] = Some(next);
            while let Some((cx, cy)) = queue.pop_front() {
                for (nx, ny) in neighbours4(cx, cy, w, h) {
                    let idx = ny * w + nx;
                    if labels[idx].is_none() && map.is_walkable(nx, ny) {
                        labels[idx] = Some(next);
                        queue.push_back((nx, ny));
                    }
                }
            }
            next += 1;
        }
    }
    labels
}

fn neighbours4(x: usize, y: usize, w: usize, h: usize) -> impl Iterator<Item = (usize, usize)> {
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
