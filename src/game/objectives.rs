//! A run has a destination: mid objectives worth supplies, then an extraction
//! hold. Objectives are picked from real named map features.

use super::economy::Stockpile;
use super::logic::{Alerts, GameOver, Score};
use super::population::Noise;
use super::tuning::*;
use super::units::{Dormant, Enemy, Soldier};
use super::world::GameWorld;
use bevy::prelude::*;

#[derive(Debug, Clone, PartialEq)]
pub enum ObjectiveKind {
    /// search the named place, reward supplies
    Search,
    /// hold the circle for EXTRACT_HOLD_S seconds
    Extract,
}

#[derive(Debug, Clone)]
pub struct Objective {
    pub kind: ObjectiveKind,
    pub name: String,
    pub pos: Vec2,
    pub done: bool,
}

#[derive(Resource, Default)]
pub struct Objectives {
    pub list: Vec<Objective>,
    /// seconds held inside the extraction circle
    pub hold: f32,
    pub alarm_fired: bool,
}

impl Objectives {
    pub fn current(&self) -> Option<&Objective> {
        self.list.iter().find(|o| !o.done)
    }
    pub fn all_done(&self) -> bool {
        self.list.iter().all(|o| o.done)
    }
}

pub struct ObjectivesPlugin;

impl Plugin for ObjectivesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Objectives>()
            .add_systems(Update, progress.run_if(resource_exists::<GameWorld>));
    }
}

/// Pick objectives at map load: extraction = farthest reachable named point
/// from the squad spawn; 1–2 search objectives roughly along the way.
pub fn choose(world: &GameWorld, spawn: Vec2) -> Objectives {
    let from = world.to_map(spawn);
    // named candidates: places (cities/towns) + POIs; reachability via A*
    let mut named: Vec<(String, (f32, f32), f32)> = world
        .places
        .iter()
        .chain(world.pois.iter())
        .filter_map(|(x, y, name)| {
            if name.is_empty() {
                return None;
            }
            let c = world.nav.cell_of(*x, *y);
            let c = world.nearest_spawnable(c)?;
            let p = world.nav.centre(c);
            world.nav.path(from, p)?;
            let d = ((p.0 - from.0).powi(2) + (p.1 - from.1).powi(2)).sqrt();
            Some((name.clone(), p, d))
        })
        .collect();
    named.sort_by(|a, b| a.2.total_cmp(&b.2));
    let extract = named.last().cloned().or_else(|| {
        // fallback: farthest reachable nav cell
        let mut best: Option<((f32, f32), f32)> = None;
        for cy in (2..world.nav.h as i32 - 2).step_by(6) {
            for cx in (2..world.nav.w as i32 - 2).step_by(6) {
                if !world.spawnable_cell((cx, cy)) {
                    continue;
                }
                let p = world.nav.centre((cx, cy));
                let d = ((p.0 - from.0).powi(2) + (p.1 - from.1).powi(2)).sqrt();
                if best.map(|(_, bd)| d > bd).unwrap_or(true) && world.nav.path(from, p).is_some() {
                    best = Some((p, d));
                }
            }
        }
        best.map(|(p, d)| ("the far perimeter".to_string(), p, d))
    });
    let Some((ex_name, ex_pos, ex_dist)) = extract else {
        return Objectives::default();
    };
    let mut list = Vec::new();
    // up to 2 mid objectives at roughly 40% and 70% of the way out
    for frac in [0.4, 0.7] {
        if let Some((name, p, _)) = named
            .iter()
            .filter(|(n, _, d)| {
                *d > ex_dist * (frac - 0.18)
                    && *d < ex_dist * (frac + 0.18)
                    && *n != ex_name
                    && !list.iter().any(|o: &Objective| o.name == *n)
            })
            .min_by(|a, b| {
                (a.2 - ex_dist * frac)
                    .abs()
                    .total_cmp(&(b.2 - ex_dist * frac).abs())
            })
        {
            list.push(Objective {
                kind: ObjectiveKind::Search,
                name: name.clone(),
                pos: world.to_world(p.0, p.1),
                done: false,
            });
        }
    }
    list.push(Objective {
        kind: ObjectiveKind::Extract,
        name: ex_name,
        pos: world.to_world(ex_pos.0, ex_pos.1),
        done: false,
    });
    Objectives {
        list,
        hold: 0.0,
        alarm_fired: false,
    }
}

#[allow(clippy::too_many_arguments)]
fn progress(
    time: Res<Time>,
    mut objectives: ResMut<Objectives>,
    mut stock: ResMut<Stockpile>,
    mut score: ResMut<Score>,
    mut alerts: ResMut<Alerts>,
    mut noise: MessageWriter<Noise>,
    mut over: MessageWriter<GameOver>,
    soldiers: Query<&Transform, With<Soldier>>,
    awake_enemies: Query<&Transform, (With<Enemy>, Without<Dormant>, Without<Soldier>)>,
) {
    let squad: Vec<Vec2> = soldiers.iter().map(|t| t.translation.truncate()).collect();
    if squad.is_empty() {
        return;
    }
    let objectives = &mut *objectives;
    let Some(current) = objectives.list.iter_mut().find(|o| !o.done) else {
        return;
    };
    match current.kind {
        ObjectiveKind::Search => {
            if squad
                .iter()
                .any(|s| s.distance(current.pos) < MID_OBJECTIVE_RADIUS)
            {
                current.done = true;
                stock.ammo += MID_REWARD_AMMO;
                stock.meds += MID_REWARD_MEDS;
                stock.scrap += MID_REWARD_SCRAP;
                log::info!("objective secured: {}", current.name);
            }
        }
        ObjectiveKind::Extract => {
            let holding = squad
                .iter()
                .any(|s| s.distance(current.pos) < EXTRACT_RADIUS);
            if holding && !objectives.alarm_fired {
                objectives.alarm_fired = true;
                // the city hears the flare go up
                noise.write(Noise {
                    pos: current.pos,
                    radius: EXTRACT_ALARM_RADIUS,
                });
                alerts.push(current.pos);
            }
            let contested = awake_enemies
                .iter()
                .any(|t| t.translation.truncate().distance(current.pos) < EXTRACT_RADIUS);
            if holding && !contested {
                objectives.hold += time.delta_secs();
            }
            if objectives.hold >= EXTRACT_HOLD_S {
                current.done = true;
                score.victory = true;
                over.write(GameOver { victory: true });
            }
        }
    }
}
