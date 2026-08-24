//! "Last Light" — real-time tactics on OSM pixel-art maps. StarCraft-style
//! squad control, hordes, fog of war; no base building.

pub mod audio;
pub mod barricade;
pub mod buildings;
pub mod control;
pub mod director;
pub mod economy;
pub mod fog;
pub mod logic;
pub mod nav;
pub mod objectives;
pub mod population;
pub mod tuning;
pub mod units;
pub mod view;
pub mod world;

use crate::config::MapConfig;
use crate::generate::{self, Generated};
use bevy::prelude::*;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};

/// Run-long day/night clock.
#[derive(Resource)]
pub struct DayNight {
    /// seconds into the current phase
    pub t: f32,
    pub is_night: bool,
}

impl Default for DayNight {
    fn default() -> Self {
        DayNight {
            t: 0.0,
            is_night: false,
        }
    }
}

fn day_night_clock(time: Res<Time>, mut dn: ResMut<DayNight>) {
    dn.t += time.delta_secs();
    let span = if dn.is_night {
        tuning::NIGHT_S
    } else {
        tuning::DAY_S
    };
    if dn.t >= span {
        dn.t = 0.0;
        dn.is_night = !dn.is_night;
        log::info!("{} falls", if dn.is_night { "night" } else { "day" });
    }
}

#[derive(States, Default, Clone, Eq, PartialEq, Debug, Hash)]
pub enum Phase {
    #[default]
    Menu,
    Loading,
    Playing,
    GameOver,
}

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<Phase>()
            .init_resource::<MapLoad>()
            .add_plugins(avian2d::PhysicsPlugins::default())
            .insert_resource(avian2d::prelude::Gravity(Vec2::ZERO))
            .add_plugins(logic::LogicPlugin)
            .add_plugins((
                population::PopulationPlugin,
                economy::EconomyPlugin,
                objectives::ObjectivesPlugin,
                barricade::BarricadePlugin,
            ))
            .add_systems(Update, day_night_clock.run_if(in_state(Phase::Playing)))
            .add_plugins((control::ControlPlugin, view::ViewPlugin))
            .add_systems(OnEnter(Phase::Loading), start_load)
            .add_systems(Update, poll_load.run_if(in_state(Phase::Loading)))
            .add_systems(Update, watch_game_over.run_if(in_state(Phase::Playing)));
    }
}

#[derive(Resource, Default)]
pub struct MapLoad {
    rx: Option<Mutex<Receiver<LoadMsg>>>,
    pub status: String,
}

enum LoadMsg {
    Progress(String),
    Done(Box<anyhow::Result<Generated>>),
}

fn start_load(cfg: Res<MapConfig>, mut load: ResMut<MapLoad>) {
    let (tx, rx) = channel();
    load.rx = Some(Mutex::new(rx));
    load.status = "Loading map …".into();
    let cfg = cfg.clone();
    let task = move || {
        let ptx = tx.clone();
        let progress = move |m: String| {
            let _ = ptx.send(LoadMsg::Progress(m));
        };
        let result = generate::generate_with_progress(&cfg, &progress);
        let _ = tx.send(LoadMsg::Done(Box::new(result)));
    };
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::spawn(task);
    #[cfg(target_arch = "wasm32")]
    task();
}

fn poll_load(
    mut commands: Commands,
    mut load: ResMut<MapLoad>,
    cfg: Res<MapConfig>,
    mut next: ResMut<NextState<Phase>>,
    images: Option<ResMut<Assets<Image>>>,
    mut cam: Query<&mut Transform, With<Camera2d>>,
) {
    let mut done = None;
    let mut progress = Vec::new();
    let mut dead = false;
    if let Some(rx) = &load.rx {
        let rx = rx.lock().unwrap();
        loop {
            match rx.try_recv() {
                Ok(LoadMsg::Progress(m)) => progress.push(m),
                Ok(LoadMsg::Done(r)) => {
                    done = Some(*r);
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    dead = true;
                    break;
                }
            }
        }
    } else {
        return;
    }
    if let Some(m) = progress.pop() {
        load.status = m;
    }
    if dead {
        load.status = "map load failed (thread died)".into();
        load.rx = None;
        return;
    }
    let Some(result) = done else { return };
    load.rx = None;
    match result {
        Err(e) => {
            load.status = format!("map load error: {e:#}");
            error!("{e:#}");
        }
        Ok(mut g) => {
            setup_session(&mut commands, &cfg, &mut g, images);
            next.set(Phase::Playing);
            // start zoomed in on the squad (units are person-sized now)
            for mut tf in &mut cam {
                tf.translation = Vec3::ZERO;
                tf.scale = Vec3::new(0.4, 0.4, 1.0);
            }
        }
    }
}

