//! Pure navigation logic: walkability grid (downsampled from the map),
//! greedy rectangle meshing of blocked pixels (→ static colliders), A* with
//! string-pulling smoothing for soldiers, and a Dijkstra flow field that
//! hundreds of enemies can follow cheaply.

/// Downsampling factor map-pixels → nav cells.
pub const CELL: u32 = 2;

#[derive(Debug, Clone)]
pub struct NavGrid {
    pub w: u32,
    pub h: u32,
    /// true = impassable (building or water)
    pub blocked: Vec<bool>,
    /// map width/height in pixels (world units)
    pub map_w: u32,
    pub map_h: u32,
}

impl NavGrid {
    /// Build from the per-pixel blocked mask of a rendered map. A nav cell is
    /// blocked if any of its pixels is blocked (conservative — no clipping
    /// through corners).
    pub fn from_blocked(map_w: u32, map_h: u32, blocked_px: &[bool]) -> Self {
        let w = map_w.div_ceil(CELL);
        let h = map_h.div_ceil(CELL);
        let mut blocked = vec![false; (w * h) as usize];
        for py in 0..map_h {
            for px in 0..map_w {
                if blocked_px[(py * map_w + px) as usize] {
                    blocked[((py / CELL) * w + px / CELL) as usize] = true;
                }
            }
        }
        NavGrid {
            w,
            h,
            blocked,
            map_w,
            map_h,
        }
    }

    #[inline]
    pub fn idx(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            None
        } else {
            Some((y as u32 * self.w + x as u32) as usize)
        }
    }

    #[inline]
    pub fn is_blocked(&self, x: i32, y: i32) -> bool {
        self.idx(x, y).map(|i| self.blocked[i]).unwrap_or(true)
    }

    /// World position (map px, y-down) → nav cell.
    pub fn cell_of(&self, x: f32, y: f32) -> (i32, i32) {
        ((x / CELL as f32) as i32, (y / CELL as f32) as i32)
    }

    /// Centre of a nav cell in world/map coordinates.
    pub fn centre(&self, c: (i32, i32)) -> (f32, f32) {
        (
            (c.0 as f32 + 0.5) * CELL as f32,
            (c.1 as f32 + 0.5) * CELL as f32,
        )
    }

    /// Nearest walkable cell to `c` (spiral search).
    pub fn nearest_walkable(&self, c: (i32, i32)) -> Option<(i32, i32)> {
        if !self.is_blocked(c.0, c.1) {
            return Some(c);
        }
        for r in 1..=64i32 {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs().max(dy.abs()) == r && !self.is_blocked(c.0 + dx, c.1 + dy) {
                        return Some((c.0 + dx, c.1 + dy));
                    }
                }
            }
        }
        None
    }

    /// Straight-line walkability between two world points (supercover ray).
    pub fn line_walkable(&self, a: (f32, f32), b: (f32, f32)) -> bool {
        let steps = ((b.0 - a.0).abs().max((b.1 - a.1).abs()) / (CELL as f32) * 2.0).ceil() as i32;
        for i in 0..=steps.max(1) {
            let t = i as f32 / steps.max(1) as f32;
            let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            let c = self.cell_of(x, y);
            if self.is_blocked(c.0, c.1) {
                return false;
            }
        }
        true
    }

    /// A* from world point to world point; returns smoothed world waypoints
    /// (excluding the start, including the goal).
    pub fn path(&self, from: (f32, f32), to: (f32, f32)) -> Option<Vec<(f32, f32)>> {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        let start = self.nearest_walkable(self.cell_of(from.0, from.1))?;
        let goal = self.nearest_walkable(self.cell_of(to.0, to.1))?;
        if start == goal {
            return Some(vec![self.centre(goal)]);
        }
        let n = (self.w * self.h) as usize;
        let mut best = vec![u32::MAX; n];
        let mut prev = vec![u32::MAX; n];
        let h = |x: i32, y: i32| -> u32 {
            let (dx, dy) = ((x - goal.0).unsigned_abs(), (y - goal.1).unsigned_abs());
            let (lo, hi) = (dx.min(dy), dx.max(dy));
            lo * 14 + (hi - lo) * 10
        };
        let si = self.idx(start.0, start.1)?;
        best[si] = 0;
        let mut open: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new();
        open.push(Reverse((h(start.0, start.1), si as u32)));
        let mut found = false;
        let gi = self.idx(goal.0, goal.1)?;
        while let Some(Reverse((_, ci))) = open.pop() {
            if ci as usize == gi {
                found = true;
                break;
            }
            let (cx, cy) = ((ci % self.w) as i32, (ci / self.w) as i32);
            let g0 = best[ci as usize];
            for (dx, dy, cost) in [
                (1, 0, 10),
                (-1, 0, 10),
                (0, 1, 10),
                (0, -1, 10),
                (1, 1, 14),
                (1, -1, 14),
                (-1, 1, 14),
                (-1, -1, 14),
            ] {
                let (nx, ny) = (cx + dx, cy + dy);
                if self.is_blocked(nx, ny) {
                    continue;
                }
                // no diagonal corner cutting
                if dx != 0
                    && dy != 0
                    && (self.is_blocked(cx + dx, cy) || self.is_blocked(cx, cy + dy))
                {
                    continue;
                }
                let ni = self.idx(nx, ny).unwrap();
                let g = g0 + cost;
                if g < best[ni] {
                    best[ni] = g;
                    prev[ni] = ci;
                    open.push(Reverse((g + h(nx, ny), ni as u32)));
                }
            }
        }
        if !found {
            return None;
        }
        let mut cells = vec![goal];
        let mut i = gi as u32;
        while prev[i as usize] != u32::MAX {
            i = prev[i as usize];
            let c = ((i % self.w) as i32, (i / self.w) as i32);
            if c == start {
                break;
            }
            cells.push(c);
        }
        cells.reverse();
        // string pulling: skip waypoints while the direct line is walkable
        let mut out: Vec<(f32, f32)> = Vec::new();
        let mut anchor = from;
        let mut k = 0;
        while k < cells.len() {
            let mut far = k;
            while far + 1 < cells.len() && self.line_walkable(anchor, self.centre(cells[far + 1])) {
                far += 1;
            }
            anchor = self.centre(cells[far]);
            out.push(anchor);
            k = far + 1;
        }
        Some(out)
    }
}

