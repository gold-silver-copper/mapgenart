//! Intensity director (Left 4 Dead style): keeps tension oscillating.
//! Lull breaker wakes a scout pack when it's been quiet too long; the relief
//! valve eases pressure when the squad is being ground down; a pre-night
//! build-up puts distant movement on the minimap before dark.

use super::DayNight;
use super::logic::{Alerts, SimRng};
use super::objectives::Objectives;
use super::population::{CalmTimer, Noise};
use super::tuning::*;
use super::units::{Dormant, Enemy, Health, Soldier, wake_enemy};
use super::world::GameWorld;
use bevy::prelude::*;

#[derive(Resource, Default)]
pub struct Director {
    /// 0..100: nearby awake enemies + recent damage + recent noise, decaying
    pub intensity: f32,
    pub quiet_for: f32,
    pub high_for: f32,
    pub cooldown: f32,
    /// relief valve active for this many more seconds
    pub relief: f32,
    pub prenight_done: bool,
    pub actions: u32,
    last_squad_hp: f32,
    /// noise heard near the squad since the last director tick
    heard: f32,
}

impl Director {
    /// wake-radius multiplier applied by the population systems
    pub fn wake_scale(&self) -> f32 {
        if self.relief > 0.0 { 0.5 } else { 1.0 }
    }
    /// calm-down speed multiplier
    pub fn calm_scale(&self) -> f32 {
        if self.relief > 0.0 { 3.0 } else { 1.0 }
    }
}

pub struct DirectorPlugin;

impl Plugin for DirectorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Director>().add_systems(
            Update,
            (listen, direct)
                .chain()
                .run_if(resource_exists::<GameWorld>),
        );
    }
}

