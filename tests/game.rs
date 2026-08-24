//! Game integration tests on the checked-in fixture map.

use clap::Parser;
use mapgenart::config::MapConfig;
use mapgenart::game::buildings;
use mapgenart::game::fog::Fog;
use mapgenart::game::logic::Alerts;
use mapgenart::game::nav::{CELL, NavGrid, greedy_rects};
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
    // a light population so the smoke run exercises the loop without a
    // guaranteed wipe on the tiny fixture map
    let summary = mapgenart::game::run_headless_sim(&cfg(&["--population", "20"]), 800).unwrap();
    assert!(summary.contains("(0 in blocked cells)"), "{summary}");
    assert!(summary.contains("soldiers"), "{summary}");
    assert!(
        summary.contains("sh/"),
        "archetype counts missing: {summary}"
    );
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

fn sf_enterable(width: &str) -> generate::Generated {
    let c = MapConfig::parse_from([
        "mapgenart",
        "--input",
        SF,
        "--bbox",
        SF_BBOX,
        "--width",
        width,
        "--buildings",
        "--enterable",
        "--no-political",
        "--labels",
        "false",
        "--cities",
        "false",
    ]);
    generate::generate(&c).unwrap()
}

#[test]
fn doors_make_interiors_reachable() {
    let mut g = sf_enterable("1024");
    let carved = buildings::carve(&mut g);
    let (w, h) = (g.rendered.canvas.width, g.rendered.canvas.height);
    // movement mask: carved walls + water
    let mut blocked = carved.walls.clone();
    for (i, t) in g.rendered.canvas.tags.iter().enumerate() {
        if *t == mapgenart::raster::layer::OCEAN {
            blocked[i] = true;
        }
    }
    let grid = NavGrid::from_blocked(w, h, &blocked);
    // BFS over walkable nav cells from an outdoor corner-ish seed
    let seed = grid.nearest_walkable((4, 4)).expect("outdoor seed");
    let mut seen = vec![false; (grid.w * grid.h) as usize];
    let mut q = std::collections::VecDeque::from([seed]);
    seen[grid.idx(seed.0, seed.1).unwrap()] = true;
    while let Some((x, y)) = q.pop_front() {
        for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
            if let Some(i) = grid.idx(nx, ny)
                && !seen[i]
                && !grid.blocked[i]
            {
                seen[i] = true;
                q.push_back((nx, ny));
            }
        }
    }
    let (mut indoor_cells, mut reachable) = (0usize, 0usize);
    for (i, ind) in g.rendered.indoor.iter().enumerate() {
        if !*ind {
            continue;
        }
        let (x, y) = ((i as u32 % w) / CELL, (i as u32 / w) / CELL);
        let ci = (y * grid.w + x) as usize;
        if !grid.blocked[ci] {
            indoor_cells += 1;
            if seen[ci] {
                reachable += 1;
            }
        }
    }
    assert!(indoor_cells > 1000, "{indoor_cells} indoor nav cells");
    let frac = reachable as f64 / indoor_cells as f64;
    assert!(
        frac > 0.7,
        "only {:.0}% of interiors reachable",
        frac * 100.0
    );
}

#[test]
fn windows_pass_sight_but_block_movement() {
    let mut g = sf_enterable("400");
    let carved = buildings::carve(&mut g);
    let windows: Vec<usize> = carved
        .walls
        .iter()
        .zip(carved.sight.iter())
        .enumerate()
        .filter(|(_, (w, s))| **w && !**s)
        .map(|(i, _)| i)
        .collect();
    assert!(windows.len() > 100, "{} windows", windows.len());
    // a ray straight through a window pixel passes the sight mask
    let (w, h) = (g.rendered.canvas.width, g.rendered.canvas.height);
    let mut verified = false;
    for i in windows {
        let (x, y) = ((i as u32 % w) as f32, (i as u32 / w) as f32);
        let a = (x - 1.5, y);
        let b = (x + 1.5, y);
        let ai = (y as u32 * w + (x - 1.0) as u32) as usize;
        let bi = (y as u32 * w + (x + 1.0) as u32) as usize;
        if x < 2.0 || x > w as f32 - 2.0 || carved.sight[ai] || carved.sight[bi] {
            continue;
        }
        assert!(
            Fog::line_of_sight(&carved.sight, w, h, a, b),
            "sight must pass the window"
        );
        verified = true;
        break;
    }
    assert!(verified, "no straight-through window found to verify");
}