/// Spawn world + squad (+ visuals when an image store exists).
pub fn setup_session(
    commands: &mut Commands,
    cfg: &MapConfig,
    g: &mut Generated,
    images: Option<ResMut<Assets<Image>>>,
) {
    let carved = if cfg.enterable && g.rendered.indoor.iter().any(|b| *b) {
        Some(buildings::carve(g))
    } else {
        None
    };
    let mut world = world::build_world(commands, g, carved.as_ref().map(|c| c.sight.clone()));
    let interiors = carved
        .as_ref()
        .map(|c| c.interior_list.clone())
        .unwrap_or_default();
    if let Some(c) = carved {
        world.indoor_id = c.indoor_id;
        world.openings = c.openings;
    }
    // squad spawns around the centre of the map on walkable ground
    let centre = (world.w as f32 / 2.0, world.h as f32 / 2.0);
    let sheets = images.map(|mut images| {
        view::spawn_map_visuals(commands, g, &world, &mut images);
        units::make_sprites(&mut images)
    });
    let mut placed = 0u32;
    let classes = [
        units::Class::Rifleman,
        units::Class::Rifleman,
        units::Class::Rifleman,
        units::Class::Rifleman,
        units::Class::Gunner,
        units::Class::Gunner,
        units::Class::Medic,
        units::Class::Rifleman,
    ];
    let mut rng = logic::SimRng::default();
    'outer: for ring in 0..40i32 {
        for dy in -ring..=ring {
            for dx in -ring..=ring {
                if dx.abs().max(dy.abs()) != ring {
                    continue;
                }
                let cell = world
                    .nav
                    .cell_of(centre.0 + dx as f32 * 6.0, centre.1 + dy as f32 * 6.0);
                if !world.spawnable_cell(cell) {
                    continue;
                }
                let (x, y) = world.nav.centre(cell);
                let class = classes[(placed as usize) % classes.len()];
                let e =
                    units::spawn_soldier(commands, sheets.as_ref(), class, world.to_world(x, y));
                let (a, b) = (rng.next(), rng.next());
                commands.entity(e).insert(units::Dossier {
                    name: units::soldier_name(a, b.wrapping_add(placed as u64)),
                    kills: 0,
                    shots: 0,
                });
                placed += 1;
                if placed >= cfg.squad.max(1) {
                    break 'outer;
                }
            }
        }
    }
    // the sleeping city
    let population = population::population_for(&world, cfg.population);
    let seeded = population::seed(commands, sheets.as_ref(), &world, &mut rng, population);
    log::info!("population: {seeded} sleepers seeded");
    // loot + objectives
    commands.insert_resource(economy::build_sites(&world, &interiors));
    let spawn_world = world.to_world(centre.0, centre.1);
    let objectives = objectives::choose(&world, spawn_world);
    if let Some(o) = objectives.list.last() {
        log::info!(
            "extraction: {} at {:?} ({} objectives)",
            o.name,
            o.pos,
            objectives.list.len()
        );
    }
    commands.insert_resource(objectives);
    commands.insert_resource(rng);
    if let Some(sheets) = sheets {
        commands.insert_resource(sheets);
    }
    commands.insert_resource(world);
    commands.insert_resource(logic::Score::default());
    commands.insert_resource(barricade::Barricades::default());
    commands.insert_resource(director::Director::default());
    commands.insert_resource(DayNight::default());
    commands.insert_resource(population::NoiseMeter::default());
    commands.insert_resource(economy::Stockpile::default());
    commands.insert_resource(logic::SquadBuffs::default());
    commands.insert_resource(logic::Alerts::default());
}

