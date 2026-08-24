//! Procedural doors and windows: carve openings into the building walls so
//! interiors are enterable, and punch windows that sight (but not movement)
//! passes through.

use crate::generate::Generated;
use crate::raster::layer;

/// A carved opening in a building wall.
#[derive(Debug, Clone)]
pub struct Opening {
    pub kind: OpeningKind,
    /// map-pixel centre of the opening
    pub centre: (f32, f32),
    /// the wall pixels that were removed (doors) or made see-through (windows)
    pub pixels: Vec<usize>,
    /// interior component this opening belongs to
    pub interior: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpeningKind {
    Door,
    Window,
}

/// One building interior (loot site).
#[derive(Debug, Clone)]
pub struct Interior {
    pub centroid: (f32, f32),
    pub pixels: usize,
}

/// Result of carving: masks the game world is built from.
pub struct Carved {
    /// blocks movement (walls minus doors; water added by the world builder)
    pub walls: Vec<bool>,
    /// blocks line of sight (walls minus doors minus windows)
    pub sight: Vec<bool>,
    pub doors: usize,
    pub windows: usize,
    pub interiors: usize,
    /// per-pixel interior component id (u32::MAX = not indoors)
    pub indoor_id: Vec<u32>,
    pub interior_list: Vec<Interior>,
    pub openings: Vec<Opening>,
}

const DOOR_HALF: i32 = 2; // carve radius → doors ≈5 px wide (nav-safe)
const WINDOW_SPACING: u64 = 6;

fn hash(x: i32, y: i32) -> u64 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^ (h >> 32)
}

