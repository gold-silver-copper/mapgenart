//! The noise-driven world: a fixed sleeping population seeded at map load,
//! woken by gunfire, shrieks and sight — no timed waves.

use super::logic::{Alerts, SimRng};
use super::tuning::*;
use super::units::*;
use super::world::GameWorld;
use super::{DayNight, fog::Fog};
use bevy::prelude::*;

/// Something loud happened at `pos` (radius already includes rank quieting).
#[derive(Message)]
pub struct Noise {
    pub pos: Vec2,
    pub radius: f32,
}

/// HUD noise meter (recent noise made by the squad).
#[derive(Resource, Default)]
pub struct NoiseMeter(pub f32);

/// Time since an awake enemy last had a reason to stay awake.
#[derive(Component, Default)]
pub struct CalmTimer(pub f32);

pub struct PopulationPlugin;

impl Plugin for PopulationPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Noise>()
            .init_resource::<NoiseMeter>()
            .add_systems(
                Update,
                (wake_by_noise, wake_by_sight, calm_down, meter_decay)
                    .chain()
                    .run_if(resource_exists::<GameWorld>),
            );
    }
}

/// Seed the sleeping population (called from session setup).
pub fn seed(
    commands: &mut Commands,
    sheets: Option<&SpriteSheets>,
    world: &GameWorld,
    rng: &mut SimRng,
    population: u32,
) -> u32 {
    let indoor_cells: Vec<usize> = world
        .indoor_id
        .iter()
        .enumerate()
        .filter(|(i, id)| **id != u32::MAX && !world.blocked[*i])
        .map(|(i, _)| i)
        .collect();
    let mut placed = 0;
    let mut guard = 0;
    while placed < population && guard < population * 30 {
        guard += 1;
        let indoors = !indoor_cells.is_empty() && rng.f32() < POP_INDOOR_FRACTION;
        let (x, y) = if indoors {
            let i = indoor_cells[rng.range(indoor_cells.len())];
            (
                (i as u32 % world.w) as f32 + 0.5,
                (i as u32 / world.w) as f32 + 0.5,
            )
        } else {
            let c = (
                (rng.f32() * world.nav.w as f32) as i32,
                (rng.f32() * world.nav.h as f32) as i32,
            );
            let Some(c) = world.nearest_spawnable(c) else {
                continue;
            };
            world.nav.centre(c)
        };
        // outdoor sleepers must be in the open world; indoor ones may lurk in
        // courtyard-locked interiors (you can still shoot them through windows)
        if !indoors {
            let cell = world.nav.cell_of(x, y);
            if !world.spawnable_cell(cell) {
                continue;
            }
        }
        let pos = world.to_world(x, y);
        let hp = 26.0 + rng.f32() * 14.0;
        let speed = 15.0 + rng.f32() * 9.0;
        let damage = 6.0 + rng.f32() * 3.0;
        let kind = EnemyKind::roll(rng.f32());
        spawn_dormant(commands, sheets, pos, kind, hp, speed, damage);
        placed += 1;
    }
    placed
}

/// Population for a map, from walkable area (override with `--population`).
pub fn population_for(world: &GameWorld, override_n: Option<u32>) -> u32 {
    if let Some(n) = override_n {
        return n;
    }
    let walkable = world.blocked.iter().filter(|b| !**b).count() as f32;
    ((walkable / 10_000.0) * POP_PER_10K_WALKABLE) as u32
}

/// Wake-radius multiplier for the time of day.
pub fn wake_mult(is_night: bool) -> f32 {
    if is_night { NIGHT_WAKE_MULT } else { 1.0 }
}

fn wake_radius_mult(dn: Option<&DayNight>) -> f32 {
    wake_mult(dn.map(|d| d.is_night).unwrap_or(false))
}

fn director_scale(d: Option<&super::director::Director>) -> f32 {
    d.map(|d| d.wake_scale()).unwrap_or(1.0)
}