fn watch_game_over(mut over: MessageReader<logic::GameOver>, mut next: ResMut<NextState<Phase>>) {
    if over.read().next().is_some() {
        next.set(Phase::GameOver);
    }
}

// ---------------------------------------------------------------------------
// Headless simulation (`--sim-ticks N`) and stress harness

/// Shared scaffold for all headless harnesses: minimal plugins + the full
/// game logic stack, deterministic 16 ms virtual ticks, no rendering.
/// Public so integration tests can drive custom scenarios.
pub fn headless_app(cfg: &MapConfig) -> App {
    use bevy::app::ScheduleRunnerPlugin;
    use bevy::state::app::StatesPlugin;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(std::time::Duration::ZERO)))
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(16),
        ))
        .add_plugins(bevy::transform::TransformPlugin)
        .add_plugins(bevy::diagnostic::DiagnosticsPlugin)
        .add_plugins(StatesPlugin)
        .init_state::<Phase>()
        .insert_resource(cfg.clone())
        .add_plugins(avian2d::PhysicsPlugins::default())
        .insert_resource(avian2d::prelude::Gravity(Vec2::ZERO))
        .add_plugins(logic::LogicPlugin)
        .add_plugins((
            population::PopulationPlugin,
            economy::EconomyPlugin,
            objectives::ObjectivesPlugin,
            barricade::BarricadePlugin,
            director::DirectorPlugin,
        ))
        .add_systems(Update, day_night_clock);
    app
}

pub fn run_headless_sim(cfg: &MapConfig, ticks: u32) -> anyhow::Result<String> {
    let g = generate::generate(cfg)?;
    let mut app = headless_app(cfg);
    {
        let mut g = g;
        let world_cell = app.world_mut();
        let mut commands_queue = bevy::ecs::world::CommandQueue::default();
        let mut commands = Commands::new(&mut commands_queue, world_cell);
        setup_session(&mut commands, cfg, &mut g, None);
        commands_queue.apply(app.world_mut());
    }
    // scripted order: attack-move the whole squad toward the east edge
    {
        let world = app.world_mut();
        let target = {
            let gw = world.resource::<world::GameWorld>();
            gw.to_world(gw.w as f32 * 0.8, gw.h as f32 * 0.5)
        };
        let mut q =
            world.query_filtered::<&mut units::Orders, bevy::prelude::With<units::Soldier>>();
        for mut o in q.iter_mut(world) {
            o.waypoints.push_back(target);
            o.attack_move = true;
        }
    }
    app.finish();
    app.cleanup();
    for _ in 0..ticks {
        app.update();
    }
    let world = app.world_mut();
    let kills = world.resource::<logic::Score>().kills;
    let stock = {
        let s = world.resource::<economy::Stockpile>();
        (s.ammo as i32, s.meds as i32, s.scrap as i32)
    };
    let dormant = world
        .query_filtered::<(), (
            bevy::prelude::With<units::Enemy>,
            bevy::prelude::With<units::Dormant>,
        )>()
        .iter(world)
        .count();
    let objective = {
        let o = world.resource::<objectives::Objectives>();
        o.current()
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "extracted".into())
    };
    let kinds = {
        let mut n = [0usize; 4];
        let mut q = world.query::<&units::Enemy>();
        for e in q.iter(world) {
            n[match e.kind {
                units::EnemyKind::Shambler => 0,
                units::EnemyKind::Shrieker => 1,
                units::EnemyKind::Runner => 2,
                units::EnemyKind::Brute => 3,
            }] += 1;
        }
        format!("{}sh/{}sk/{}ru/{}br", n[0], n[1], n[2], n[3])
    };
    let director = {
        let d = world.resource::<director::Director>();
        format!("intensity {:.0} ({} actions)", d.intensity, d.actions)
    };
    let soldiers = world
        .query_filtered::<(), bevy::prelude::With<units::Soldier>>()
        .iter(world)
        .count();
    let enemies = world
        .query_filtered::<(), bevy::prelude::With<units::Enemy>>()
        .iter(world)
        .count();
    // invariant: no unit inside a blocked nav cell
    let positions: Vec<Vec2> = world
        .query_filtered::<&Transform, bevy::prelude::With<units::Soldier>>()
        .iter(world)
        .map(|tf| tf.translation.truncate())
        .collect();
    let gw = world.resource::<world::GameWorld>();
    let stuck = positions.iter().filter(|p| !gw.walkable_world(**p)).count();
    Ok(format!(
        "sim: {ticks} ticks, {soldiers} soldiers ({stuck} in blocked cells), {enemies} enemies ({dormant} dormant; {kinds}), {kills} kills, ammo {} meds {} scrap {}, objective: {objective}, director: {director}",
        stock.0, stock.1, stock.2
    ))
}