/// Dijkstra flow field: for every walkable cell, the direction to step to get
/// closer to the nearest goal. Recomputed periodically; hordes just sample it.
#[derive(Debug, Clone, Default)]
pub struct FlowField {
    pub w: u32,
    pub h: u32,
    /// (dx, dy) per cell in {-1,0,1}; (0,0) = unreachable or at goal.
    pub dir: Vec<(i8, i8)>,
}

impl FlowField {
    pub fn compute(grid: &NavGrid, goals: &[(f32, f32)]) -> Self {
        use std::collections::VecDeque;
        let n = (grid.w * grid.h) as usize;
        let mut dist = vec![u32::MAX; n];
        let mut q = VecDeque::new();
        for g in goals {
            let c = grid.cell_of(g.0, g.1);
            if let Some(c) = grid.nearest_walkable(c) {
                let i = grid.idx(c.0, c.1).unwrap();
                if dist[i] != 0 {
                    dist[i] = 0;
                    q.push_back(c);
                }
            }
        }
        while let Some((x, y)) = q.pop_front() {
            let d = dist[grid.idx(x, y).unwrap()];
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (nx, ny) = (x + dx, y + dy);
                if grid.is_blocked(nx, ny) {
                    continue;
                }
                let ni = grid.idx(nx, ny).unwrap();
                if dist[ni] == u32::MAX {
                    dist[ni] = d + 1;
                    q.push_back((nx, ny));
                }
            }
        }
        let mut dir = vec![(0i8, 0i8); n];
        for y in 0..grid.h as i32 {
            for x in 0..grid.w as i32 {
                let i = grid.idx(x, y).unwrap();
                if grid.blocked[i] || dist[i] == u32::MAX || dist[i] == 0 {
                    continue;
                }
                let mut bd = dist[i];
                let mut bv = (0i8, 0i8);
                for (dx, dy) in [
                    (1, 0),
                    (-1, 0),
                    (0, 1),
                    (0, -1),
                    (1, 1),
                    (1, -1),
                    (-1, 1),
                    (-1, -1),
                ] {
                    if dx != 0
                        && dy != 0
                        && (grid.is_blocked(x + dx, y) || grid.is_blocked(x, y + dy))
                    {
                        continue;
                    }
                    if let Some(ni) = grid.idx(x + dx, y + dy)
                        && !grid.blocked[ni]
                        && dist[ni] < bd
                    {
                        bd = dist[ni];
                        bv = (dx as i8, dy as i8);
                    }
                }
                dir[i] = bv;
            }
        }
        FlowField {
            w: grid.w,
            h: grid.h,
            dir,
        }
    }

    /// Unit-length-ish step direction at a world position (map px, y-down).
    pub fn sample(&self, grid: &NavGrid, x: f32, y: f32) -> (f32, f32) {
        if self.dir.is_empty() {
            return (0.0, 0.0);
        }
        let c = grid.cell_of(x, y);
        match grid.idx(c.0, c.1) {
            Some(i) => {
                let (dx, dy) = self.dir[i];
                let v = (dx as f32, dy as f32);
                let len = (v.0 * v.0 + v.1 * v.1).sqrt();
                if len > 0.0 {
                    (v.0 / len, v.1 / len)
                } else {
                    (0.0, 0.0)
                }
            }
            None => (0.0, 0.0),
        }
    }
}