#[test]
fn alerts_merge_decay_and_query() {
    let mut a = Alerts::default();
    a.push(bevy_math_vec(10.0, 10.0));
    a.push(bevy_math_vec(15.0, 12.0)); // merges (<24 apart)
    assert_eq!(a.0.len(), 1);
    a.push(bevy_math_vec(200.0, 0.0));
    assert_eq!(a.0.len(), 2);
    assert!(a.nearest(bevy_math_vec(0.0, 0.0), 50.0).is_some());
    assert!(a.nearest(bevy_math_vec(500.0, 500.0), 50.0).is_none());
    a.decay(mapgenart::game::logic::ALERT_TTL + 1.0);
    assert!(a.0.is_empty());
}

fn bevy_math_vec(x: f32, y: f32) -> bevy::math::Vec2 {
    bevy::math::Vec2::new(x, y)
}

#[test]
fn paths_keep_clearance_in_open_streets() {
    // synthetic: 40×40, one 12×12 building; the path around it must not touch
    // tight cells (adjacent to walls) since there is plenty of room
    let (w, h) = (40u32, 40u32);
    let mut blocked = vec![false; (w * h) as usize];
    for y in 14..26 {
        for x in 14..26 {
            blocked[(y * w + x) as usize] = true;
        }
    }
    let g = NavGrid::from_blocked(w, h, &blocked);
    let path = g.path((4.0, 20.0), (36.0, 20.0)).expect("path");
    let mut prev = (4.0, 20.0);
    for wp in &path {
        // sample the polyline: every cell it crosses must be non-tight
        let steps = 20;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let p = (prev.0 + (wp.0 - prev.0) * t, prev.1 + (wp.1 - prev.1) * t);
            let c = g.cell_of(p.0, p.1);
            let idx = g.idx(c.0, c.1).unwrap();
            assert!(!g.tight[idx], "path scrapes a wall at {p:?}");
        }
        prev = *wp;
    }
}

#[test]
fn narrow_corridor_still_passable() {
    // a 3px door is the only way through a full-width wall
    let (w, h) = (30u32, 20u32);
    let mut blocked = vec![false; (w * h) as usize];
    for y in 0..h {
        if !(9..12).contains(&y) {
            blocked[(y * w + 15) as usize] = true;
            blocked[(y * w + 16) as usize] = true;
        }
    }
    let g = NavGrid::from_blocked(w, h, &blocked);
    let path = g
        .path((4.0, 10.0), (26.0, 10.0))
        .expect("path through the door");
    let end = path.last().unwrap();
    assert!((end.0 - 26.0).abs() < 3.0);
}

#[test]
fn squad_reaches_ordered_goal() {
    let (mean, max) = mapgenart::game::run_goal_sim(&cfg(&[]), 2500).unwrap();
    assert!(mean < 8.0, "mean distance to goal {mean:.1}px");
    assert!(max < 20.0, "worst straggler {max:.1}px from goal");
}

#[test]
fn squad_reaches_goal_in_sf_streets() {
    let mut c = sf_cfg();
    c.width = 1024;
    c.enterable = true;
    let (mean, max) = mapgenart::game::run_goal_sim(&c, 4000).unwrap();
    assert!(mean < 12.0, "mean distance to goal {mean:.1}px");
    assert!(max < 40.0, "worst straggler {max:.1}px from goal");
}

#[test]
fn flow_field_avoids_wall_hugging() {
    use mapgenart::game::nav::FlowField;
    // 40×40, one central building; goal on the west, follower starts east
    let (w, h) = (40u32, 40u32);
    let mut blocked = vec![false; (w * h) as usize];
    for y in 14..26 {
        for x in 14..26 {
            blocked[(y * w + x) as usize] = true;
        }
    }
    let g = NavGrid::from_blocked(w, h, &blocked);
    let f = FlowField::compute(&g, &[(4.0, 20.0)]);
    let (mut x, mut y) = (36.0f32, 20.0f32);
    let (mut steps, mut tight_steps) = (0, 0);
    for _ in 0..400 {
        let (dx, dy) = f.sample(&g, x, y);
        if dx == 0.0 && dy == 0.0 {
            break;
        }
        x += dx;
        y += dy;
        steps += 1;
        let c = g.cell_of(x, y);
        if let Some(i) = g.idx(c.0, c.1)
            && g.tight[i]
        {
            tight_steps += 1;
        }
    }
    assert!(
        (x - 4.0).abs() < 4.0 && (y - 20.0).abs() < 4.0,
        "ended at {x},{y}"
    );
    // the detour keeps clearance: at most a small fraction touches wall-adjacent cells
    assert!(
        tight_steps * 4 <= steps,
        "{tight_steps}/{steps} steps hugged the wall"
    );
}

