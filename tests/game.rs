//! Game integration tests on the checked-in fixture map.

use clap::Parser;
use mapgenart::config::MapConfig;
use mapgenart::game::fog::Fog;
use mapgenart::game::nav::{NavGrid, greedy_rects};
use mapgenart::generate;
use mapgenart::palette::Palette;
use mapgenart::scenario::Scenario;

const FIXTURE: &str = "tests/fixtures/small.json";
const BBOX: &str = "55.674,12.588,55.686,12.602";
/// The bundled game map (dense SF building footprints).
const SF: &str = "assets/maps/sf.json";
const SF_BBOX: &str = "37.780,-122.425,37.795,-122.398";

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

fn sf_cfg() -> MapConfig {
    MapConfig::parse_from([
        "mapgenart",
        "--input",
        SF,
        "--bbox",
        SF_BBOX,
        "--width",
        "300",
        "--buildings",
        "--no-political",
        "--labels",
        "false",
        "--cities",
        "false",
    ])
}

fn generated() -> generate::Generated {
    generate::generate(&sf_cfg()).unwrap()
}

#[test]
fn colliders_cover_buildings() {
    let g = generated();
    let blocked = g.rendered.blocked();
    let (w, h) = (g.rendered.canvas.width, g.rendered.canvas.height);
    let rects = greedy_rects(w, h, &blocked);
    assert!(
        rects.len() > 50,
        "expected many colliders, got {}",
        rects.len()
    );
    // every building pixel is covered by some rect, and rect pixels are blocked
    let mut covered = vec![false; blocked.len()];
    for (x, y, rw, rh) in &rects {
        for yy in *y..y + rh {
            for xx in *x..x + rw {
                let i = (yy * w + xx) as usize;
                assert!(blocked[i], "rect covers walkable pixel {xx},{yy}");
                covered[i] = true;
            }
        }
    }
    assert_eq!(
        covered, blocked,
        "colliders must cover exactly the blocked pixels"
    );
    // a known point inside a building is blocked
    let inside = g
        .rendered
        .building
        .iter()
        .position(|b| *b)
        .expect("some building");
    assert!(blocked[inside]);
}

#[test]
fn nav_path_avoids_buildings_on_real_map() {
    // full game resolution: streets must stay open in the nav grid
    let mut c = sf_cfg();
    c.width = 1024;
    let g = generate::generate(&c).unwrap();
    let blocked = g.rendered.blocked();
    let (w, h) = (g.rendered.canvas.width, g.rendered.canvas.height);
    let grid = NavGrid::from_blocked(w, h, &blocked);
    // find two walkable points on opposite sides with a straight line that
    // crosses something blocked, then require a valid detour
    let mut found = false;
    'outer: for y in (10..h as i32 - 10).step_by(5) {
        // snap endpoints to walkable cells near the row ends
        let sa = grid.nearest_walkable(grid.cell_of(10.0, y as f32));
        let sb = grid.nearest_walkable(grid.cell_of(w as f32 - 10.0, y as f32));
        let (Some(sa), Some(sb)) = (sa, sb) else {
            continue;
        };
        let a = grid.centre(sa);
        let b = grid.centre(sb);
        if grid.line_walkable(a, b) {
            continue; // no obstacle between these two
        }
        if let Some(path) = grid.path(a, b) {
            let mut prev = a;
            for wp in &path {
                assert!(grid.line_walkable(prev, *wp), "path segment blocked");
                prev = *wp;
            }
            found = true;
            break 'outer;
        }
    }
    assert!(found, "no blocked row with a valid detour found");
}

#[test]
fn los_blocked_by_real_building() {
    let g = generated();
    let (w, h) = (g.rendered.canvas.width, g.rendered.canvas.height);
    let sight = &g.rendered.building;
    // pick a building pixel and points on either side of it horizontally
    let mut checked = false;
    for (i, b) in sight.iter().enumerate() {
        if !*b {
            continue;
        }
        let (x, y) = ((i as u32 % w) as f32, (i as u32 / w) as f32);
        if x < 8.0 || x > w as f32 - 8.0 {
            continue;
        }
        let a = (x - 6.0, y);
        let c = (x + 6.0, y);
        let ai = (y as u32 * w + (x - 6.0) as u32) as usize;
        let ci = (y as u32 * w + (x + 6.0) as u32) as usize;
        if sight[ai] || sight[ci] {
            continue;
        }
        assert!(
            !Fog::line_of_sight(sight, w, h, a, c),
            "building must block LOS"
        );
        assert!(
            Fog::line_of_sight(sight, w, h, a, (x - 2.0, y))
                || sight[(y as u32 * w + (x - 2.0) as u32) as usize]
        );
        checked = true;
        break;
    }
    assert!(checked, "no suitable building found");
}

#[test]
fn wave_scaling_math() {
    // mirrors logic::wave_director
    let count = |wave: u32| 6 + wave * 5;
    let hp = |wave: u32| 26.0 * 1.12f32.powi(wave as i32 - 1);
    assert_eq!(count(1), 11);
    assert!(count(10) > count(5));
    assert!(hp(10) > hp(1) * 2.0);
    let speed = |wave: u32| (16.0 + wave as f32 * 1.2).min(34.0);
    assert_eq!(speed(50), 34.0, "speed is capped");
}

#[test]
fn headless_sim_smoke() {
    let summary = mapgenart::game::run_headless_sim(&cfg(&[]), 800).unwrap();
    assert!(summary.contains("(0 in blocked cells)"), "{summary}");
    assert!(summary.contains("8 soldiers"), "{summary}");
}

#[test]
fn stress_200_units_no_tunneling_no_nan() {
    let (stuck, nan) = mapgenart::game::run_stress(&cfg(&[]), 200, 500).unwrap();
    assert_eq!(nan, 0, "NaN positions after stress");
    assert_eq!(stuck, 0, "{stuck} units ended inside blocked pixels");
}

#[test]
fn postapoc_palette_loads() {
    let p = Palette::load(std::path::Path::new("palettes/postapoc.toml")).unwrap();
    assert_ne!(p.ocean, Palette::default().ocean);
    let _ = Scenario::default();
}
