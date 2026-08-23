//! "Last Light" — real-time tactics on OSM pixel-art maps. StarCraft-style
//! squad control, hordes, fog of war; no base building.

pub mod buildings;
pub mod control;
pub mod fog;
pub mod logic;
pub mod nav;
pub mod units;
pub mod view;
pub mod world;

use crate::config::MapConfig;
use crate::generate::{self, Generated};
use bevy::prelude::*;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};

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
    let world = world::build_world(commands, g, carved.map(|c| c.sight));
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
                units::spawn_soldier(commands, sheets.as_ref(), class, world.to_world(x, y));
                placed += 1;
                if placed >= cfg.squad.max(1) {
                    break 'outer;
                }
            }
        }
    }
    if let Some(sheets) = sheets {
        commands.insert_resource(sheets);
    }
    commands.insert_resource(world);
    commands.insert_resource(logic::Score::default());
    commands.insert_resource(logic::WaveState::default());
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

pub fn run_headless_sim(cfg: &MapConfig, ticks: u32) -> anyhow::Result<String> {
    use bevy::app::ScheduleRunnerPlugin;
    use bevy::state::app::StatesPlugin;
    let g = generate::generate(cfg)?;
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
        .add_plugins(logic::LogicPlugin);
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
    let wave_no = world.resource::<logic::WaveState>().wave;
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
        "sim: {ticks} ticks, wave {wave_no}, {soldiers} soldiers ({stuck} in blocked cells), {enemies} enemies alive, {kills} kills"
    ))
}

/// Goal-reaching harness: order the squad to a reachable spot ~a third of the
/// map away and report (mean, max) final distance to it after `ticks`.
/// Exercises A*, clearance, wall-slide and stuck-recovery end to end.
pub fn run_goal_sim(cfg: &MapConfig, ticks: u32) -> anyhow::Result<(f32, f32)> {
    let debug = std::env::var("GOAL_DEBUG").is_ok();
    // pathfinding harness: no enemy waves interfering
    fn no_waves(mut wave: ResMut<logic::WaveState>) {
        wave.countdown = Some(Timer::from_seconds(1.0e6, TimerMode::Once));
    }
    use bevy::app::ScheduleRunnerPlugin;
    use bevy::state::app::StatesPlugin;
    let mut g = generate::generate(cfg)?;
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
        .add_systems(First, no_waves);
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
    use bevy::app::ScheduleRunnerPlugin;
    use bevy::state::app::StatesPlugin;
    let g = generate::generate(cfg)?;
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
        .add_plugins(logic::LogicPlugin);
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