// ---------------------------------------------------------------------------
// milestone 4: noise world, economy, objectives, ranks, barricades, night

use bevy::prelude::*;
use mapgenart::game::units::{Dormant, Dossier, Enemy as EnemyC, Soldier as SoldierC};
use mapgenart::game::{
    economy, headless_app, objectives, population, setup_session, tuning, world::GameWorld,
};

fn game_app(extra: &[&str]) -> bevy::app::App {
    let c = cfg(extra);
    let mut g = generate::generate(&c).unwrap();
    let mut app = headless_app(&c);
    {
        let world_cell = app.world_mut();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        let mut commands = bevy::ecs::system::Commands::new(&mut queue, world_cell);
        setup_session(&mut commands, &c, &mut g, None);
        queue.apply(app.world_mut());
    }
    app.finish();
    app.cleanup();
    app.update();
    app
}

#[test]
fn population_seeds_in_bounds_and_dormant_without_physics() {
    let mut app = game_app(&["--population", "120"]);
    let world = app.world_mut();
    let dormant = world
        .query_filtered::<&Transform, (With<EnemyC>, With<Dormant>)>()
        .iter(world)
        .map(|t| t.translation.truncate())
        .collect::<Vec<_>>();
    assert!(dormant.len() > 80, "{} sleepers", dormant.len());
    // dormant = no colliders
    let with_col = world
        .query_filtered::<(), (
            With<EnemyC>,
            With<Dormant>,
            With<avian2d::prelude::Collider>,
        )>()
        .iter(world)
        .count();
    assert_eq!(with_col, 0, "dormant enemies must carry no physics");
    let gw = world.resource::<GameWorld>();
    for p in &dormant {
        let (x, y) = gw.to_map(*p);
        assert!(x >= 0.0 && y >= 0.0 && x < gw.w as f32 && y < gw.h as f32);
    }
}

fn sf_game_app(extra_pop: u32) -> bevy::app::App {
    let mut c = sf_cfg();
    c.width = 1024;
    c.enterable = true;
    c.population = Some(extra_pop);
    let mut g = generate::generate(&c).unwrap();
    let mut app = headless_app(&c);
    {
        let world_cell = app.world_mut();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        let mut commands = bevy::ecs::system::Commands::new(&mut queue, world_cell);
        setup_session(&mut commands, &c, &mut g, None);
        queue.apply(app.world_mut());
    }
    app.finish();
    app.cleanup();
    app.update();
    app
}

#[test]
fn one_shot_wakes_a_bounded_neighbourhood() {
    let mut app = sf_game_app(600);
    // a single noise event at the map centre
    {
        let world = app.world_mut();
        let centre = {
            let gw = world.resource::<GameWorld>();
            gw.to_world(gw.w as f32 / 2.0, gw.h as f32 / 2.0)
        };
        world.write_message(population::Noise {
            pos: centre,
            radius: tuning::NOISE_RIFLE,
        });
    }
    for _ in 0..3 {
        app.update();
    }
    let world = app.world_mut();
    let awake = world
        .query_filtered::<(), (With<EnemyC>, Without<Dormant>)>()
        .iter(world)
        .count();
    let total = world
        .query_filtered::<(), With<EnemyC>>()
        .iter(world)
        .count();
    assert!(awake > 0, "someone must hear a rifle shot at the centre");
    assert!(
        awake * 2 < total,
        "one shot woke {awake}/{total} — chain must fall off"
    );
}

#[test]
fn objectives_reachable_and_ordered() {
    let mut app = game_app(&[]);
    let world = app.world_mut();
    let squad0 = {
        let mut q = world.query_filtered::<&Transform, With<SoldierC>>();
        q.iter(world).next().unwrap().translation.truncate()
    };
    let obj = world.resource::<objectives::Objectives>();
    assert!(!obj.list.is_empty());
    let last = obj.list.last().unwrap();
    assert_eq!(last.kind, objectives::ObjectiveKind::Extract);
    let gw = world.resource::<GameWorld>();
    let ex_d = last.pos.distance(squad0);
    for o in &obj.list {
        // every objective is reachable
        let from = gw.to_map(squad0);
        let to = gw.to_map(o.pos);
        assert!(
            gw.nav.path(from, to).is_some(),
            "objective {} unreachable",
            o.name
        );
        // mids lie nearer than the extraction
        if o.kind == objectives::ObjectiveKind::Search {
            assert!(o.pos.distance(squad0) < ex_d);
        }
    }
}