/// Carve doors/windows into `g`'s wall mask, repaint carved pixels on the
/// composed canvas, and return the movement/sight masks.
pub fn carve(g: &mut Generated) -> Carved {
    let w = g.rendered.canvas.width as i32;
    let h = g.rendered.canvas.height as i32;
    let n = (w * h) as usize;
    let walls0 = g.rendered.building.clone();
    let indoor = g.rendered.indoor.clone();
    let tags = g.rendered.canvas.tags.clone();
    let at = |x: i32, y: i32| -> Option<usize> {
        if x < 0 || y < 0 || x >= w || y >= h {
            None
        } else {
            Some((y * w + x) as usize)
        }
    };
    let outdoor_open = |i: usize| -> bool { !walls0[i] && !indoor[i] && tags[i] != layer::OCEAN };

    // label interior components
    let mut label = vec![u32::MAX; n];
    let mut components: Vec<Vec<usize>> = Vec::new();
    let mut stack = Vec::new();
    for start in 0..n {
        if !indoor[start] || label[start] != u32::MAX {
            continue;
        }
        let id = components.len() as u32;
        label[start] = id;
        stack.push(start);
        let mut members = Vec::new();
        while let Some(i) = stack.pop() {
            members.push(i);
            let (x, y) = (i as i32 % w, i as i32 / w);
            for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                if let Some(j) = at(nx, ny)
                    && indoor[j]
                    && label[j] == u32::MAX
                {
                    label[j] = id;
                    stack.push(j);
                }
            }
        }
        components.push(members);
    }

    // candidate door positions per component: wall pixel with this interior on
    // one side and open outdoor ground straight across
    let mut candidates: Vec<Vec<(i32, i32, i32, i32)>> = vec![Vec::new(); components.len()];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if !walls0[i] {
                continue;
            }
            for (dx, dy) in [(1, 0), (0, 1)] {
                let (a, b) = (at(x - dx, y - dy), at(x + dx, y + dy));
                let (Some(a), Some(b)) = (a, b) else { continue };
                // 1px wall: indoor | wall | outdoor (either direction)
                let mut comp = if indoor[a] && outdoor_open(b) {
                    label[a]
                } else if indoor[b] && outdoor_open(a) {
                    label[b]
                } else {
                    u32::MAX
                };
                // 2px wall: indoor | wall | wall | outdoor
                if comp == u32::MAX {
                    let (a2, b2) = (at(x - 2 * dx, y - 2 * dy), at(x + 2 * dx, y + 2 * dy));
                    if let (Some(a2), Some(b2)) = (a2, b2) {
                        if indoor[a] && walls0[b] && outdoor_open(b2) {
                            comp = label[a];
                        } else if indoor[b] && walls0[a] && outdoor_open(a2) {
                            comp = label[b];
                        } else if walls0[a] && indoor[a2] && outdoor_open(b) {
                            comp = label[a2];
                        } else if walls0[b] && indoor[b2] && outdoor_open(a) {
                            comp = label[b2];
                        }
                    }
                }
                if comp != u32::MAX {
                    candidates[comp as usize].push((x, y, dx, dy));
                }
            }
        }
    }

    let mut walls = walls0.clone();
    let mut sight = walls0.clone();
    let mut doors = 0usize;
    let mut windows = 0usize;
    let mut door_px: Vec<usize> = Vec::new();
    let mut window_px: Vec<usize> = Vec::new();
    let mut openings: Vec<Opening> = Vec::new();

    for (comp, cands) in candidates.iter().enumerate() {
        if cands.is_empty() {
            continue;
        }
        let want = (1 + components[comp].len() / 600).min(4);
        // deterministic spread: sort by hash, take `want` far-apart picks
        let mut picks: Vec<(i32, i32, i32, i32)> = Vec::new();
        let mut sorted: Vec<_> = cands.clone();
        sorted.sort_by_key(|(x, y, _, _)| hash(*x, *y));
        for c in sorted {
            if picks.len() >= want {
                break;
            }
            if picks
                .iter()
                .all(|p| (p.0 - c.0).abs() + (p.1 - c.1).abs() > 12)
            {
                picks.push(c);
            }
        }
        for (px, py, dx, dy) in &picks {
            doors += 1;
            // carve along the wall tangent (perpendicular to the through axis)
            let (tx, ty) = (*dy, *dx);
            let mut removed = Vec::new();
            for t in -DOOR_HALF..=DOOR_HALF {
                for u in -2..=2 {
                    // small cross section to break 2px-thick corner walls too
                    if let Some(i) = at(px + tx * t + dx * u, py + ty * t + dy * u)
                        && walls[i]
                    {
                        walls[i] = false;
                        sight[i] = false;
                        door_px.push(i);
                        removed.push(i);
                    }
                }
            }
            openings.push(Opening {
                kind: OpeningKind::Door,
                centre: (*px as f32, *py as f32),
                pixels: removed,
                interior: comp as u32,
            });
        }
        // windows on remaining wall candidates, spaced out
        for (px, py, _, _) in cands {
            if let Some(i) = at(*px, *py)
                && walls[i]
                && hash(*px, *py).is_multiple_of(WINDOW_SPACING)
            {
                sight[i] = false;
                windows += 1;
                window_px.push(i);
                openings.push(Opening {
                    kind: OpeningKind::Window,
                    centre: (*px as f32, *py as f32),
                    pixels: vec![i],
                    interior: comp as u32,
                });
            }
        }
    }

    // repaint carved pixels: doors become floor, windows a pale glass tint
    for i in door_px {
        // nearest indoor neighbour colour, else a light grey
        let (x, y) = (i as i32 % w, i as i32 / w);
        let mut c = [120, 115, 105, 255];
        for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
            if let Some(j) = at(nx, ny)
                && indoor[j]
            {
                c = g.composed.pixels[j];
                break;
            }
        }
        g.composed.pixels[i] = c;
    }
    for i in window_px {
        let p = g.composed.pixels[i];
        g.composed.pixels[i] = [
            (p[0] as u16 * 2 / 3 + 60) as u8,
            (p[1] as u16 * 2 / 3 + 70) as u8,
            (p[2] as u16 * 2 / 3 + 90) as u8,
            255,
        ];
    }

    g.rendered.building = walls.clone();
    let interior_list: Vec<Interior> = components
        .iter()
        .map(|members| {
            let (mut sx, mut sy) = (0.0f64, 0.0f64);
            for &i in members {
                sx += (i as i32 % w) as f64;
                sy += (i as i32 / w) as f64;
            }
            let n = members.len().max(1) as f64;
            Interior {
                centroid: ((sx / n) as f32, (sy / n) as f32),
                pixels: members.len(),
            }
        })
        .collect();
    log::info!(
        "buildings: {} interiors, {doors} doors, {windows} windows",
        components.len()
    );
    Carved {
        walls,
        sight,
        doors,
        windows,
        interiors: components.len(),
        indoor_id: label,
        interior_list,
        openings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MapConfig;
    use crate::generate;
    use clap::Parser;

    /// tiny synthetic map: no OSM needed — build a Generated via the fixture
    /// with enterable buildings is done in tests/game.rs; here test hashing.
    #[test]
    fn hash_is_deterministic_and_spread() {
        assert_eq!(hash(3, 4), hash(3, 4));
        assert_ne!(hash(3, 4), hash(4, 3));
        let hits = (0..1000)
            .filter(|i| hash(*i, 0).is_multiple_of(WINDOW_SPACING))
            .count();
        assert!(hits > 100 && hits < 300, "{hits}");
    }

    #[test]
    fn carve_on_fixture_map_makes_doors() {
        let cfg = MapConfig::parse_from([
            "mapgenart",
            "--input",
            "assets/maps/sf.json",
            "--bbox",
            "37.780,-122.425,37.795,-122.398",
            "--width",
            "400",
            "--buildings",
            "--enterable",
            "--no-political",
            "--labels",
            "false",
            "--cities",
            "false",
        ]);
        let mut g = generate::generate(&cfg).unwrap();
        assert!(g.rendered.indoor.iter().any(|b| *b), "interiors rendered");
        let carved = carve(&mut g);
        assert!(carved.interiors > 20);
        // most interiors get a door (some are fully enclosed by neighbours)
        assert!(
            carved.doors * 10 >= carved.interiors * 7,
            "{} doors for {} interiors",
            carved.doors,
            carved.interiors
        );
        assert!(carved.windows > 100, "{} windows", carved.windows);
        // doors actually open the walls
        let opened = carved
            .walls
            .iter()
            .zip(g.rendered.indoor.iter())
            .filter(|(w, _)| !*w)
            .count();
        assert!(opened > 0);
        if std::env::var("DEBUG_CARVE").is_ok() {
            generate::save_png(&g.composed, std::path::Path::new("out/carve-debug.png")).unwrap();
        }
    }
}