/// Goal-reaching harness: order the squad to a reachable spot ~a third of the
/// map away and report (mean, max) final distance to it after `ticks`.
/// Exercises A*, clearance, wall-slide and stuck-recovery end to end.
pub fn run_goal_sim(cfg: &MapConfig, ticks: u32) -> anyhow::Result<(f32, f32)> {
    let debug = std::env::var("GOAL_DEBUG").is_ok();
    let mut g = generate::generate(cfg)?;
    // pathfinding harness: an empty world (no sleepers interfering)
    let mut cfg = cfg.clone();
    cfg.population = Some(0);
    let cfg = &cfg;
    let mut app = headless_app(cfg);
    {
        let world_cell = app.world_mut();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        let mut commands = Commands::new(&mut queue, world_cell);
        setup_session(&mut commands, cfg, &mut g, None);
        queue.apply(app.world_mut());
    }
    let target = {
        let world = app.world_mut();
        let start = {
            let mut q = world.query_filtered::<&Transform, bevy::prelude::With<units::Soldier>>();
            q.iter(world)
                .next()
                .map(|t| t.translation.truncate())
                .expect("squad spawned")
        };
        let gw = world.resource::<world::GameWorld>();
        let from = gw.to_map(start);
        // scan for a walkable, reachable spot far from the start
        let mut best: Option<((f32, f32), f32)> = None;
        for cy in (2..gw.nav.h as i32 - 2).step_by(4) {
            for cx in (2..gw.nav.w as i32 - 2).step_by(4) {
                if gw.nav.is_blocked(cx, cy) {
                    continue;
                }
                let p = gw.nav.centre((cx, cy));
                let d = ((p.0 - from.0).powi(2) + (p.1 - from.1).powi(2)).sqrt();
                let span = gw.w.min(gw.h) as f32;
                if d < span * 0.25 || d > span * 0.45 {
                    continue;
                }
                if gw.nav.path(from, p).is_some() && best.map(|(_, bd)| d > bd).unwrap_or(true) {
                    best = Some((p, d));
                }
            }
        }
        let (p, _) = best.expect("a reachable target exists");
        gw.to_world(p.0, p.1)
    };
    {
        let world = app.world_mut();
        let positions: Vec<Vec2> = {
            let mut q = world.query_filtered::<&Transform, bevy::prelude::With<units::Soldier>>();
            q.iter(world).map(|tf| tf.translation.truncate()).collect()
        };
        let paths: Vec<_> = {
            let gw = world.resource::<world::GameWorld>();
            positions
                .iter()
                .map(|pos| {
                    let from = gw.to_map(*pos);
                    let to = gw.to_map(target);
                    gw.nav
                        .path(from, to)
                        .map(|p| {
                            p.iter()
                                .map(|q| gw.to_world(q.0, q.1))
                                .collect::<std::collections::VecDeque<_>>()
                        })
                        .unwrap_or_else(|| std::collections::VecDeque::from([target]))
                })
                .collect()
        };
        let mut q =
            world.query_filtered::<&mut units::Orders, bevy::prelude::With<units::Soldier>>();
        for (mut o, path) in q.iter_mut(world).zip(paths) {
            o.waypoints = path;
            o.attack_move = false;
        }
    }
    app.finish();
    app.cleanup();
    for t in 0..ticks {
        app.update();
        if debug && t % 500 == 0 {
            let world = app.world_mut();
            let mut q = world
                .query_filtered::<(&Transform, &units::Orders), bevy::prelude::With<units::Soldier>>();
            let ds: Vec<(i32, usize, i32)> = q
                .iter(world)
                .map(|(tf, o)| {
                    (
                        tf.translation.truncate().distance(target) as i32,
                        o.waypoints.len(),
                        (o.stuck_t * 10.0) as i32,
                    )
                })
                .collect();
            eprintln!("tick {t}: (dist, wps, stuck) {ds:?}");
        }
    }
    let world = app.world_mut();
    let dists: Vec<f32> = world
        .query_filtered::<&Transform, bevy::prelude::With<units::Soldier>>()
        .iter(world)
        .map(|tf| tf.translation.truncate().distance(target))
        .collect();
    let mean = dists.iter().sum::<f32>() / dists.len().max(1) as f32;
    let max = dists.iter().cloned().fold(0.0, f32::max);
    Ok((mean, max))
}

