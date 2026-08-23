//! Label placement: region names at each region's pole of inaccessibility
//! (computed with a distance transform on the region-id buffer) and
//! city/town dots with names. Produces a sparse [`Overlay`] applied after
//! post-fx so smoothing never touches text.

use crate::font;
use crate::osm::{Feature, Geometry, Kind};
use crate::palette::Rgba;
use crate::raster::{Overlay, Rendered, layer};
use crate::scenario::Scenario;
use std::collections::VecDeque;

pub struct LabelOptions {
    pub regions: bool,
    pub cities: bool,
    pub colour: Rgba,
    pub metres_per_pixel: f64,
}

#[derive(Clone, Copy)]
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Rect {
    fn intersects(&self, o: &Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }
}

/// Multi-source BFS distance to the nearest pixel *outside* the region.
/// Returns, per region index, the interior pixel with the largest distance —
/// the raster pole of inaccessibility.
pub fn poles(r: &Rendered) -> Vec<Option<(i32, i32, u32)>> {
    let (w, h) = (r.canvas.width as i32, r.canvas.height as i32);
    let n = (w * h) as usize;
    let ids = &r.region_ids;
    let mut dist: Vec<u32> = vec![u32::MAX; n];
    let mut queue: VecDeque<(i32, i32)> = VecDeque::new();
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let border = ids[i] == u32::MAX
                || x == 0
                || y == 0
                || x == w - 1
                || y == h - 1
                || [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]
                    .into_iter()
                    .any(|(nx, ny)| {
                        nx >= 0
                            && ny >= 0
                            && nx < w
                            && ny < h
                            && ids[(ny * w + nx) as usize] != ids[i]
                    });
            if border {
                dist[i] = 0;
                queue.push_back((x, y));
            }
        }
    }
    while let Some((x, y)) = queue.pop_front() {
        let d = dist[(y * w + x) as usize];
        for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
            if nx < 0 || ny < 0 || nx >= w || ny >= h {
                continue;
            }
            let j = (ny * w + nx) as usize;
            if dist[j] == u32::MAX {
                dist[j] = d + 1;
                queue.push_back((nx, ny));
            }
        }
    }
    let mut best: Vec<Option<(i32, i32, u32)>> = vec![None; r.regions.len()];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let id = ids[i];
            if id == u32::MAX {
                continue;
            }
            let e = &mut best[id as usize];
            if e.is_none() || dist[i] > e.unwrap().2 {
                *e = Some((x, y, dist[i]));
            }
        }
    }
    best
}

#[allow(clippy::too_many_arguments)]
fn stamp_text(
    ov: &mut Overlay,
    placed: &mut Vec<Rect>,
    r: &Rendered,
    text: &str,
    cx: i32,
    cy: i32,
    scale: usize,
    colour: Rgba,
    forbid_water: bool,
) -> bool {
    let tw = font::text_width(text, scale) as i32;
    let th = font::text_height(scale) as i32;
    if tw == 0 {
        return false;
    }
    let (w, h) = (r.canvas.width as i32, r.canvas.height as i32);
    let rect = Rect {
        x: cx - tw / 2 - 1,
        y: cy - th / 2 - 1,
        w: tw + 2,
        h: th + 2,
    };
    if rect.x < 0 || rect.y < 0 || rect.x + rect.w > w || rect.y + rect.h > h {
        return false;
    }
    if placed.iter().any(|p| p.intersects(&rect)) {
        return false;
    }
    if forbid_water {
        let mut water = 0;
        for y in rect.y..rect.y + rect.h {
            for x in rect.x..rect.x + rect.w {
                if r.canvas.tags[(y * w + x) as usize] == layer::OCEAN {
                    water += 1;
                }
            }
        }
        if water * 5 > rect.w * rect.h {
            return false;
        }
    }
    let (ox, oy) = (cx - tw / 2, cy - th / 2);
    font::render(text, scale, |px, py| {
        let (x, y) = (ox + px as i32, oy + py as i32);
        if x >= 0 && y >= 0 && x < w && y < h {
            ov[(y * w + x) as usize] = Some(colour);
        }
    });
    placed.push(rect);
    true
}

