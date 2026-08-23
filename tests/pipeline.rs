//! Integration tests on a small checked-in Overpass fixture (central
//! Copenhagen harbour, cropped and thinned, plus a synthetic admin relation).

use clap::Parser;
use mapgenart::config::MapConfig;
use mapgenart::generate;
use mapgenart::osm::{self, Geometry, Kind};
use mapgenart::palette::Palette;
use mapgenart::raster::layer;
use mapgenart::scenario::Scenario;
use std::path::Path;

const FIXTURE: &str = "tests/fixtures/small.json";
const GOLDEN: &str = "tests/fixtures/small.golden.png";
const BBOX: &str = "55.674,12.588,55.686,12.602";

fn cfg(extra: &[&str]) -> MapConfig {
    let mut args = vec![
        "mapgenart",
        "--input",
        FIXTURE,
        "--bbox",
        BBOX,
        "--width",
        "160",
    ];
    args.extend_from_slice(extra);
    MapConfig::parse_from(args)
}

fn features() -> Vec<osm::Feature> {
    let raw = std::fs::read_to_string(FIXTURE).unwrap();
    osm::parse(&raw).unwrap()
}

#[test]
fn full_pipeline_renders() {
    let g = generate::generate(&cfg(&[])).expect("pipeline");
    let c = g.canvas();
    assert_eq!(c.width, 160);
    assert!(c.height > 100 && c.height < 260, "height {}", c.height);
    assert!(g.feature_count > 100);
}

#[test]
fn coastline_vote_fill_produces_land_and_ocean() {
    let g = generate::generate(&cfg(&["--no-political", "--shoreline", "false"])).unwrap();
    let c = g.canvas();
    let ocean = c.tags.iter().filter(|t| **t == layer::OCEAN).count();
    let land = c.tags.iter().filter(|t| **t != layer::OCEAN).count();
    let total = c.tags.len();
    assert!(ocean > total / 10, "too little ocean: {ocean}/{total}");
    assert!(land > total / 10, "too little land: {land}/{total}");
    let pal = Palette::default();
    assert!(c.pixels.contains(&pal.ocean));
}

#[test]
fn region_assembly_yields_closed_rings() {
    let feats = features();
    let regions: Vec<_> = feats
        .iter()
        .filter(|f| matches!(f.kind, Kind::Region(_)))
        .collect();
    assert!(!regions.is_empty(), "no Region features parsed");
    for r in &regions {
        let Geometry::Polygon(rings) = &r.geom else {
            panic!("region must be a polygon")
        };
        for ring in rings {
            assert!(ring.len() >= 4);
            assert_eq!(
                ring.first(),
                ring.last(),
                "ring not closed for relation {}",
                r.id
            );
        }
    }
    let test_region = regions
        .iter()
        .find(|r| r.name.as_deref() == Some("Test Region"))
        .expect("synthetic region");
    assert_eq!(test_region.kind, Kind::Region(4));
}

#[test]
fn political_fill_and_scenario_colour() {
    let mut scen = Scenario::default();
    scen.set_colour(-10, [10, 20, 30, 255]);
    let c = cfg(&["--shoreline", "false"]);
    let feats = features();
    let g = generate::render_features(&c, &feats, &Palette::default(), scen, None).unwrap();
    assert_eq!(g.rendered.admin_level_used, Some(4));
    assert_eq!(g.rendered.regions.len(), 1);
    assert!(g.rendered.regions[0].pixels > 100);
    assert!(g.canvas().pixels.contains(&[10, 20, 30, 255]));
}

#[test]
fn tiles_split_and_metres_per_pixel() {
    let b = cfg(&[]).bbox().unwrap();
    let t = b.tiles(2);
    assert_eq!(t.len(), 4);
    assert_eq!(t[0].south, b.south);
    assert_eq!(t[3].north, b.north);
    assert_eq!(t[3].east, b.east);
    let mpp = b.metres_per_pixel(160);
    assert!((4.0..8.0).contains(&mpp), "mpp {mpp}");
}

#[test]
fn golden_image() {
    let g = generate::generate(&cfg(&[])).unwrap();
    let c = g.canvas();
    if std::env::var("UPDATE_GOLDEN").is_ok() || !Path::new(GOLDEN).exists() {
        generate::save_png(c, Path::new(GOLDEN)).unwrap();
        eprintln!("wrote {GOLDEN}");
        return;
    }
    let img = image::open(GOLDEN).unwrap().into_rgba8();
    assert_eq!(
        (img.width(), img.height()),
        (c.width, c.height),
        "golden size mismatch"
    );
    let mut mismatched = 0usize;
    for (i, px) in img.pixels().enumerate() {
        if px.0 != c.pixels[i] {
            mismatched += 1;
        }
    }
    let total = c.pixels.len();
    let tolerance = total / 200; // 0.5 %
    assert!(
        mismatched <= tolerance,
        "{mismatched} of {total} pixels differ from {GOLDEN} (tolerance {tolerance}); run with UPDATE_GOLDEN=1 to accept"
    );
}