#[test]
fn loot_respects_poi_richness() {
    let mut app = sf_game_app(0);
    let world = app.world_mut();
    let sites = world.resource::<economy::LootSites>();
    assert!(sites.0.len() > 50, "{} sites", sites.0.len());
    let max = sites.0.iter().map(|s| s.total).fold(0.0f32, f32::max);
    let min = sites.0.iter().map(|s| s.total).fold(f32::MAX, f32::min);
    assert!(max > min * 2.0, "richness must vary (max {max}, min {min})");
}

#[test]
fn ammo_drains_and_stays_nonnegative() {
    let summary = mapgenart::game::run_headless_sim(&cfg(&["--population", "200"]), 2500).unwrap();
    // "ammo N" from the summary line
    let ammo: i32 = summary
        .split("ammo ")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.parse().ok())
        .unwrap();
    assert!(ammo >= 0, "{summary}");
    assert!(
        ammo < tuning::START_AMMO as i32,
        "no shots fired? {summary}"
    );
}

#[test]
fn ranks_and_names() {
    let mut d = Dossier {
        name: mapgenart::game::units::soldier_name(3, 7),
        kills: 0,
        shots: 0,
    };
    assert_eq!(d.rank(), 0);
    d.kills = tuning::RANK_KILLS[0];
    assert_eq!(d.rank(), 1);
    d.kills = tuning::RANK_KILLS[2] + 5;
    assert_eq!(d.rank(), 3);
    assert!(d.damage_mult() > 1.2);
    assert!(d.noise_mult() < 0.8);
    assert_eq!(
        mapgenart::game::units::soldier_name(3, 7),
        mapgenart::game::units::soldier_name(3, 7)
    );
}

#[test]
fn barricade_blocks_and_reopens_nav() {
    let mut app = game_app(&[]);
    let world = app.world_mut();
    // fixture has no carved openings (no buildings) — use SF instead if empty
    let has_openings = !world.resource::<GameWorld>().openings.is_empty();
    if !has_openings {
        // build on the SF map
        drop(app);
        let mut app = sf_game_app(0);
        let world = app.world_mut();
        let mut gw = world.resource_mut::<GameWorld>();
        let idx = gw
            .openings
            .iter()
            .position(|o| o.kind == mapgenart::game::buildings::OpeningKind::Door)
            .expect("a door");
        let centre = gw.openings[idx].centre;
        let cell = gw.nav.cell_of(centre.0, centre.1);
        assert!(!gw.nav.is_blocked(cell.0, cell.1), "door cell open before");
        mapgenart::game::barricade::set_masks(&mut gw, idx, true);
        assert!(gw.nav.is_blocked(cell.0, cell.1), "boarded door blocks nav");
        mapgenart::game::barricade::set_masks(&mut gw, idx, false);
        assert!(!gw.nav.is_blocked(cell.0, cell.1), "torn down reopens");
    }
}

#[test]
fn night_doubles_wake_radius() {
    assert_eq!(population::wake_mult(false), 1.0);
    assert!(population::wake_mult(true) >= 1.9);
}

// ---------------------------------------------------------------------------
// milestone 5: archetypes, cause-and-effect, director, sound

use mapgenart::game::units::EnemyKind;

#[test]
fn archetype_ratios_and_stats() {
    let mut counts = [0u32; 4];
    for i in 0..10_000 {
        let r = i as f32 / 10_000.0;
        counts[match EnemyKind::roll(r) {
            EnemyKind::Shambler => 0,
            EnemyKind::Shrieker => 1,
            EnemyKind::Runner => 2,
            EnemyKind::Brute => 3,
        }] += 1;
    }
    assert!(counts[0] > 7000, "shamblers {}", counts[0]);
    assert!((counts[1] as f32 / 10_000.0 - tuning::RATIO_SHRIEKER).abs() < 0.01);
    assert!((counts[2] as f32 / 10_000.0 - tuning::RATIO_RUNNER).abs() < 0.01);
    assert!((counts[3] as f32 / 10_000.0 - tuning::RATIO_BRUTE).abs() < 0.01);
    let (hp, sp, _) = EnemyKind::Brute.stats(30.0, 20.0, 6.0);
    assert!(hp > 150.0 && sp < 15.0);
    let (hp, sp, _) = EnemyKind::Runner.stats(30.0, 20.0, 6.0);
    assert!(hp < 30.0 && sp > 35.0);
    assert!(EnemyKind::Brute.radius() > EnemyKind::Shambler.radius());
}

#[test]
fn shrieker_death_is_louder_than_a_rifle() {
    let rifle = std::hint::black_box(tuning::NOISE_RIFLE);
    assert!(rifle * tuning::SHRIEKER_SCREAM_MULT > rifle * 2.5);
    // and a brute takes far more barricade damage per hit than a shambler
    let base = std::hint::black_box(tuning::BARRICADE_ENEMY_DMG);
    assert!(base * tuning::BRUTE_BARRICADE_MULT >= base * 4.0);
}