/// Physics stress harness: spawn `n` extra soldiers bunched on one walkable
/// spot and push them through the streets for `ticks`. Returns
/// (units_in_blocked_cells, units_with_nan_positions).
pub fn run_stress(cfg: &MapConfig, n: u32, ticks: u32) -> anyhow::Result<(usize, usize)> {
    let g = generate::generate(cfg)?;
    let mut app = headless_app(cfg);
    {
        let mut g = g;
        let world_cell = app.world_mut();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        let mut commands = Commands::new(&mut queue, world_cell);
        setup_session(&mut commands, cfg, &mut g, None);
        queue.apply(app.world_mut());
    }
    // pile n riflemen onto one spot and order them across the map
    {
        let world = app.world_mut();
        let (spawn, target) = {
            let gw = world.resource::<world::GameWorld>();
            let c = gw
                .nav
                .nearest_walkable(gw.nav.cell_of(gw.w as f32 * 0.3, gw.h as f32 * 0.5))
                .expect("walkable spawn");
            let (x, y) = gw.nav.centre(c);
            (
                gw.to_world(x, y),
                gw.to_world(gw.w as f32 * 0.75, gw.h as f32 * 0.5),
            )
        };
        let mut queue = bevy::ecs::world::CommandQueue::default();
        let mut commands = Commands::new(&mut queue, world);
        for i in 0..n {
            let jitter = Vec2::new((i % 10) as f32 * 0.5, (i / 10) as f32 * 0.5);
            let e =
                units::spawn_soldier(&mut commands, None, units::Class::Rifleman, spawn + jitter);
            commands.entity(e).insert({
                let mut o = units::Orders::default();
                o.waypoints.push_back(target);
                o.attack_move = false;
                o
            });
        }
        queue.apply(app.world_mut());
    }
    app.finish();
    app.cleanup();
    for _ in 0..ticks {
        app.update();
    }
    let world = app.world_mut();
    let positions: Vec<Vec2> = world
        .query_filtered::<&Transform, bevy::prelude::With<units::Soldier>>()
        .iter(world)
        .map(|tf| tf.translation.truncate())
        .collect();
    let gw = world.resource::<world::GameWorld>();
    let nan = positions
        .iter()
        .filter(|p| !p.x.is_finite() || !p.y.is_finite())
        .count();
    // allow the soft radius: count units whose *centre* is inside a blocked pixel
    let stuck = positions
        .iter()
        .filter(|p| p.x.is_finite() && p.y.is_finite())
        .filter(|p| {
            let (x, y) = gw.to_map(**p);
            let (xi, yi) = (x as i32, y as i32);
            xi >= 0
                && yi >= 0
                && xi < gw.w as i32
                && yi < gw.h as i32
                && gw.blocked[(yi as u32 * gw.w + xi as u32) as usize]
        })
        .count();
    Ok((stuck, nan))
}