/// Greedy meshing: cover all blocked pixels with as few axis-aligned
/// rectangles as practical (→ static physics colliders). Returns
/// (x, y, w, h) in map pixels.
pub fn greedy_rects(map_w: u32, map_h: u32, blocked_px: &[bool]) -> Vec<(u32, u32, u32, u32)> {
    let mut used = vec![false; blocked_px.len()];
    let mut out = Vec::new();
    let at = |x: u32, y: u32| (y * map_w + x) as usize;
    for y in 0..map_h {
        for x in 0..map_w {
            let i = at(x, y);
            if !blocked_px[i] || used[i] {
                continue;
            }
            // grow right
            let mut w = 1;
            while x + w < map_w && blocked_px[at(x + w, y)] && !used[at(x + w, y)] {
                w += 1;
            }
            // grow down while the full row is blocked & unused
            let mut h = 1;
            'grow: while y + h < map_h {
                for xx in x..x + w {
                    let j = at(xx, y + h);
                    if !blocked_px[j] || used[j] {
                        break 'grow;
                    }
                }
                h += 1;
            }
            for yy in y..y + h {
                for xx in x..x + w {
                    used[at(xx, yy)] = true;
                }
            }
            out.push((x, y, w, h));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 20×20 map, one 8×8 building in the middle.
    fn grid_with_building() -> NavGrid {
        let (w, h) = (20u32, 20u32);
        let mut blocked = vec![false; (w * h) as usize];
        for y in 6..14 {
            for x in 6..14 {
                blocked[(y * w + x) as usize] = true;
            }
        }
        NavGrid::from_blocked(w, h, &blocked)
    }

    #[test]
    fn path_goes_around_building_not_through() {
        let g = grid_with_building();
        let path = g.path((2.0, 10.0), (18.0, 10.0)).expect("path");
        assert!(!path.is_empty());
        // walk the polyline: every sampled point must be walkable
        let mut prev = (2.0, 10.0);
        for wp in &path {
            assert!(
                g.line_walkable(prev, *wp),
                "segment {prev:?}→{wp:?} crosses the building"
            );
            prev = *wp;
        }
        // straight line would cross the building
        assert!(!g.line_walkable((2.0, 10.0), (18.0, 10.0)));
        let end = path.last().unwrap();
        assert!((end.0 - 18.0).abs() < 3.0 && (end.1 - 10.0).abs() < 3.0);
    }

    #[test]
    fn path_none_when_sealed() {
        let (w, h) = (10u32, 10u32);
        let mut blocked = vec![false; 100];
        for x in 0..10 {
            blocked[(4 * w + x) as usize] = true;
            blocked[(5 * w + x) as usize] = true;
        }
        let g = NavGrid::from_blocked(w, h, &blocked);
        assert!(g.path((5.0, 1.0), (5.0, 9.0)).is_none());
    }

    #[test]
    fn flow_field_points_toward_goal() {
        let g = grid_with_building();
        let f = FlowField::compute(&g, &[(2.0, 10.0)]);
        // east of the building: flow must not be zero and must route around
        let (dx, _dy) = f.sample(&g, 18.0, 10.0);
        assert!(dx != 0.0 || _dy != 0.0);
        // following the field from the east side reaches the goal
        let (mut x, mut y) = (18.0, 10.0);
        for _ in 0..300 {
            let (dx, dy) = f.sample(&g, x, y);
            if dx == 0.0 && dy == 0.0 {
                break;
            }
            x += dx * 1.0;
            y += dy * 1.0;
        }
        assert!(
            (x - 2.0).abs() < 4.0 && (y - 10.0).abs() < 4.0,
            "ended at {x},{y}"
        );
    }

    #[test]
    fn greedy_rects_cover_exactly() {
        let (w, h) = (12u32, 8u32);
        let mut blocked = vec![false; (w * h) as usize];
        for y in 1..4 {
            for x in 2..7 {
                blocked[(y * w + x) as usize] = true;
            }
        }
        blocked[(6 * w + 10) as usize] = true;
        let rects = greedy_rects(w, h, &blocked);
        assert_eq!(rects.len(), 2);
        let mut covered = vec![false; blocked.len()];
        for (x, y, rw, rh) in rects {
            for yy in y..y + rh {
                for xx in x..x + rw {
                    covered[(yy * w + xx) as usize] = true;
                }
            }
        }
        assert_eq!(covered, blocked);
    }

    #[test]
    fn nearest_walkable_escapes_building() {
        let g = grid_with_building();
        let c = g.nearest_walkable(g.cell_of(10.0, 10.0)).unwrap();
        assert!(!g.is_blocked(c.0, c.1));
    }
}