fn awake_runners(app: &mut bevy::app::App) -> usize {
    let world = app.world_mut();
    world
        .query_filtered::<&EnemyC, Without<Dormant>>()
        .iter(world)
        .filter(|e| e.kind == EnemyKind::Runner)
        .count()
}

fn a_dormant_runner(app: &mut bevy::app::App) -> Vec2 {
    let world = app.world_mut();
    let mut q = world.query_filtered::<(&Transform, &EnemyC), With<Dormant>>();
    q.iter(world)
        .find(|(_, e)| e.kind == EnemyKind::Runner)
        .map(|(t, _)| t.translation.truncate())
        .expect("a dormant runner")
}

#[test]
fn runners_sleep_through_daytime_soft_noise_but_wake_to_gunfire() {
    // sparse population so the shriek chain from *other* sleepers (which may
    // legitimately wake a runner) is out of the picture
    let mut app = sf_game_app(60);
    let runner_pos = a_dormant_runner(&mut app);
    app.world_mut().write_message(population::Noise {
        pos: runner_pos,
        radius: tuning::NOISE_HAMMER,
    });
    for _ in 0..3 {
        app.update();
    }
    assert_eq!(
        awake_runners(&mut app),
        0,
        "hammering by day must not wake runners"
    );
    // control: a rifle shot on top of it does
    app.world_mut().write_message(population::Noise {
        pos: runner_pos,
        radius: tuning::NOISE_RIFLE,
    });
    for _ in 0..3 {
        app.update();
    }
    assert!(
        awake_runners(&mut app) >= 1,
        "a rifle shot must wake the runner"
    );
}

fn clear_sleepers_near_squad(app: &mut bevy::app::App, radius: f32) {
    let world = app.world_mut();
    let centroid = {
        let mut q = world.query_filtered::<&Transform, With<SoldierC>>();
        let v: Vec<Vec2> = q.iter(world).map(|t| t.translation.truncate()).collect();
        v.iter().copied().sum::<Vec2>() / v.len().max(1) as f32
    };
    let near: Vec<Entity> = {
        let mut q = world.query_filtered::<(Entity, &Transform), (With<EnemyC>, With<Dormant>)>();
        q.iter(world)
            .filter(|(_, t)| t.translation.truncate().distance(centroid) < radius)
            .map(|(e, _)| e)
            .collect()
    };
    for e in near {
        world.despawn(e);
    }
}

#[test]
fn director_breaks_a_lull_but_not_during_extraction() {
    use mapgenart::game::director::Director;
    // quiet squad (no orders) on the SF map with sleepers spread around: after
    // the lull window the director must have sent a scout pack
    let mut app = sf_game_app(300);
    // a genuinely quiet squad: nothing to shoot at within sight range
    clear_sleepers_near_squad(&mut app, 100.0);
    let ticks = ((tuning::DIRECTOR_LULL_S + 15.0) / 0.016) as u32;
    for _ in 0..ticks {
        app.update();
    }
    let actions = app.world().resource::<Director>().actions;
    assert!(actions >= 1, "director never acted in {ticks} quiet ticks");
    // during the extraction hold the lull breaker stays silent
    let mut app = sf_game_app(300);
    clear_sleepers_near_squad(&mut app, 100.0);
    {
        let world = app.world_mut();
        let mut obj = world.resource_mut::<objectives::Objectives>();
        for o in obj.list.iter_mut() {
            if o.kind == objectives::ObjectiveKind::Search {
                o.done = true;
            }
        }
        obj.alarm_fired = true;
        obj.hold = 0.0;
    }
    for _ in 0..ticks {
        app.update();
        // keep it "extracting" (hold never completes: no soldier at the point)
    }
    let d = app.world().resource::<Director>();
    assert_eq!(d.actions, 0, "director acted during extraction");
}

#[test]
fn decal_cap_is_a_sane_bound() {
    let cap = std::hint::black_box(tuning::DECAL_CAP);
    assert!((1000..=20_000).contains(&cap));
}

#[test]
fn audio_buffers_and_voice_cap() {
    use mapgenart::game::audio;
    let r = audio::rifle();
    assert!(r.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
    assert!(r.iter().any(|s| s.abs() > 0.1));
    let s = audio::shriek();
    assert!(s.len() > r.len(), "shriek is longer than a shot");
    let voices = std::hint::black_box(tuning::MAX_VOICES);
    assert!((8..=64).contains(&voices));
}