#[allow(clippy::too_many_arguments)]
fn wake_by_noise(
    mut commands: Commands,
    mut noise: MessageReader<Noise>,
    mut meter: ResMut<NoiseMeter>,
    mut alerts: ResMut<Alerts>,
    daynight: Option<Res<DayNight>>,
    dormant: Query<(Entity, &Transform, &Enemy), With<Dormant>>,
    director: Option<Res<super::director::Director>>,
    mut woken_shriek: Local<Vec<Vec2>>,
) {
    let night = daynight.as_deref().map(|d| d.is_night).unwrap_or(false);
    let mult = wake_radius_mult(daynight.as_deref()) * director_scale(director.as_deref());
    woken_shriek.clear();
    for n in noise.read() {
        meter.0 = (meter.0 + n.radius * 0.35).min(100.0);
        alerts.push(n.pos);
        // runners only stir for loud noise (a rifle shot or more) or at night
        let loud = n.radius >= NOISE_RIFLE * 0.9;
        for (ent, tf, e) in &dormant {
            if e.kind == EnemyKind::Runner && !loud && !night {
                continue;
            }
            let p = tf.translation.truncate();
            if p.distance(n.pos) <= n.radius * mult {
                wake_enemy(&mut commands, ent, e.kind);
                commands.entity(ent).insert(CalmTimer(0.0));
                woken_shriek.push(p);
            }
        }
    }
    // shriek chain: one hop with a smaller radius (falloff — a single shot
    // wakes a bounded neighbourhood, not the whole map)
    for shriek in woken_shriek.iter() {
        alerts.push(*shriek);
        for (ent, tf, e) in &dormant {
            if tf.translation.truncate().distance(*shriek) <= SHRIEK_RADIUS * mult {
                wake_enemy(&mut commands, ent, e.kind);
                commands.entity(ent).insert(CalmTimer(0.0));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn wake_by_sight(
    mut commands: Commands,
    world: Res<GameWorld>,
    time: Res<Time>,
    mut rng: ResMut<SimRng>,
    daynight: Option<Res<DayNight>>,
    soldiers: Query<&Transform, With<Soldier>>,
    dormant: Query<(Entity, &Transform, &Enemy), With<Dormant>>,
    director: Option<Res<super::director::Director>>,
) {
    let mult = wake_radius_mult(daynight.as_deref()) * director_scale(director.as_deref());
    let night = daynight.map(|d| d.is_night).unwrap_or(false);
    let squad: Vec<Vec2> = soldiers.iter().map(|t| t.translation.truncate()).collect();
    for (ent, tf, e) in &dormant {
        // runners sleep through daylight sightings
        if e.kind == EnemyKind::Runner && !night {
            continue;
        }
        let p = tf.translation.truncate();
        let seen = squad.iter().any(|s| {
            s.distance(p) < DORMANT_SIGHT * mult
                && Fog::line_of_sight(
                    &world.sight_blocked,
                    world.w,
                    world.h,
                    world.to_map(p),
                    world.to_map(*s),
                )
        });
        let restless = night && rng.f32() < NIGHT_SELF_WAKE_P * time.delta_secs() * 60.0;
        if seen || restless {
            wake_enemy(&mut commands, ent, e.kind);
            commands.entity(ent).insert(CalmTimer(0.0));
        }
    }
}

/// Awake, alert-less, unseen enemies drift back to sleep (never at night).
fn calm_down(
    mut commands: Commands,
    time: Res<Time>,
    daynight: Option<Res<DayNight>>,
    director: Option<Res<super::director::Director>>,
    mut awake: Query<(Entity, &Enemy, &mut CalmTimer), Without<Dormant>>,
) {
    if daynight.map(|d| d.is_night).unwrap_or(false) {
        return;
    }
    let speed = director.map(|d| d.calm_scale()).unwrap_or(1.0);
    for (ent, e, mut calm) in &mut awake {
        if e.alert.is_some() {
            calm.0 = 0.0;
            continue;
        }
        calm.0 += time.delta_secs() * speed;
        if calm.0 > CALM_AFTER_S {
            sleep_enemy(&mut commands, ent);
        }
    }
}

fn meter_decay(time: Res<Time>, mut meter: ResMut<NoiseMeter>) {
    meter.0 = (meter.0 - NOISE_METER_DECAY * time.delta_secs()).max(0.0);
}