/// Build the label overlay: region names (largest first), then city/town dots.
pub fn build(
    r: &Rendered,
    features: &[Feature],
    scenario: &Scenario,
    opts: &LabelOptions,
) -> Overlay {
    let n = r.region_ids.len();
    let mut ov: Overlay = vec![None; n];
    let mut placed: Vec<Rect> = Vec::new();

    if opts.regions && !r.regions.is_empty() {
        let poles = poles(r);
        let mut order: Vec<usize> = (0..r.regions.len()).collect();
        order.sort_by_key(|i| std::cmp::Reverse(r.regions[*i].pixels));
        for idx in order {
            let info = &r.regions[idx];
            let name = scenario
                .assignment(info.id, info.name.as_deref())
                .and_then(|a| a.label.clone())
                .or_else(|| info.name.clone());
            let Some(name) = name else { continue };
            let Some((cx, cy, _)) = poles[idx] else {
                continue;
            };
            let scale = if info.pixels > 20_000 { 2 } else { 1 };
            // try 2× then 1× then give up (region too small)
            if scale == 2
                && stamp_text(&mut ov, &mut placed, r, &name, cx, cy, 2, opts.colour, true)
            {
                continue;
            }
            let fits = font::text_width(&name, 1) * font::text_height(1) * 3 < info.pixels;
            if fits {
                stamp_text(&mut ov, &mut placed, r, &name, cx, cy, 1, opts.colour, true);
            }
        }
    }

    if opts.cities {
        let (w, h) = (r.canvas.width as i32, r.canvas.height as i32);
        let mut cities: Vec<(&Feature, [f64; 2])> = features
            .iter()
            .filter_map(|f| match (&f.kind, &f.geom) {
                (Kind::City, Geometry::Point(p)) => Some((f, *p)),
                (Kind::Town, Geometry::Point(p)) if opts.metres_per_pixel < 120.0 => Some((f, *p)),
                _ => None,
            })
            .collect();
        cities.sort_by_key(|(f, _)| (f.kind, f.id)); // cities before towns
        for (f, lonlat) in cities {
            let p = r.proj.project(lonlat);
            let (x, y) = (p[0].floor() as i32, p[1].floor() as i32);
            if x < 1 || y < 1 || x >= w - 1 || y >= h - 1 {
                continue;
            }
            // 2×2 dot
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                ov[((y + dy) * w + x + dx) as usize] = Some(opts.colour);
            }
            if let Some(name) = &f.name {
                let tw = font::text_width(name, 1) as i32;
                let th = font::text_height(1) as i32;
                stamp_text(
                    &mut ov,
                    &mut placed,
                    r,
                    name,
                    x + 3 + tw / 2,
                    y - th / 2,
                    1,
                    opts.colour,
                    false,
                );
            }
        }
    }

    ov
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BBox;
    use crate::osm::{Feature, Geometry, Kind};
    use crate::palette::Palette;
    use crate::raster::{Detail, RenderOptions, render};
    use crate::scenario::Scenario;

    fn u_shape_rendered() -> Rendered {
        // U-shaped region: the centroid falls in the notch (outside), the pole
        // of inaccessibility must be inside one of the arms / the base.
        let ring = vec![
            [0.05, 0.05],
            [0.95, 0.05],
            [0.95, 0.95],
            [0.65, 0.95],
            [0.65, 0.30],
            [0.35, 0.30],
            [0.35, 0.95],
            [0.05, 0.95],
            [0.05, 0.05],
        ];
        let feats = vec![Feature {
            kind: Kind::Region(4),
            geom: Geometry::Polygon(vec![ring]),
            id: 1,
            name: Some("U".into()),
        }];
        let pal = Palette::default();
        let scen = Scenario::default();
        let opts = RenderOptions {
            palette: &pal,
            scenario: &scen,
            detail: Detail::full(),
            political_level: Some(4),
            land: None,
            osm_borders: false,
        };
        let bbox = BBox {
            south: 0.0,
            west: 0.0,
            north: 1.0,
            east: 1.0,
        };
        render(&feats, bbox, 60, &opts)
    }

    #[test]
    fn polylabel_inside_u_shape() {
        let r = u_shape_rendered();
        let poles = poles(&r);
        let (x, y, d) = poles[0].expect("pole");
        let i = r.canvas.idx(x, y).unwrap();
        assert_eq!(r.region_ids[i], 0, "pole must be inside the region");
        assert!(d >= 2, "distance {d}");
        // the notch centre (0.5, mercator-mid of the U opening) is NOT the pole
        let notch = r.canvas.idx(30, 10).unwrap();
        assert_ne!(r.region_ids[notch], 0);
    }

    #[test]
    fn labels_do_not_overlap() {
        let r = u_shape_rendered();
        let scen = Scenario::default();
        let mut placed = Vec::new();
        let mut ov: Overlay = vec![None; r.region_ids.len()];
        assert!(stamp_text(
            &mut ov,
            &mut placed,
            &r,
            "AA",
            15,
            40,
            1,
            [0, 0, 0, 255],
            false
        ));
        // second label at the same spot must be refused
        assert!(!stamp_text(
            &mut ov,
            &mut placed,
            &r,
            "BB",
            15,
            40,
            1,
            [0, 0, 0, 255],
            false
        ));
        let opts = LabelOptions {
            regions: true,
            cities: false,
            colour: [0, 0, 0, 255],
            metres_per_pixel: 10.0,
        };
        let ov = build(&r, &[], &scen, &opts);
        assert!(ov.iter().flatten().count() > 0, "region label rendered");
    }
}