/// Accumulate noise near the squad (separate system: a reader and a writer
/// of the same message type can't share one system).
fn listen(
    mut d: ResMut<Director>,
    mut noise_in: MessageReader<Noise>,
    soldiers: Query<&Transform, With<Soldier>>,
) {
    let squad: Vec<Vec2> = soldiers.iter().map(|t| t.translation.truncate()).collect();
    if squad.is_empty() {
        noise_in.clear();
        return;
    }
    let centroid = squad.iter().copied().sum::<Vec2>() / squad.len() as f32;
    for n in noise_in.read() {
        if n.pos.distance(centroid) < 200.0 {
            d.heard += n.radius * 0.05;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn direct(
    time: Res<Time>,
    mut commands: Commands,
    mut d: ResMut<Director>,
    mut rng: ResMut<SimRng>,
    mut alerts: ResMut<Alerts>,
    mut noise: MessageWriter<Noise>,
    world: Res<GameWorld>,
    daynight: Option<Res<DayNight>>,
    objectives: Option<Res<Objectives>>,
    soldiers: Query<(&Transform, &Health), With<Soldier>>,
    awake: Query<(&Transform, &Enemy), (Without<Dormant>, Without<Soldier>)>,
    dormant: Query<(Entity, &Transform, &Enemy), With<Dormant>>,
) {
    let dt = time.delta_secs();
    let squad: Vec<Vec2> = soldiers
        .iter()
        .map(|(t, _)| t.translation.truncate())
        .collect();
    if squad.is_empty() {
        return;
    }
    let centroid = squad.iter().copied().sum::<Vec2>() / squad.len() as f32;
    // --- intensity sampling
    // only *hunting* enemies count — a sleeper that woke and wanders off is
    // not pressure
    let near = awake
        .iter()
        .filter(|(t, e)| e.alert.is_some() && t.translation.truncate().distance(centroid) < 150.0)
        .count() as f32;
    let hp_now: f32 = soldiers.iter().map(|(_, h)| h.hp).sum();
    let damage = (d.last_squad_hp - hp_now).max(0.0);
    d.last_squad_hp = hp_now;
    let heard = std::mem::take(&mut d.heard);
    let target = (near * 6.0 + damage * 1.5 + heard).min(100.0);
    if target > d.intensity {
        d.intensity = target;
    } else {
        d.intensity = (d.intensity - INTENSITY_DECAY * dt).max(target * 0.5);
    }
    d.cooldown = (d.cooldown - dt).max(0.0);
    d.relief = (d.relief - dt).max(0.0);
    if d.intensity < 5.0 {
        d.quiet_for += dt;
        d.high_for = 0.0;
    } else {
        d.quiet_for = 0.0;
        if d.intensity > DIRECTOR_HIGH_INTENSITY {
            d.high_for += dt;
        } else {
            d.high_for = 0.0;
        }
    }
    let night = daynight.as_deref().map(|x| x.is_night).unwrap_or(false);
    let extracting = objectives
        .as_deref()
        .map(|o| o.alarm_fired && !o.all_done())
        .unwrap_or(false);
    let total_max: f32 = soldiers.iter().map(|(_, h)| h.max).sum();

    // --- lull breaker: a scout pack from 120–200 px, alerted to the squad
    if d.quiet_for > DIRECTOR_LULL_S && d.cooldown <= 0.0 && !extracting {
        let want = DIRECTOR_SCOUT_MIN
            + rng.range((DIRECTOR_SCOUT_MAX - DIRECTOR_SCOUT_MIN + 1) as usize) as u32;
        let mut candidates: Vec<(Entity, Vec2, super::units::EnemyKind)> = dormant
            .iter()
            .map(|(e, t, en)| (e, t.translation.truncate(), en.kind))
            .filter(|(_, p, _)| {
                let dd = p.distance(centroid);
                (DIRECTOR_SCOUT_NEAR..=DIRECTOR_SCOUT_FAR).contains(&dd)
            })
            .collect();
        candidates.sort_by(|a, b| a.1.distance(centroid).total_cmp(&b.1.distance(centroid)));
        let mut woke = 0;
        for (e, p, kind) in candidates.into_iter().take(want as usize) {
            wake_enemy(&mut commands, e, kind);
            commands.entity(e).insert(CalmTimer(0.0));
            noise.write(Noise {
                pos: p,
                radius: 12.0,
            });
            woke += 1;
        }
        if woke > 0 {
            alerts.push(centroid + Vec2::new(rng.f32() - 0.5, rng.f32() - 0.5) * 30.0);
            d.cooldown = DIRECTOR_COOLDOWN_S;
            d.quiet_for = 0.0;
            d.actions += 1;
            log::info!("director: lull breaker — {woke} scouts sent");
        }
    }
    // --- relief valve
    if d.high_for > DIRECTOR_HIGH_FOR_S
        && hp_now < total_max * 0.5
        && d.relief <= 0.0
        && d.cooldown <= 0.0
        && !night
        && !extracting
    {
        d.relief = DIRECTOR_RELIEF_S;
        d.cooldown = DIRECTOR_COOLDOWN_S;
        d.high_for = 0.0;
        d.actions += 1;
        log::info!("director: relief valve open for {DIRECTOR_RELIEF_S}s");
    }
    // --- pre-night build-up: distant sleepers stir 60 s before dark
    if let Some(dn) = daynight.as_deref() {
        if !dn.is_night && DAY_S - dn.t < DIRECTOR_PRENIGHT_S && !d.prenight_done {
            d.prenight_done = true;
            let mut far: Vec<(Entity, super::units::EnemyKind)> = dormant
                .iter()
                .filter(|(_, t, _)| t.translation.truncate().distance(centroid) > 220.0)
                .map(|(e, _, en)| (e, en.kind))
                .collect();
            let n = far.len().min(6);
            for _ in 0..n {
                let i = rng.range(far.len());
                let (e, kind) = far.swap_remove(i);
                wake_enemy(&mut commands, e, kind);
                commands.entity(e).insert(CalmTimer(0.0));
            }
            d.actions += 1;
            log::info!("director: pre-night build-up — {n} distant sleepers stir");
        }
        if dn.is_night {
            d.prenight_done = false;
        }
    }
    let _ = &world;
}
