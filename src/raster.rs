//! A tiny software rasterizer that turns OSM features into a pixel-art map.

use crate::config::BBox;
use crate::land::LandPolygons;
use crate::osm::{Feature, Geometry, Kind};
use crate::palette::{Palette, Rgba};
use crate::scenario::Scenario;
use std::collections::VecDeque;

/// Web-Mercator projection from lon/lat to pixel coordinates of a canvas.
#[derive(Debug, Clone, Copy)]
pub struct Projection {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    pub width: u32,
    pub height: u32,
}

fn merc_y(lat_deg: f64) -> f64 {
    let lat = lat_deg.clamp(-85.05, 85.05).to_radians();
    (std::f64::consts::FRAC_PI_4 + lat / 2.0).tan().ln()
}

impl Projection {
    pub fn new(bbox: BBox, width: u32) -> Self {
        let x0 = bbox.west.to_radians();
        let x1 = bbox.east.to_radians();
        let y0 = merc_y(bbox.north); // top
        let y1 = merc_y(bbox.south); // bottom
        let height = ((width as f64) * (y1 - y0).abs() / (x1 - x0))
            .round()
            .max(1.0) as u32;
        Projection {
            x0,
            y0,
            x1,
            y1,
            width,
            height,
        }
    }

    /// Inverse of [`Projection::project`]: pixel coords → lon/lat.
    pub fn unproject(&self, xy: [f64; 2]) -> [f64; 2] {
        let lon = (xy[0] / self.width as f64 * (self.x1 - self.x0) + self.x0).to_degrees();
        let my = xy[1] / self.height as f64 * (self.y1 - self.y0) + self.y0;
        let lat = (2.0 * my.exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees();
        [lon, lat]
    }

    pub fn project(&self, lonlat: [f64; 2]) -> [f64; 2] {
        let x = (lonlat[0].to_radians() - self.x0) / (self.x1 - self.x0) * self.width as f64;
        let y = (merc_y(lonlat[1]) - self.y0) / (self.y1 - self.y0) * self.height as f64;
        [x, y]
    }
}

/// Which logical layer last painted a pixel. Used by post-processing (lines
/// are never smoothed) and by the editor (only region-base pixels recolour).
pub mod layer {
    pub const LAND: u8 = 0;
    pub const OCEAN: u8 = 1;
    pub const REGION: u8 = 2;
    pub const COVER: u8 = 3;
    pub const LINE: u8 = 4;
    pub const SHORE: u8 = 5;
    pub const LABEL: u8 = 6;
}

/// RGBA8 pixel canvas with a per-pixel layer tag.
#[derive(Debug, Clone)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<Rgba>,
    pub tags: Vec<u8>,
    /// Layer tag stamped by subsequent draw calls.
    pub layer: u8,
    /// When set, draw calls leave ocean pixels untouched (used for borders).
    pub skip_ocean: bool,
}

impl Canvas {
    pub fn new(width: u32, height: u32, fill: Rgba) -> Self {
        let n = (width * height) as usize;
        Canvas {
            width,
            height,
            pixels: vec![fill; n],
            tags: vec![layer::LAND; n],
            layer: layer::LAND,
            skip_ocean: false,
        }
    }

