//! Fetching OpenStreetMap data through the Overpass API and turning it into
//! simple, tagged geometry (lines and polygons in lon/lat) for the rasterizer.

use crate::config::{BBox, MapConfig};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// A classified map feature. `Polygon` rings are filled with the even-odd
/// rule, so multipolygon inner rings (islands in lakes, courtyards) just work.
#[derive(Debug, Clone)]
pub enum Geometry {
    Point([f64; 2]),
    Line(Vec<[f64; 2]>),
    Polygon(Vec<Vec<[f64; 2]>>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// Political region (admin boundary relation) at the given admin level.
    /// Drawn first so land-cover and lines sit on top.
    Region(u8),
    // area fills (drawn in this order, large → small within a kind)
    Farmland,
    Urban,
    Industrial,
    Grass,
    Forest,
    Sand,
    Wetland,
    Water,
    Building,
    // lines
    Coastline,
    River,
    Stream,
    Rail,
    RoadMinor,
    RoadMajor,
    BorderLocal,
    BorderRegion,
    BorderCountry,
    // labelled points
    City,
    Town,
    /// Supply-drop point of interest (hospital, supermarket, pharmacy).
    Poi,
}

#[derive(Debug, Clone)]
pub struct Feature {
    pub kind: Kind,
    pub geom: Geometry,
    /// OSM id of the originating way/relation.
    pub id: i64,
    /// `name` tag, when present (used for regions).
    pub name: Option<String>,
}

impl Feature {
    fn new(kind: Kind, geom: Geometry, id: i64, tags: &HashMap<String, String>) -> Self {
        Feature {
            kind,
            geom,
            id,
            name: tags.get("name").cloned(),
        }
    }
}

// ---------------------------------------------------------------------------
// Overpass JSON model

#[derive(Deserialize)]
struct OverpassResponse {
    elements: Vec<Element>,
}

#[derive(Deserialize)]
struct Element {
    #[serde(rename = "type")]
    ty: String,
    id: i64,
    #[serde(default)]
    lat: Option<f64>,
    #[serde(default)]
    lon: Option<f64>,
    #[serde(default)]
    nodes: Vec<i64>,
    #[serde(default)]
    members: Vec<Member>,
    #[serde(default)]
    tags: HashMap<String, String>,
}

#[derive(Deserialize)]
struct Member {
    #[serde(rename = "type")]
    ty: String,
    #[serde(rename = "ref")]
    reference: i64,
    #[serde(default)]
    role: String,
}

// ---------------------------------------------------------------------------
// Query

/// Build the Overpass query for one tile. `metres_per_pixel` (of the whole
/// map) trims the query for wide maps: no roads/land-use at continent scale,
/// where they could not be drawn anyway and would make the request enormous.
pub fn build_query(bbox: &BBox, cfg: &MapConfig, metres_per_pixel: f64) -> String {
    let b = bbox.overpass();
    let detail = crate::raster::Detail::for_scale(metres_per_pixel);
    let landcover = metres_per_pixel < 300.0;
    let mut q = String::new();
    q.push_str("[out:json][timeout:180];(\n");
    let mut line = |s: &str| {
        q.push_str(&s.replace("{{bbox}}", &b));
        q.push('\n');
    };
    line(r#"way["natural"="coastline"]({{bbox}});"#);
    line(r#"relation["natural"="water"]({{bbox}});"#);
    line(r#"way["waterway"~"^(river|canal)$"]({{bbox}});"#);
    if landcover {
        line(r#"way["natural"="water"]({{bbox}});"#);
        line(r#"way["waterway"~"^(riverbank|dock)$"]({{bbox}});"#);
        line(r#"relation["waterway"~"^(riverbank|dock)$"]({{bbox}});"#);
        line(
            r#"way["landuse"~"^(forest|residential|industrial|commercial|retail|farmland|farmyard|orchard|vineyard|grass|meadow|cemetery|allotments|port)$"]({{bbox}});"#,
        );
        line(
            r#"relation["landuse"~"^(forest|residential|industrial|farmland|meadow)$"]({{bbox}});"#,
        );
        line(
            r#"way["natural"~"^(wood|sand|beach|wetland|grassland|scrub|heath|shoal)$"]({{bbox}});"#,
        );
        line(r#"relation["natural"~"^(wood|wetland|beach)$"]({{bbox}});"#);
        line(r#"way["leisure"~"^(park|golf_course)$"]({{bbox}});"#);
    }
    if detail.streams {
        line(r#"way["waterway"="stream"]({{bbox}});"#);
    }
    if !cfg.no_roads {
        if detail.minor_roads {
            line(r#"way["highway"~"^(motorway|trunk|primary|secondary|tertiary)$"]({{bbox}});"#);
        } else if detail.major_roads {
            line(r#"way["highway"~"^(motorway|trunk|primary)$"]({{bbox}});"#);
        }
        if detail.rail {
            line(r#"way["railway"="rail"]({{bbox}});"#);
        }
    }
    if cfg.buildings && detail.buildings {
        line(r#"way["building"]({{bbox}});"#);
    }
    if metres_per_pixel < 800.0 {
        line(r#"node["place"="city"]({{bbox}});"#);
    }
    if metres_per_pixel < 120.0 {
        line(r#"node["place"="town"]({{bbox}});"#);
        line(r#"node["amenity"~"^(hospital|pharmacy)$"]({{bbox}});"#);
        line(r#"node["shop"="supermarket"]({{bbox}});"#);
    }
    let levels = if detail.local_borders {
        "^(2|3|4|6|8)$"
    } else {
        "^(2|3|4)$"
    };
    line(&format!(
        r#"relation["boundary"="administrative"]["admin_level"~"{levels}"]({{{{bbox}}}});"#
    ));
    q.push_str(");\nout body;\n>;\nout skel qt;");
    q
}

fn cache_path(cfg: &MapConfig, query: &str) -> PathBuf {
    // cheap content hash so different queries/bboxes get different files
    let mut h: u64 = 0xcbf29ce484222325;
    for b in query.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    cfg.cache_dir.join(format!("overpass-{h:016x}.json"))
}

/// Progress callback used by the pipeline to report status messages.
pub type Progress<'a> = &'a (dyn Fn(String) + Sync);

/// Fetch raw Overpass JSON for every tile of the bbox. Returns one JSON string
/// per tile (from `--input`, the cache, or the network).
pub fn load_tiles(cfg: &MapConfig, bbox: &BBox, progress: Progress) -> Result<Vec<String>> {
    #[cfg(target_arch = "wasm32")]
    {
        // No filesystem or blocking HTTP on the web build: render the bundled
        // demo fixture (central Copenhagen harbour).
        let _ = (bbox, &cfg.input);
        progress("Loading bundled demo data …".into());
        return Ok(vec![
            include_str!("../tests/fixtures/small.json").to_string(),
        ]);
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(path) = &cfg.input {
        progress(format!("reading {}", path.display()));
        let s = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        return Ok(vec![s]);
    }
    let tiles = bbox.tiles(cfg.tiles);
    let mpp = bbox.metres_per_pixel(cfg.width.max(8));
    let n = tiles.len();
    let mut out = Vec::with_capacity(n);
    for (i, tile) in tiles.iter().enumerate() {
        progress(format!("Fetching tile {}/{n} …", i + 1));
        let query = build_query(tile, cfg, mpp);
        out.push(load_raw(cfg, &query)?);
    }
    Ok(out)
}

/// Get raw Overpass JSON for one query: from the on-disk cache or the network.
pub fn load_raw(cfg: &MapConfig, query: &str) -> Result<String> {
    let cache = cache_path(cfg, query);
    if !cfg.no_cache && cache.exists() {
        log::info!("using cached Overpass response {}", cache.display());
        return Ok(fs::read_to_string(&cache)?);
    }
    let body = fetch_with_retry(&cfg.overpass, query)?;
    if let Some(parent) = cache.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Err(e) = fs::write(&cache, &body) {
        log::warn!("could not write cache {}: {e}", cache.display());
    }
    Ok(body)
}

/// Retry on Overpass rate limiting (429) and gateway timeouts (504) with
/// increasing backoff.
fn fetch_with_retry(endpoint: &str, query: &str) -> Result<String> {
    const BACKOFF_SECS: [u64; 3] = [5, 15, 45];
    let mut attempt = 0;
    loop {
        match fetch(endpoint, query) {
            Ok(body) => return Ok(body),
            Err(FetchError::Retryable(status)) if attempt < BACKOFF_SECS.len() => {
                let wait = BACKOFF_SECS[attempt];
                log::warn!("Overpass returned {status}; retrying in {wait}s");
                std::thread::sleep(std::time::Duration::from_secs(wait));
                attempt += 1;
            }
            Err(FetchError::Retryable(status)) => {
                anyhow::bail!(
                    "Overpass kept returning {status} after {} attempts",
                    attempt + 1
                )
            }
            Err(FetchError::Other(e)) => return Err(e),
        }
    }
}

enum FetchError {
    Retryable(u16),
    Other(anyhow::Error),
}

#[cfg(not(target_arch = "wasm32"))]
fn fetch(endpoint: &str, query: &str) -> std::result::Result<String, FetchError> {
    use std::io::Read;
    log::info!("querying Overpass at {endpoint} …");
    let started = std::time::Instant::now();
    let resp = match ureq::post(endpoint)
        .timeout(std::time::Duration::from_secs(240))
        .send_form(&[("data", query)])
    {
        Ok(r) => r,
        Err(ureq::Error::Status(code @ (429 | 504 | 502 | 503), _)) => {
            return Err(FetchError::Retryable(code));
        }
        Err(e) => {
            return Err(FetchError::Other(
                anyhow::Error::new(e).context("Overpass request failed"),
            ));
        }
    };
    let mut body = String::new();
    resp.into_reader().read_to_string(&mut body).map_err(|e| {
        FetchError::Other(anyhow::Error::new(e).context("reading Overpass response"))
    })?;
    log::info!(
        "received {:.1} MB in {:.1}s",
        body.len() as f64 / 1e6,
        started.elapsed().as_secs_f64()
    );
    Ok(body)
}

#[cfg(target_arch = "wasm32")]
fn fetch(_endpoint: &str, _query: &str) -> std::result::Result<String, FetchError> {
    Err(FetchError::Other(anyhow::anyhow!(
        "network fetching is not supported on wasm; pass --input <file.json>"
    )))
}

// ---------------------------------------------------------------------------
// Parsing + classification

pub fn parse(json: &str) -> Result<Vec<Feature>> {
    parse_many(std::slice::from_ref(&json.to_string()))
}

/// Parse one or more Overpass responses (e.g. tiles) into features. Elements
/// are de-duplicated by id across responses; tagged copies win over the bare
/// skeletons that `out skel` re-emits.
pub fn parse_many(jsons: &[String]) -> Result<Vec<Feature>> {
    let mut responses: Vec<OverpassResponse> = Vec::with_capacity(jsons.len());
    for j in jsons {
        responses.push(serde_json::from_str(j).context("parsing Overpass JSON")?);
    }

    let mut nodes: HashMap<i64, [f64; 2]> = HashMap::new();
    let mut ways: HashMap<i64, &Element> = HashMap::new();
    let mut relations: HashMap<i64, &Element> = HashMap::new();
    let mut points: HashMap<i64, &Element> = HashMap::new();
    for e in responses.iter().flat_map(|r| r.elements.iter()) {
        match e.ty.as_str() {
            "node" => {
                if let (Some(lat), Some(lon)) = (e.lat, e.lon) {
                    nodes.insert(e.id, [lon, lat]);
                    if !e.tags.is_empty() {
                        points.insert(e.id, e);
                    }
                }
            }
            "way" => {
                let keep = ways.get(&e.id).is_none_or(|prev| prev.tags.is_empty());
                if keep {
                    ways.insert(e.id, e);
                }
            }
            "relation" => {
                relations.insert(e.id, e);
            }
            _ => {}
        }
    }
    let mut relations: Vec<&Element> = relations.into_values().collect();
    relations.sort_by_key(|r| r.id);

    let way_coords = |w: &Element| -> Vec<[f64; 2]> {
        w.nodes
            .iter()
            .filter_map(|n| nodes.get(n).copied())
            .collect()
    };

    let mut out = Vec::new();

    let mut point_els: Vec<&Element> = points.into_values().collect();
    point_els.sort_by_key(|e| e.id);
    for e in point_els {
        let kind = match e.tags.get("place").map(String::as_str) {
            Some("city") => Kind::City,
            Some("town") => Kind::Town,
            _ => match (
                e.tags.get("amenity").map(String::as_str),
                e.tags.get("shop").map(String::as_str),
            ) {
                (Some("hospital" | "pharmacy"), _) | (_, Some("supermarket")) => Kind::Poi,
                _ => continue,
            },
        };
        if e.tags.contains_key("name") {
            out.push(Feature::new(
                kind,
                Geometry::Point([e.lon.unwrap(), e.lat.unwrap()]),
                e.id,
                &e.tags,
            ));
        }
    }

    for w in ways.values() {
        if w.tags.is_empty() {
            continue;
        }
        let coords = way_coords(w);
        if coords.len() < 2 {
            continue;
        }
        let closed = coords.first() == coords.last() && coords.len() >= 4;
        if let Some(kind) = classify_area(&w.tags)
            && closed
        {
            out.push(Feature::new(
                kind,
                Geometry::Polygon(vec![coords.clone()]),
                w.id,
                &w.tags,
            ));
            continue;
        }
        if let Some(kind) = classify_line(&w.tags) {
            out.push(Feature::new(kind, Geometry::Line(coords), w.id, &w.tags));
        }
    }

    let member_segments = |r: &Element, roles: &[&str]| -> Vec<Vec<[f64; 2]>> {
        let mut segments = Vec::new();
        for m in &r.members {
            if m.ty == "way"
                && (roles.contains(&m.role.as_str()) || m.role.is_empty())
                && let Some(w) = ways.get(&m.reference)
            {
                let c = way_coords(w);
                if c.len() >= 2 {
                    segments.push(c);
                }
            }
        }
        segments
    };

    for r in relations {
        if let Some((kind, level)) = classify_border(&r.tags) {
            // border lines
            for m in &r.members {
                if m.ty == "way"
                    && let Some(w) = ways.get(&m.reference)
                {
                    // borders that run along the shore are implied by the coastline
                    if w.tags.get("natural").map(String::as_str) == Some("coastline") {
                        continue;
                    }
                    let coords = way_coords(w);
                    if coords.len() >= 2 {
                        out.push(Feature::new(kind, Geometry::Line(coords), r.id, &r.tags));
                    }
                }
            }
            // political polygon
            let expected = r.members.iter().filter(|m| m.ty == "way").count();
            let segments = member_segments(r, &["outer", "inner"]);
            if segments.len() < expected {
                log::warn!(
                    "relation {} ({}): {} of {} member ways missing; ring may be force-closed",
                    r.id,
                    r.tags.get("name").map(String::as_str).unwrap_or("?"),
                    expected - segments.len(),
                    expected
                );
            }
            let (rings, forced) = assemble_rings_report(segments);
            if forced > 0 {
                log::warn!("relation {}: {forced} ring(s) force-closed", r.id);
            }
            if !rings.is_empty() {
                out.push(Feature::new(
                    Kind::Region(level),
                    Geometry::Polygon(rings),
                    r.id,
                    &r.tags,
                ));
            }
            continue;
        }
        if let Some(kind) = classify_area(&r.tags) {
            let rings = assemble_rings(member_segments(r, &["outer", "inner"]));
            if !rings.is_empty() {
                out.push(Feature::new(kind, Geometry::Polygon(rings), r.id, &r.tags));
            }
        }
    }

    Ok(out)
}

/// Join way segments end-to-end into closed rings (multipolygon assembly).
/// Unclosable chains are force-closed – good enough for map art.
pub fn assemble_rings(segments: Vec<Vec<[f64; 2]>>) -> Vec<Vec<[f64; 2]>> {
    assemble_rings_report(segments).0
}

/// Like [`assemble_rings`], also returning how many rings had to be force-closed.
pub fn assemble_rings_report(mut segments: Vec<Vec<[f64; 2]>>) -> (Vec<Vec<[f64; 2]>>, usize) {
    let mut rings = Vec::new();
    let mut forced = 0;
    while let Some(mut ring) = segments.pop() {
        loop {
            if ring.len() >= 4 && ring.first() == ring.last() {
                break;
            }
            let end = *ring.last().unwrap();
            let mut found = None;
            for (i, seg) in segments.iter().enumerate() {
                if seg[0] == end {
                    found = Some((i, false));
                    break;
                }
                if *seg.last().unwrap() == end {
                    found = Some((i, true));
                    break;
                }
            }
            match found {
                Some((i, reverse)) => {
                    let mut seg = segments.swap_remove(i);
                    if reverse {
                        seg.reverse();
                    }
                    ring.extend_from_slice(&seg[1..]);
                }
                None => {
                    if ring.len() >= 3 {
                        ring.push(ring[0]);
                        forced += 1;
                    }
                    break;
                }
            }
        }
        if ring.len() >= 4 {
            rings.push(ring);
        }
    }
    (rings, forced)
}

fn classify_area(tags: &HashMap<String, String>) -> Option<Kind> {
    let t = |k: &str| tags.get(k).map(String::as_str);
    if t("building").is_some() {
        return Some(Kind::Building);
    }
    match t("natural") {
        Some("water") => return Some(Kind::Water),
        Some("wood") => return Some(Kind::Forest),
        Some("sand" | "beach" | "shoal") => return Some(Kind::Sand),
        Some("wetland") => return Some(Kind::Wetland),
        Some("grassland" | "scrub" | "heath") => return Some(Kind::Grass),
        _ => {}
    }
    if let Some("riverbank" | "dock") = t("waterway") {
        return Some(Kind::Water);
    }
    match t("landuse") {
        Some("forest" | "orchard" | "vineyard") => return Some(Kind::Forest),
        Some("residential" | "commercial" | "retail") => return Some(Kind::Urban),
        Some("industrial" | "port") => return Some(Kind::Industrial),
        Some("farmland" | "farmyard" | "allotments") => return Some(Kind::Farmland),
        Some("grass" | "meadow" | "cemetery") => return Some(Kind::Grass),
        _ => {}
    }
    if let Some("park" | "golf_course") = t("leisure") {
        return Some(Kind::Grass);
    }
    None
}

fn classify_line(tags: &HashMap<String, String>) -> Option<Kind> {
    let t = |k: &str| tags.get(k).map(String::as_str);
    if t("natural") == Some("coastline") {
        return Some(Kind::Coastline);
    }
    match t("waterway") {
        Some("river" | "canal") => return Some(Kind::River),
        Some("stream") => return Some(Kind::Stream),
        _ => {}
    }
    match t("highway") {
        Some("motorway" | "trunk" | "primary") => return Some(Kind::RoadMajor),
        Some("secondary" | "tertiary") => return Some(Kind::RoadMinor),
        _ => {}
    }
    if t("railway") == Some("rail") {
        return Some(Kind::Rail);
    }
    None
}

/// Border line kind + numeric admin level.
fn classify_border(tags: &HashMap<String, String>) -> Option<(Kind, u8)> {
    if tags.get("boundary").map(String::as_str) != Some("administrative") {
        return None;
    }
    let level: u8 = tags.get("admin_level")?.parse().ok()?;
    let kind = match level {
        0..=3 => Kind::BorderCountry,
        4 => Kind::BorderRegion,
        5..=8 => Kind::BorderLocal,
        _ => return None,
    };
    Some((kind, level))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembles_split_ring_in_any_direction() {
        // square split into 3 ways, one reversed
        let a = vec![[0.0, 0.0], [1.0, 0.0]];
        let b = vec![[1.0, 1.0], [1.0, 0.0]]; // reversed
        let c = vec![[1.0, 1.0], [0.0, 1.0], [0.0, 0.0]];
        let (rings, forced) = assemble_rings_report(vec![a, b, c]);
        assert_eq!(rings.len(), 1);
        assert_eq!(forced, 0);
        assert_eq!(rings[0].first(), rings[0].last());
        assert_eq!(rings[0].len(), 5);
    }

    #[test]
    fn force_closes_open_chain() {
        let a = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]];
        let (rings, forced) = assemble_rings_report(vec![a]);
        assert_eq!(rings.len(), 1);
        assert_eq!(forced, 1);
    }
}
