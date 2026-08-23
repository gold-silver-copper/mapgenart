//! Fog of war: per-map-pixel visibility with line of sight blocked by
//! buildings. Three states per pixel: unexplored (never seen, black),
//! explored (seen before, dimmed, buildings remembered) and visible.

pub const UNEXPLORED: u8 = 0;
pub const EXPLORED: u8 = 1;
pub const VISIBLE: u8 = 2;

#[derive(Debug, Clone)]
pub struct Fog {
    pub w: u32,
    pub h: u32,
    pub state: Vec<u8>,
    /// Bounding boxes touched by the last update (for partial texture upload).
    pub dirty: Vec<(u32, u32, u32, u32)>,
}

impl Fog {
    pub fn new(w: u32, h: u32) -> Self {
        Fog {
            w,
            h,
            state: vec![UNEXPLORED; (w * h) as usize],
            dirty: Vec::new(),
        }
    }

    #[inline]
    fn idx(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            None
        } else {
            Some((y as u32 * self.w + x as u32) as usize)
        }
    }

    pub fn is_visible(&self, x: f32, y: f32) -> bool {
        self.idx(x as i32, y as i32)
            .map(|i| self.state[i] == VISIBLE)
            .unwrap_or(false)
    }

    /// Recompute visibility for the given viewers. `sight_blocked` is the
    /// per-pixel building mask (same dimensions as the fog).
    pub fn update(&mut self, sight_blocked: &[bool], viewers: &[(f32, f32)], radius: f32) {
        self.dirty.clear();
        // demote all currently-visible pixels (they stay explored)
        for s in self.state.iter_mut() {
            if *s == VISIBLE {
                *s = EXPLORED;
            }
        }
        self.dirty.push((0, 0, self.w, self.h));
        let r = radius.max(1.0);
        let ri = r.ceil() as i32;
        for &(vx, vy) in viewers {
            let (cx, cy) = (vx as i32, vy as i32);
            // cast a ray to every perimeter cell of the vision square
            let mut ray = |tx: i32, ty: i32| {
                let dx = (tx - cx) as f32;
                let dy = (ty - cy) as f32;
                let len = (dx * dx + dy * dy).sqrt();
                if len == 0.0 {
                    return;
                }
                let steps = len.ceil() as i32;
                for s in 0..=steps {
                    let t = s as f32 / steps as f32;
                    if t * len > r {
                        break;
                    }
                    let (x, y) = (cx + (dx * t).round() as i32, cy + (dy * t).round() as i32);
                    let Some(i) = self.idx(x, y) else { break };
                    self.state[i] = VISIBLE;
                    if sight_blocked[i] {
                        break; // the wall itself is visible, nothing behind it
                    }
                }
            };
            for t in -ri..=ri {
                ray(cx + t, cy - ri);
                ray(cx + t, cy + ri);
                ray(cx - ri, cy + t);
                ray(cx + ri, cy + t);
            }
            // the viewer's own pixel
            if let Some(i) = self.idx(cx, cy) {
                self.state[i] = VISIBLE;
            }
        }
    }

    /// Direct line of sight between two points (for targeting).
    pub fn line_of_sight(
        sight_blocked: &[bool],
        w: u32,
        h: u32,
        a: (f32, f32),
        b: (f32, f32),
    ) -> bool {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt();
        let steps = len.ceil().max(1.0) as i32;
        for s in 1..steps {
            let t = s as f32 / steps as f32;
            let (x, y) = ((a.0 + dx * t) as i32, (a.1 + dy * t) as i32);
            if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
                return false;
            }
            if sight_blocked[(y as u32 * w + x as u32) as usize] {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 40×20, vertical wall at x=20 with no gaps (y 0..20).
    fn wall() -> (Vec<bool>, u32, u32) {
        let (w, h) = (40u32, 20u32);
        let mut m = vec![false; (w * h) as usize];
        for y in 0..h {
            m[(y * w + 20) as usize] = true;
        }
        (m, w, h)
    }

    #[test]
    fn los_blocked_by_wall() {
        let (m, w, h) = wall();
        assert!(!Fog::line_of_sight(&m, w, h, (5.0, 10.0), (35.0, 10.0)));
        assert!(Fog::line_of_sight(&m, w, h, (5.0, 10.0), (15.0, 10.0)));
    }

    #[test]
    fn fog_states_transition() {
        let (m, w, h) = wall();
        let mut fog = Fog::new(w, h);
        fog.update(&m, &[(5.0, 10.0)], 10.0);
        assert_eq!(fog.state[(10 * w + 5) as usize], VISIBLE);
        // behind the wall stays unexplored
        assert_eq!(fog.state[(10 * w + 30) as usize], UNEXPLORED);
        // move away: previously visible becomes explored
        fog.update(&m, &[(5.0, 3.0)], 4.0);
        assert_eq!(fog.state[(10 * w + 5) as usize], EXPLORED);
        assert_eq!(fog.state[(3 * w + 5) as usize], VISIBLE);
    }

    #[test]
    fn wall_itself_is_visible_but_not_behind() {
        let (m, w, h) = wall();
        let mut fog = Fog::new(w, h);
        fog.update(&m, &[(15.0, 10.0)], 12.0);
        assert_eq!(
            fog.state[(10 * w + 20) as usize],
            VISIBLE,
            "wall pixel seen"
        );
        assert_eq!(
            fog.state[(10 * w + 26) as usize],
            UNEXPLORED,
            "behind wall hidden"
        );
    }
}