    #[inline]
    pub fn idx(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            None
        } else {
            Some(y as usize * self.width as usize + x as usize)
        }
    }

    #[inline]
    pub fn set(&mut self, x: i32, y: i32, c: Rgba) {
        if let Some(i) = self.idx(x, y) {
            if self.skip_ocean && self.tags[i] == layer::OCEAN {
                return;
            }
            self.pixels[i] = c;
            self.tags[i] = self.layer;
        }
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32) -> Option<Rgba> {
        self.idx(x, y).map(|i| self.pixels[i])
    }

    /// Bresenham line, 8-connected, with an optional "thickness" by stamping
    /// a small square at each step.
    pub fn line(&mut self, a: [f64; 2], b: [f64; 2], c: Rgba, thickness: i32) {
        let (mut x0, mut y0) = (a[0].floor() as i32, a[1].floor() as i32);
        let (x1, y1) = (b[0].floor() as i32, b[1].floor() as i32);
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let r = (thickness - 1) / 2;
        loop {
            if thickness <= 1 {
                self.set(x0, y0, c);
            } else {
                for oy in -r..=(thickness - 1 - r) {
                    for ox in -r..=(thickness - 1 - r) {
                        self.set(x0 + ox, y0 + oy, c);
                    }
                }
            }
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    pub fn polyline(&mut self, pts: &[[f64; 2]], c: Rgba, thickness: i32) {
        for w in pts.windows(2) {
            self.line(w[0], w[1], c, thickness);
        }
    }

    /// Even-odd scanline fill over a set of rings (handles holes).
    pub fn fill_polygon(&mut self, rings: &[Vec<[f64; 2]>], c: Rgba) {
        let mut min_y = f64::MAX;
        let mut max_y = f64::MIN;
        for r in rings {
            for p in r {
                min_y = min_y.min(p[1]);
                max_y = max_y.max(p[1]);
            }
        }
        if !min_y.is_finite() {
            return;
        }
        let y_start = (min_y.floor() as i32).max(0);
        let y_end = (max_y.ceil() as i32).min(self.height as i32 - 1);
        let mut xs: Vec<f64> = Vec::new();
        for py in y_start..=y_end {
            let sy = py as f64 + 0.5;
            xs.clear();
            for r in rings {
                let n = r.len();
                if n < 2 {
                    continue;
                }
                for i in 0..n {
                    let a = r[i];
                    let b = r[(i + 1) % n];
                    if (a[1] <= sy) != (b[1] <= sy) {
                        let t = (sy - a[1]) / (b[1] - a[1]);
                        xs.push(a[0] + t * (b[0] - a[0]));
                    }
                }
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            for pair in xs.chunks(2) {
                if pair.len() < 2 {
                    break;
                }
                let xa = (pair[0].round() as i32).max(0);
                let xb = (pair[1].round() as i32).min(self.width as i32);
                for px in xa..xb {
                    self.set(px, py, c);
                }
            }
        }
    }

    /// Draw the outline of every ring (1px) – used to make sure thin polygons
    /// survive at low resolutions.
    pub fn outline_polygon(&mut self, rings: &[Vec<[f64; 2]>], c: Rgba) {
        for r in rings {
            self.polyline(r, c, 1);
        }
    }

    pub fn to_rgba_bytes(&self) -> Vec<u8> {
        self.pixels.iter().flatten().copied().collect()
    }

    /// Nearest-neighbour integer upscale.
    pub fn upscale(&self, factor: u32) -> Canvas {
        let f = factor.max(1);
        let mut out = Canvas::new(self.width * f, self.height * f, [0, 0, 0, 0]);
        for y in 0..out.height {
            for x in 0..out.width {
                let si = (y / f * self.width + x / f) as usize;
                let di = (y * out.width + x) as usize;
                out.pixels[di] = self.pixels[si];
                out.tags[di] = self.tags[si];
            }
        }
        out
    }

    /// Draw 1px grid lines every `step` pixels (for upscaled exports),
    /// alpha-blending `colour` over the existing pixels.
    pub fn draw_grid(&mut self, step: u32, colour: Rgba) {
        let step = step.max(2);
        let a = colour[3] as u32;
        for y in 0..self.height {
            for x in 0..self.width {
                if x % step == 0 || y % step == 0 {
                    let i = (y * self.width + x) as usize;
                    let p = &mut self.pixels[i];
                    for k in 0..3 {
                        p[k] = ((p[k] as u32 * (255 - a) + colour[k] as u32 * a) / 255) as u8;
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Ocean detection

/// OSM coastlines are drawn with land on the left and water on the right.
/// We rasterize the coastline into a mask, label the connected regions that
/// remain, and let every coastline chord vote: the region just to its right
/// gets a "sea" vote, the region to its left a "land" vote. Regions where sea
/// wins are painted as ocean. Voting makes the result robust against the odd
/// chord whose offset sample lands on the wrong side at sharp corners.
fn paint_ocean(canvas: &mut Canvas, proj: &Projection, coastlines: &[&[[f64; 2]]], ocean: Rgba) {
    if coastlines.is_empty() {
        return;
    }
    let (w, h) = (proj.width as i32, proj.height as i32);
    let mut coast = Canvas::new(proj.width, proj.height, [0, 0, 0, 0]);
    // (x, y, is_sea_side)
    let mut samples: Vec<(i32, i32, bool)> = Vec::new();

    for line in coastlines {
        let pts: Vec<[f64; 2]> = line.iter().map(|p| proj.project(*p)).collect();
        coast.polyline(&pts, [1, 1, 1, 255], 1);
        // Resample the way into chords of at least ~2 px so that densely
        // noded coastlines still produce samples.
        let mut i = 0;
        while i + 1 < pts.len() {
            let a = pts[i];
            let mut j = i + 1;
            let mut b = pts[j];
            let (mut dx, mut dy) = (b[0] - a[0], b[1] - a[1]);
            let mut len = (dx * dx + dy * dy).sqrt();
            while len < 2.0 && j + 1 < pts.len() {
                j += 1;
                b = pts[j];
                dx = b[0] - a[0];
                dy = b[1] - a[1];
                len = (dx * dx + dy * dy).sqrt();
            }
            i = j;
            if len < 1.0 {
                continue;
            }
            // screen space has y down, so "right of direction" is (-dy, dx)
            let nx = -dy / len;
            let ny = dx / len;
            let mx = (a[0] + b[0]) / 2.0;
            let my = (a[1] + b[1]) / 2.0;
            for (sign, is_sea) in [(1.0, true), (-1.0, false)] {
                for dist in [1.5, 2.5] {
                    let sx = (mx + sign * nx * dist).floor() as i32;
                    let sy = (my + sign * ny * dist).floor() as i32;
                    if sx >= 0 && sy >= 0 && sx < w && sy < h && coast.get(sx, sy).unwrap()[3] == 0
                    {
                        samples.push((sx, sy, is_sea));
                        break;
                    }
                }
            }
        }
    }

    // Label 4-connected regions of non-coast pixels.
    let mut label = vec![u32::MAX; (w * h) as usize];
    let mut region_count = 0u32;
    let mut queue: VecDeque<(i32, i32)> = VecDeque::new();
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if label[i] != u32::MAX || coast.pixels[i][3] != 0 {
                continue;
            }
            let id = region_count;
            region_count += 1;
            label[i] = id;
            queue.push_back((x, y));
            while let Some((cx, cy)) = queue.pop_front() {
                for (nx, ny) in [(cx + 1, cy), (cx - 1, cy), (cx, cy + 1), (cx, cy - 1)] {
                    if nx < 0 || ny < 0 || nx >= w || ny >= h {
                        continue;
                    }
                    let ni = (ny * w + nx) as usize;
                    if label[ni] != u32::MAX || coast.pixels[ni][3] != 0 {
                        continue;
                    }
                    label[ni] = id;
                    queue.push_back((nx, ny));
                }
            }
        }
    }

    let mut votes = vec![0i32; region_count as usize];
    for (x, y, is_sea) in &samples {
        let id = label[(*y * w + *x) as usize];
        if id != u32::MAX {
            votes[id as usize] += if *is_sea { 1 } else { -1 };
        }
    }
    log::info!(
        "ocean fill: {} coastline ways, {} samples, {} regions, {} sea",
        coastlines.len(),
        samples.len(),
        region_count,
        votes.iter().filter(|v| **v > 0).count()
    );

    for (i, &id) in label.iter().enumerate() {
        if id != u32::MAX && votes[id as usize] > 0 {
            canvas.pixels[i] = ocean;
            canvas.tags[i] = layer::OCEAN;
        }
    }
    // Coast pixels themselves stay land, which reads as a thin shore at
    // pixel-art scale (`postfx::shoreline` can darken it).
}

/// Land/ocean base from external land polygons: start as ocean, fill land.
fn paint_land_polygons(
    canvas: &mut Canvas,
    proj: &Projection,
    land: &LandPolygons,
    land_colour: Rgba,
) {
    canvas.layer = layer::LAND;
    for poly in land {
        let rings: Vec<Vec<[f64; 2]>> = poly
            .iter()
            .map(|r| r.iter().map(|p| proj.project(*p)).collect())
            .collect();
        // cheap reject: skip polygons entirely outside the canvas
        let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for r in &rings {
            for p in r {
                minx = minx.min(p[0]);
                miny = miny.min(p[1]);
                maxx = maxx.max(p[0]);
                maxy = maxy.max(p[1]);
            }
        }
        if maxx < 0.0 || maxy < 0.0 || minx > canvas.width as f64 || miny > canvas.height as f64 {
            continue;
        }
        canvas.fill_polygon(&rings, land_colour);
    }
}

// ---------------------------------------------------------------------------
// Main render

fn ring_area(ring: &[[f64; 2]]) -> f64 {
    let n = ring.len();
    let mut a = 0.0;
    for i in 0..n {
        let p = ring[i];
        let q = ring[(i + 1) % n];
        a += p[0] * q[1] - q[0] * p[1];
    }
    a.abs() / 2.0
}

fn fill_colour(pal: &Palette, kind: Kind) -> Option<Rgba> {
    Some(match kind {
        Kind::Farmland => pal.farmland,
        Kind::Urban => pal.urban,
        Kind::Industrial => pal.industrial,
        Kind::Grass => pal.grass,
        Kind::Forest => pal.forest,
        Kind::Sand => pal.sand,
        Kind::Wetland => pal.wetland,
        Kind::Water => pal.lake,
        Kind::Building => pal.building,
        _ => return None,
    })
}

fn line_style(pal: &Palette, kind: Kind) -> Option<(Rgba, i32)> {
    Some(match kind {
        Kind::River => (pal.river, 1),
        Kind::Stream => (pal.river, 1),
        Kind::Rail => (pal.rail, 1),
        Kind::RoadMinor => (pal.road_minor, 1),
        Kind::RoadMajor => (pal.road_major, 1),
        Kind::BorderLocal => (pal.border_local, 1),
        Kind::BorderRegion => (pal.border_region, 1),
        Kind::BorderCountry => (pal.border_country, 2),
        _ => return None,
    })
}

/// Which feature classes to draw at a given map scale. Wide maps drop fine
/// detail automatically so they stay readable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Detail {
    pub streams: bool,
    pub minor_roads: bool,
    pub major_roads: bool,
    pub rail: bool,
    pub buildings: bool,
    pub local_borders: bool,
}

impl Detail {
    pub fn for_scale(metres_per_pixel: f64) -> Self {
        Detail {
            streams: metres_per_pixel < 25.0,
            minor_roads: metres_per_pixel < 60.0,
            major_roads: metres_per_pixel < 250.0,
            rail: metres_per_pixel < 120.0,
            buildings: metres_per_pixel < 12.0,
            local_borders: metres_per_pixel < 150.0,
        }
    }

    pub fn full() -> Self {
        Detail {
            streams: true,
            minor_roads: true,
            major_roads: true,
            rail: true,
            buildings: true,
            local_borders: true,
        }
    }

    fn allows(&self, kind: Kind) -> bool {
        match kind {
            Kind::Stream => self.streams,
            Kind::RoadMinor => self.minor_roads,
            Kind::RoadMajor => self.major_roads,
            Kind::Rail => self.rail,
            Kind::Building => self.buildings,
            Kind::BorderLocal => self.local_borders,
            _ => true,
        }
    }
}

/// A political region that was drawn, for the editor / legend.
#[derive(Debug, Clone, PartialEq)]
pub struct RegionInfo {
    pub id: i64,
    pub name: Option<String>,
    pub admin_level: u8,
    pub colour: Rgba,
    pub pixels: usize,
}

/// Everything `render` produces.
#[derive(Debug, Clone)]
pub struct Rendered {
    pub canvas: Canvas,
    /// Index into `regions` per pixel, `u32::MAX` where no region.
    pub region_ids: Vec<u32>,
    pub regions: Vec<RegionInfo>,
    pub admin_level_used: Option<u8>,
    pub proj: Projection,
    /// Building-footprint pixels (blocks movement and line of sight).
    pub building: Vec<bool>,
}

impl Rendered {
    /// Pixels that block unit movement: buildings and water.
    pub fn blocked(&self) -> Vec<bool> {
        self.building
            .iter()
            .zip(&self.canvas.tags)
            .map(|(b, t)| *b || *t == layer::OCEAN)
            .collect()
    }
}

/// A sparse pixel overlay (derived borders, labels, selection outlines).
pub type Overlay = Vec<Option<Rgba>>;

/// Apply overlays (in order) over a copy of the canvas. Overlay pixels are
/// tagged with `tag` so post-processing can recognise them.
pub fn compose(canvas: &Canvas, overlays: &[(&Overlay, u8)]) -> Canvas {
    let mut out = canvas.clone();
    for (ov, tag) in overlays {
        for (i, c) in ov.iter().enumerate() {
            if let Some(c) = c {
                out.pixels[i] = *c;
                out.tags[i] = *tag;
            }
        }
    }
    out
}

/// Derive political borders from the region-id buffer and an owner index per
/// region (`u32::MAX` = unowned). Different owners ⇒ country style (2 px);
/// same owner (or both unowned) ⇒ region style (1 px) when `inner` is set.
/// No borders against the ocean.
pub fn derive_owner_borders(r: &Rendered, owner_of: &[u32], pal: &Palette, inner: bool) -> Overlay {
    let mut ov: Overlay = vec![None; r.region_ids.len()];
    let (w, h) = (r.canvas.width as i32, r.canvas.height as i32);
    let ids = &r.region_ids;
    let tags = &r.canvas.tags;
    let owner = |rid: u32| -> Option<u32> {
        if rid == u32::MAX {
            None
        } else {
            Some(owner_of[rid as usize])
        }
    };
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if tags[i] == layer::OCEAN {
                continue;
            }
            let a = ids[i];
            for (nx, ny) in [(x + 1, y), (x, y + 1)] {
                if nx >= w || ny >= h {
                    continue;
                }
                let j = (ny * w + nx) as usize;
                if tags[j] == layer::OCEAN {
                    continue;
                }
                let b = ids[j];
                if a == b {
                    continue;
                }
                match (owner(a), owner(b)) {
                    // between two regions with different ownership (one side
                    // owned and the other not counts too) → country border
                    (Some(oa), Some(ob)) if oa != ob => {
                        ov[i] = Some(pal.border_country);
                        ov[j] = Some(pal.border_country);
                    }
                    // same owner / both unowned / region vs unregioned land
                    _ if inner => ov[i] = Some(pal.border_region),
                    _ => {}
                }
            }
        }
    }
    ov
}

pub struct RenderOptions<'a> {
    pub palette: &'a Palette,
    pub scenario: &'a Scenario,
    pub detail: Detail,
    /// Political fills at this admin level (falls back to 2); `None` disables.
    pub political_level: Option<u8>,
    pub land: Option<&'a LandPolygons>,
    /// Draw OSM admin-boundary lines even for the level that has polygon
    /// fills (otherwise those are replaced by derived owner borders).
    pub osm_borders: bool,
}

/// Render features into a new canvas.
pub fn render(features: &[Feature], bbox: BBox, width: u32, opts: &RenderOptions) -> Rendered {
    let pal = opts.palette;
    let proj = Projection::new(bbox, width);
    let npx = (proj.width * proj.height) as usize;
    let mut region_ids = vec![u32::MAX; npx];
    let mut regions: Vec<RegionInfo> = Vec::new();
    let mut building = vec![false; npx];

    // 1. land / ocean base
    let mut canvas = match opts.land {
        Some(land) => {
            let mut c = Canvas::new(proj.width, proj.height, pal.ocean);
            c.tags.fill(layer::OCEAN);
            paint_land_polygons(&mut c, &proj, land, pal.land);
            c
        }
        None => {
            let mut c = Canvas::new(proj.width, proj.height, pal.land);
            let coastlines: Vec<&[[f64; 2]]> = features
                .iter()
                .filter_map(|f| match (&f.kind, &f.geom) {
                    (Kind::Coastline, Geometry::Line(pts)) => Some(pts.as_slice()),
                    _ => None,
                })
                .collect();
            paint_ocean(&mut c, &proj, &coastlines, pal.ocean);
            c
        }
    };

    // 2. political fills (only over land pixels)
    let mut admin_level_used = None;
    if let Some(requested) = opts.political_level {
        let available = |lvl: u8| features.iter().any(|f| f.kind == Kind::Region(lvl));
        let level = if available(requested) {
            Some(requested)
        } else if requested != 2 && available(2) {
            log::info!("no admin_level={requested} relations; falling back to admin_level=2");
            Some(2)
        } else {
            None
        };
        if let Some(level) = level {
            admin_level_used = Some(level);
            canvas.layer = layer::REGION;
            let mut polys: Vec<(&Feature, Vec<Vec<[f64; 2]>>, f64)> = features
                .iter()
                .filter(|f| f.kind == Kind::Region(level))
                .filter_map(|f| match &f.geom {
                    Geometry::Polygon(rings) => {
                        let pr: Vec<Vec<[f64; 2]>> = rings
                            .iter()
                            .map(|r| r.iter().map(|p| proj.project(*p)).collect())
                            .collect();
                        let area = pr.iter().map(|r| ring_area(r)).sum::<f64>();
                        Some((f, pr, area))
                    }
                    _ => None,
                })
                .collect();
            polys.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
            for (f, rings, _) in polys {
                let colour = opts.scenario.colour_for(f.id, f.name.as_deref());
                let idx = regions.len() as u32;
                fill_polygon_with(&mut canvas, &rings, |c, i| {
                    // keep water
                    if c.tags[i] != layer::OCEAN {
                        c.pixels[i] = colour;
                        c.tags[i] = layer::REGION;
                        region_ids[i] = idx;
                    }
                });
                regions.push(RegionInfo {
                    id: f.id,
                    name: f.name.clone(),
                    admin_level: level,
                    colour,
                    pixels: 0,
                });
            }
            for id in &region_ids {
                if *id != u32::MAX {
                    regions[*id as usize].pixels += 1;
                }
            }
        }
    }

    // 3. area fills, grouped by kind in enum order; largest first within a kind
    let mut areas: Vec<(&Feature, Vec<Vec<[f64; 2]>>, f64)> = features
        .iter()
        .filter(|f| opts.detail.allows(f.kind))
        .filter_map(|f| match &f.geom {
            Geometry::Polygon(rings) if fill_colour(pal, f.kind).is_some() => {
                let projected: Vec<Vec<[f64; 2]>> = rings
                    .iter()
                    .map(|r| r.iter().map(|p| proj.project(*p)).collect())
                    .collect();
                let area = projected.iter().map(|r| ring_area(r)).sum::<f64>();
                Some((f, projected, area))
            }
            _ => None,
        })
        .collect();
    areas.sort_by(|a, b| a.0.kind.cmp(&b.0.kind).then(b.2.partial_cmp(&a.2).unwrap()));
    for (f, rings, area) in &areas {
        let c = fill_colour(pal, f.kind).unwrap();
        if f.kind == Kind::Water {
            canvas.layer = layer::OCEAN;
            canvas.fill_polygon(rings, c);
        } else {
            let is_building = f.kind == Kind::Building;
            // land cover never paints over the sea (nature reserves, ports and
            // the like often extend into open water)
            fill_polygon_with(&mut canvas, rings, |cv, i| {
                if cv.tags[i] != layer::OCEAN {
                    cv.pixels[i] = c;
                    cv.tags[i] = layer::COVER;
                    if is_building {
                        building[i] = true;
                    }
                }
            });
        }
        // tiny features would vanish; give water & buildings at least an outline
        if *area < 4.0 && matches!(f.kind, Kind::Water | Kind::Building) {
            canvas.layer = if f.kind == Kind::Water {
                layer::OCEAN
            } else {
                layer::COVER
            };
            canvas.outline_polygon(rings, c);
        }
    }

    // 4. lines in enum order (rivers under roads under borders)
    canvas.layer = layer::LINE;
    // The admin level that has polygon fills gets derived owner borders
    // instead of its OSM boundary lines (unless explicitly requested).
    let replaced_border_kind = match admin_level_used {
        Some(l) if !opts.osm_borders => Some(match l {
            0..=3 => Kind::BorderCountry,
            4 => Kind::BorderRegion,
            _ => Kind::BorderLocal,
        }),
        _ => None,
    };
    let mut lines: Vec<&Feature> = features
        .iter()
        .filter(|f| matches!(f.geom, Geometry::Line(_)) && line_style(pal, f.kind).is_some())
        .filter(|f| opts.detail.allows(f.kind))
        .filter(|f| Some(f.kind) != replaced_border_kind)
        .collect();
    lines.sort_by_key(|f| f.kind);
    for f in lines {
        if let Geometry::Line(pts) = &f.geom {
            let (c, t) = line_style(pal, f.kind).unwrap();
            // maritime boundaries are not drawn (Q-BAM convention)
            canvas.skip_ocean = matches!(
                f.kind,
                Kind::BorderCountry | Kind::BorderRegion | Kind::BorderLocal
            );
            let projected: Vec<[f64; 2]> = pts.iter().map(|p| proj.project(*p)).collect();
            canvas.polyline(&projected, c, t);
        }
    }
    canvas.skip_ocean = false;

    Rendered {
        canvas,
        region_ids,
        regions,
        admin_level_used,
        proj,
        building,
    }
}

/// Even-odd scanline fill calling `paint(canvas, index)` for every covered pixel.
fn fill_polygon_with(
    canvas: &mut Canvas,
    rings: &[Vec<[f64; 2]>],
    mut paint: impl FnMut(&mut Canvas, usize),
) {
    let mut min_y = f64::MAX;
    let mut max_y = f64::MIN;
    for r in rings {
        for p in r {
            min_y = min_y.min(p[1]);
            max_y = max_y.max(p[1]);
        }
    }
    if !min_y.is_finite() {
        return;
    }
    let y_start = (min_y.floor() as i32).max(0);
    let y_end = (max_y.ceil() as i32).min(canvas.height as i32 - 1);
    let mut xs: Vec<f64> = Vec::new();
    for py in y_start..=y_end {
        let sy = py as f64 + 0.5;
        xs.clear();
        for r in rings {
            let n = r.len();
            if n < 2 {
                continue;
            }
            for i in 0..n {
                let a = r[i];
                let b = r[(i + 1) % n];
                if (a[1] <= sy) != (b[1] <= sy) {
                    let t = (sy - a[1]) / (b[1] - a[1]);
                    xs.push(a[0] + t * (b[0] - a[0]));
                }
            }
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for pair in xs.chunks(2) {
            if pair.len() < 2 {
                break;
            }
            let xa = (pair[0].round() as i32).max(0);
            let xb = (pair[1].round() as i32).min(canvas.width as i32);
            for px in xa..xb {
                if let Some(i) = canvas.idx(px, py) {
                    paint(canvas, i);
                }
            }
        }
    }
}

/// Recolour every base pixel of region `idx` (used by the editor; cheap).
pub fn recolour_region(r: &mut Rendered, idx: usize, colour: Rgba) {
    if idx >= r.regions.len() {
        return;
    }
    for (i, id) in r.region_ids.iter().enumerate() {
        if *id as usize == idx && r.canvas.tags[i] == layer::REGION {
            r.canvas.pixels[i] = colour;
        }
    }
    r.regions[idx].colour = colour;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::Palette;

    #[test]
    fn fill_square_even_odd_hole() {
        let mut c = Canvas::new(10, 10, [0, 0, 0, 255]);
        let outer = vec![[1.0, 1.0], [9.0, 1.0], [9.0, 9.0], [1.0, 9.0], [1.0, 1.0]];
        let inner = vec![[4.0, 4.0], [6.0, 4.0], [6.0, 6.0], [4.0, 6.0], [4.0, 4.0]];
        c.fill_polygon(&[outer, inner], [255, 0, 0, 255]);
        assert_eq!(c.get(2, 2).unwrap(), [255, 0, 0, 255]);
        assert_eq!(c.get(5, 5).unwrap(), [0, 0, 0, 255]);
        assert_eq!(c.get(0, 0).unwrap(), [0, 0, 0, 255]);
    }

    #[test]
    fn ocean_is_right_of_coastline() {
        let pal = Palette::default();
        let bbox = BBox {
            south: 0.0,
            west: 0.0,
            north: 1.0,
            east: 1.0,
        };
        let proj = Projection::new(bbox, 20);
        let mut canvas = Canvas::new(proj.width, proj.height, pal.land);
        // coastline running north→south through the middle: walking south, the
        // right hand (water) points west, land is east.
        let coast = vec![[0.5, 1.0], [0.5, 0.0]];
        paint_ocean(&mut canvas, &proj, &[coast.as_slice()], pal.ocean);
        assert_eq!(canvas.get(2, 10).unwrap(), pal.ocean);
        assert_eq!(canvas.get(17, 10).unwrap(), pal.land);
    }

    #[test]
    fn region_fill_and_recolour() {
        let pal = Palette::default();
        let bbox = BBox {
            south: 0.0,
            west: 0.0,
            north: 1.0,
            east: 1.0,
        };
        let square = vec![vec![
            [0.1, 0.1],
            [0.9, 0.1],
            [0.9, 0.9],
            [0.1, 0.9],
            [0.1, 0.1],
        ]];
        let feats = vec![Feature {
            kind: Kind::Region(4),
            geom: Geometry::Polygon(square),
            id: 7,
            name: Some("Test".into()),
        }];
        let scen = Scenario::default();
        let opts = RenderOptions {
            palette: &pal,
            scenario: &scen,
            detail: Detail::full(),
            political_level: Some(4),
            land: None,
            osm_borders: false,
        };
        let mut r = render(&feats, bbox, 20, &opts);
        assert_eq!(r.regions.len(), 1);
        assert_eq!(r.admin_level_used, Some(4));
        let centre = r.canvas.idx(10, 10).unwrap();
        assert_eq!(r.region_ids[centre], 0);
        assert_eq!(r.canvas.pixels[centre], Palette::region_colour(7));
        recolour_region(&mut r, 0, [1, 2, 3, 255]);
        assert_eq!(r.canvas.pixels[centre], [1, 2, 3, 255]);
        assert_eq!(r.canvas.tags[r.canvas.idx(0, 0).unwrap()], layer::LAND);
    }

    #[test]
    fn land_polygons_base() {
        let pal = Palette::default();
        let bbox = BBox {
            south: 0.0,
            west: 0.0,
            north: 1.0,
            east: 1.0,
        };
        let land: LandPolygons = vec![vec![vec![
            [0.0, 0.0],
            [0.5, 0.0],
            [0.5, 1.0],
            [0.0, 1.0],
            [0.0, 0.0],
        ]]];
        let scen = Scenario::default();
        let opts = RenderOptions {
            palette: &pal,
            scenario: &scen,
            detail: Detail::full(),
            political_level: None,
            land: Some(&land),
            osm_borders: false,
        };
        let r = render(&[], bbox, 20, &opts);
        assert_eq!(r.canvas.get(2, 10).unwrap(), pal.land);
        assert_eq!(r.canvas.get(17, 10).unwrap(), pal.ocean);
    }

    #[test]
    fn detail_drops_fine_features_at_wide_scale() {
        assert!(Detail::for_scale(5.0).streams);
        assert!(!Detail::for_scale(500.0).minor_roads);
        assert!(Detail::for_scale(500.0).allows(Kind::River));
    }
}
