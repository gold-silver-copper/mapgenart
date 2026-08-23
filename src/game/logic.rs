//! Head-independent game simulation: orders & movement, horde AI on the flow
//! field, combat (hitscan with line of sight), medic healing, waves, supply
//! drops, fog updates and win/lose. No rendering types in here — the same
//! systems drive the window build and `--sim-ticks` headless runs.

use super::fog::Fog;
use super::nav::FlowField;
use super::units::*;
use super::world::GameWorld;
use avian2d::prelude::*;
use bevy::prelude::*;
use std::collections::VecDeque;

pub struct LogicPlugin;

impl Plugin for LogicPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Score>()
            .init_resource::<WaveState>()
            .init_resource::<SimRng>()
            .init_resource::<SquadBuffs>()
            .add_message::<TracerFx>()
            .add_message::<GameOver>()
            .add_systems(
                Update,
                (
                    soldier_move,
                    refresh_flow,
                    enemy_ai,
                    soldier_combat,
                    medic_heal,
                    resolve_deaths,
                    wave_director,
                    pickups,
                    fog_update,
                )
                    .chain()
                    .run_if(resource_exists::<GameWorld>),
            );
    }
}

// ---------------------------------------------------------------------------
// Resources / events

#[derive(Resource, Default)]
pub struct Score {
    pub kills: u32,
    pub waves_survived: u32,
}

#[derive(Resource)]
pub struct WaveState {
    pub wave: u32,
    /// time until the next wave while between waves; None while a wave is live
    pub countdown: Option<Timer>,
    pub alive: u32,
}

impl Default for WaveState {
    fn default() -> Self {
        WaveState {
            wave: 0,
            countdown: Some(Timer::from_seconds(6.0, TimerMode::Once)),
            alive: 0,
        }
    }
}

/// Squad-wide pickups.
#[derive(Resource, Default)]
pub struct SquadBuffs {
    pub damage_mult: f32,
}

impl SquadBuffs {
    pub fn damage(&self, base: f32) -> f32 {
        base * (1.0 + self.damage_mult)
    }
}

/// Deterministic tiny RNG (no rand dependency; survives headless runs).
#[derive(Resource)]
pub struct SimRng(pub u64);

impl Default for SimRng {
    fn default() -> Self {
        SimRng(0x2545F4914F6CDD1D)
    }
}

impl SimRng {
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn f32(&mut self) -> f32 {
        (self.next() % 10_000) as f32 / 10_000.0
    }
    pub fn range(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// A shot was fired (visual tracer).
#[derive(Message)]
pub struct TracerFx {
    pub from: Vec2,
    pub to: Vec2,
    pub heal: bool,
}

#[derive(Message)]
pub struct GameOver {
    pub kills: u32,
    pub waves: u32,
}

/// Marks damage for the flash effect.
#[derive(Component)]
pub struct Hurt(pub Timer);

// ---------------------------------------------------------------------------
// Movement

const ARRIVE: f32 = 3.0;

fn soldier_move(
    time: Res<Time>,
    world: Res<GameWorld>,
    mut q: Query<(&Soldier, &mut Orders, &Transform, &mut LinearVelocity)>,
) {
    for (s, mut orders, tf, mut vel) in &mut q {
        orders.replan_cooldown = (orders.replan_cooldown - time.delta_secs()).max(0.0);
        if orders.hold {
            vel.0 = Vec2::ZERO;
            continue;
        }
        // patrol loops its two endpoints
        if orders.waypoints.is_empty()
            && let Some((a, b)) = orders.patrol
        {
            orders.waypoints.push_back(b);
            orders.patrol = Some((b, a));
        }
        let pos = tf.translation.truncate();
        while let Some(&wp) = orders.waypoints.front() {
            if pos.distance(wp) < ARRIVE {
                orders.waypoints.pop_front();
            } else {
                break;
            }
        }
        match orders.waypoints.front() {
            Some(&wp) => {
                let dir = (wp - pos).normalize_or_zero();
                vel.0 = dir * s.stats.speed;
                // if the next leg is not directly walkable (physics pushed us
                // off the path), re-plan — rate-limited so a jammed crowd
                // doesn't run A* every frame
                let (ax, ay) = world.to_map(pos);
                let (bx, by) = world.to_map(wp);
                if orders.replan_cooldown <= 0.0 && !world.nav.line_walkable((ax, ay), (bx, by)) {
                    orders.replan_cooldown = 0.5;
                    if let Some(path) = world.nav.path((ax, ay), (bx, by)) {
                        let rest: Vec<Vec2> = orders.waypoints.iter().skip(1).copied().collect();
                        orders.waypoints = path
                            .iter()
                            .map(|p| world.to_world(p.0, p.1))
                            .collect::<VecDeque<_>>();
                        orders.waypoints.extend(rest);
                    }
                }
            }
            None => vel.0 = Vec2::ZERO,
        }
    }
}

// ---------------------------------------------------------------------------
// Enemies

#[derive(Resource)]
pub struct FlowTimer(pub Timer);

impl Default for FlowTimer {
    fn default() -> Self {
        FlowTimer(Timer::from_seconds(0.5, TimerMode::Repeating))
    }
}

fn refresh_flow(
    time: Res<Time>,
    mut timer: Local<FlowTimer>,
    mut world: ResMut<GameWorld>,
    soldiers: Query<&Transform, With<Soldier>>,
) {
    timer.0.tick(time.delta());
    if !timer.0.just_finished() && !world.flow.dir.is_empty() {
        return;
    }
    let goals: Vec<(f32, f32)> = soldiers
        .iter()
        .map(|t| world.to_map(t.translation.truncate()))
        .collect();
    if goals.is_empty() {
        return;
    }
    world.flow = FlowField::compute(&world.nav, &goals);
}

const ENEMY_CONTACT: f32 = UNIT_RADIUS + ENEMY_RADIUS + 1.5;
const ENEMY_CHASE: f32 = 30.0;

fn enemy_ai(
    time: Res<Time>,
    world: Res<GameWorld>,
    mut enemies: Query<(&mut Enemy, &Transform, &mut LinearVelocity)>,
    mut soldiers: Query<(&Transform, &mut Health), (With<Soldier>, Without<Enemy>)>,
) {
    let targets: Vec<Vec2> = soldiers
        .iter()
        .map(|(t, _)| t.translation.truncate())
        .collect();
    for (mut e, tf, mut vel) in &mut enemies {
        e.cooldown.tick(time.delta());
        let pos = tf.translation.truncate();
        let nearest = targets
            .iter()
            .copied()
            .min_by(|a, b| a.distance_squared(pos).total_cmp(&b.distance_squared(pos)));
        let mut dir = Vec2::ZERO;
        if let Some(t) = nearest {
            let d = pos.distance(t);
            if d < ENEMY_CONTACT {
                vel.0 = Vec2::ZERO;
                if e.cooldown.is_finished() {
                    e.cooldown.reset();
                    // hit the nearest soldier
                    if let Some((_, mut hp)) = soldiers.iter_mut().min_by(|a, b| {
                        a.0.translation
                            .truncate()
                            .distance_squared(pos)
                            .total_cmp(&b.0.translation.truncate().distance_squared(pos))
                    }) {
                        hp.hp -= e.damage;
                    }
                }
                continue;
            }
            if d < ENEMY_CHASE {
                dir = (t - pos).normalize_or_zero();
            }
        }
        if dir == Vec2::ZERO {
            let (mx, my) = world.to_map(pos);
            let (fx, fy) = world.flow.sample(&world.nav, mx, my);
            dir = Vec2::new(fx, -fy); // map y-down → world y-up
        }
        vel.0 = dir * e.speed;
    }
}

// ---------------------------------------------------------------------------
// Combat

fn soldier_combat(
    time: Res<Time>,
    world: Res<GameWorld>,
    buffs: Res<SquadBuffs>,
    mut commands: Commands,
    mut tracers: MessageWriter<TracerFx>,
    mut soldiers: Query<(&mut Soldier, &Orders, &Transform, &mut LinearVelocity)>,
    mut enemies: Query<(Entity, &Transform, &mut Health), With<Enemy>>,
) {
    for (mut s, orders, tf, mut vel) in &mut soldiers {
        s.cooldown.tick(time.delta());
        if s.stats.damage <= 0.0 {
            continue;
        }
        let pos = tf.translation.truncate();
        let moving_freely = !orders.waypoints.is_empty() && !orders.attack_move && !orders.hold;
        if moving_freely {
            continue; // plain move: don't stop to shoot
        }
        let (sx, sy) = world.to_map(pos);
        let mut best: Option<(Entity, Vec2, f32)> = None;
        for (ent, etf, _) in &enemies {
            let ep = etf.translation.truncate();
            let d = pos.distance(ep);
            if d > s.stats.range {
                continue;
            }
            let (ex, ey) = world.to_map(ep);
            if !world.fog.is_visible(ex, ey) {
                continue;
            }
            if !Fog::line_of_sight(&world.sight_blocked, world.w, world.h, (sx, sy), (ex, ey)) {
                continue;
            }
            if best.map(|(_, _, bd)| d < bd).unwrap_or(true) {
                best = Some((ent, ep, d));
            }
        }
        if let Some((ent, ep, _)) = best {
            // attack-move / stationary: halt while firing
            vel.0 = Vec2::ZERO;
            if s.cooldown.is_finished() {
                s.cooldown.reset();
                if let Ok((_, _, mut hp)) = enemies.get_mut(ent) {
                    hp.hp -= buffs.damage(s.stats.damage);
                    commands
                        .entity(ent)
                        .insert(Hurt(Timer::from_seconds(0.1, TimerMode::Once)));
                }
                tracers.write(TracerFx {
                    from: pos,
                    to: ep,
                    heal: false,
                });
            }
        }
    }
}

fn medic_heal(
    time: Res<Time>,
    mut tracers: MessageWriter<TracerFx>,
    mut q: Query<(&Soldier, &Transform)>,
    mut wounded: Query<(&Soldier, &Transform, &mut Health)>,
) {
    let medics: Vec<(Vec2, f32, f32)> = q
        .iter_mut()
        .filter(|(s, _)| s.stats.heal > 0.0)
        .map(|(s, t)| (t.translation.truncate(), s.stats.range, s.stats.heal))
        .collect();
    for (s, tf, mut hp) in &mut wounded {
        if s.stats.heal > 0.0 || hp.hp >= hp.max {
            continue;
        }
        let pos = tf.translation.truncate();
        for (mp, range, heal) in &medics {
            if mp.distance(pos) <= *range {
                hp.hp = (hp.hp + heal * time.delta_secs()).min(hp.max);
                if (hp.hp - hp.max).abs() > 1.0 && (time.elapsed_secs() * 2.0).fract() < 0.1 {
                    tracers.write(TracerFx {
                        from: *mp,
                        to: pos,
                        heal: true,
                    });
                }
                break;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_deaths(
    mut commands: Commands,
    mut score: ResMut<Score>,
    mut wave: ResMut<WaveState>,
    mut over: MessageWriter<GameOver>,
    sheets: Option<Res<SpriteSheets>>,
    enemies: Query<(Entity, &Transform, &Health), With<Enemy>>,
    soldiers: Query<(Entity, &Health), With<Soldier>>,
) {
    for (ent, tf, hp) in &enemies {
        if hp.hp <= 0.0 {
            score.kills += 1;
            wave.alive = wave.alive.saturating_sub(1);
            let mut c = commands.spawn((
                Corpse {
                    timer: Timer::from_seconds(15.0, TimerMode::Once),
                },
                Transform::from_translation(tf.translation.truncate().extend(1.0)),
            ));
            if let Some(sheets) = &sheets {
                c.insert(Sprite::from_image(sheets.corpse.clone()));
            }
            commands.entity(ent).despawn();
        }
    }
    let mut any_alive = false;
    for (ent, hp) in &soldiers {
        if hp.hp <= 0.0 {
            commands.entity(ent).despawn();
        } else {
            any_alive = true;
        }
    }
    if !any_alive && soldiers.iter().count() > 0 {
        // all remaining are dead this frame
        over.write(GameOver {
            kills: score.kills,
            waves: score.waves_survived,
        });
    } else if !any_alive && score.kills > 0 {
        over.write(GameOver {
            kills: score.kills,
            waves: score.waves_survived,
        });
    }
}

// ---------------------------------------------------------------------------
// Waves & supplies

#[allow(clippy::too_many_arguments)]
fn wave_director(
    time: Res<Time>,
    mut commands: Commands,
    mut wave: ResMut<WaveState>,
    mut score: ResMut<Score>,
    mut rng: ResMut<SimRng>,
    world: Res<GameWorld>,
    sheets: Option<Res<SpriteSheets>>,
    soldiers: Query<&Transform, With<Soldier>>,
    enemies: Query<(), With<Enemy>>,
) {
    let live = enemies.iter().count() as u32;
    wave.alive = live;
    match &mut wave.countdown {
        Some(t) => {
            t.tick(time.delta());
            if t.just_finished() {
                wave.wave += 1;
                let n = 6 + wave.wave * 5;
                let hp = 26.0 * 1.12f32.powi(wave.wave as i32 - 1);
                let speed = (16.0 + wave.wave as f32 * 1.2).min(34.0);
                let damage = 6.0 + wave.wave as f32 * 0.8;
                let spawned = spawn_wave(
                    &mut commands,
                    sheets.as_deref(),
                    &world,
                    &mut rng,
                    &soldiers,
                    n,
                    hp,
                    speed,
                    damage,
                );
                log::info!("wave {}: {spawned} enemies (hp {hp:.0})", wave.wave);
                wave.countdown = None;
            }
        }
        None => {
            if live == 0 {
                score.waves_survived = wave.wave;
                wave.countdown = Some(Timer::from_seconds(10.0, TimerMode::Once));
                spawn_supplies(&mut commands, sheets.as_deref(), &world, &mut rng);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_wave(
    commands: &mut Commands,
    sheets: Option<&SpriteSheets>,
    world: &GameWorld,
    rng: &mut SimRng,
    soldiers: &Query<&Transform, With<Soldier>>,
    n: u32,
    hp: f32,
    speed: f32,
    damage: f32,
) -> u32 {
    let squad: Vec<Vec2> = soldiers.iter().map(|t| t.translation.truncate()).collect();
    let mut spawned = 0;
    let mut guard = 0;
    while spawned < n && guard < n * 50 {
        guard += 1;
        // random point along a random edge, nudged inward until walkable
        let (mut x, mut y) = match rng.range(4) {
            0 => (rng.f32() * world.w as f32, 3.0),
            1 => (rng.f32() * world.w as f32, world.h as f32 - 4.0),
            2 => (3.0, rng.f32() * world.h as f32),
            _ => (world.w as f32 - 4.0, rng.f32() * world.h as f32),
        };
        let c = world.nav.cell_of(x, y);
        let Some(c) = world.nav.nearest_walkable(c) else {
            continue;
        };
        let (wx, wy) = world.nav.centre(c);
        x = wx;
        y = wy;
        let pos = world.to_world(x, y);
        if squad.iter().any(|s| s.distance(pos) < 90.0) {
            continue; // don't spawn on top of the squad
        }
        spawn_enemy(commands, sheets, pos, hp, speed, damage);
        spawned += 1;
    }
    spawned
}

fn spawn_supplies(
    commands: &mut Commands,
    sheets: Option<&SpriteSheets>,
    world: &GameWorld,
    rng: &mut SimRng,
) {
    let kinds = [SupplyKind::Medkit, SupplyKind::Ammo, SupplyKind::Recruit];
    let kind = kinds[rng.range(kinds.len())];
    let pos = if world.pois.is_empty() {
        // fall back to a random walkable spot
        let c = (
            (rng.f32() * world.nav.w as f32) as i32,
            (rng.f32() * world.nav.h as f32) as i32,
        );
        world.nav.nearest_walkable(c).map(|c| world.nav.centre(c))
    } else {
        let (x, y, _) = world.pois[rng.range(world.pois.len())];
        world
            .nav
            .nearest_walkable(world.nav.cell_of(x, y))
            .map(|c| world.nav.centre(c))
    };
    let Some((x, y)) = pos else { return };
    let mut e = commands.spawn((
        SupplyDrop { kind },
        Transform::from_translation(world.to_world(x, y).extend(3.0)),
    ));
    if let Some(sheets) = sheets {
        e.insert(Sprite::from_image(sheets.supply.clone()));
    }
    log::info!("supply drop ({kind:?}) at map ({x:.0},{y:.0})");
}

#[allow(clippy::too_many_arguments)]
fn pickups(
    mut commands: Commands,
    world: Res<GameWorld>,
    mut buffs: ResMut<SquadBuffs>,
    sheets: Option<Res<SpriteSheets>>,
    drops: Query<(Entity, &SupplyDrop, &Transform)>,
    mut soldiers: Query<(&Transform, &mut Health), With<Soldier>>,
) {
    for (ent, drop, dtf) in &drops {
        let dp = dtf.translation.truncate();
        let taker = soldiers
            .iter()
            .any(|(t, _)| t.translation.truncate().distance(dp) < 8.0);
        if !taker {
            continue;
        }
        match drop.kind {
            SupplyKind::Medkit => {
                for (_, mut hp) in &mut soldiers {
                    hp.hp = (hp.hp + 40.0).min(hp.max);
                }
            }
            SupplyKind::Ammo => buffs.damage_mult += 0.15,
            SupplyKind::Recruit => {
                let spawn = dp + Vec2::new(6.0, 0.0);
                let spawn = if world.walkable_world(spawn) {
                    spawn
                } else {
                    dp
                };
                spawn_soldier(&mut commands, sheets.as_deref(), Class::Rifleman, spawn);
            }
        }
        commands.entity(ent).despawn();
    }
}

// ---------------------------------------------------------------------------
// Fog

#[derive(Resource)]
pub struct FogTimer(pub Timer);

impl Default for FogTimer {
    fn default() -> Self {
        FogTimer(Timer::from_seconds(0.15, TimerMode::Repeating))
    }
}

fn fog_update(
    time: Res<Time>,
    mut timer: Local<FogTimer>,
    mut world: ResMut<GameWorld>,
    soldiers: Query<&Transform, With<Soldier>>,
) {
    timer.0.tick(time.delta());
    if !timer.0.just_finished() {
        return;
    }
    let viewers: Vec<(f32, f32)> = soldiers
        .iter()
        .map(|t| world.to_map(t.translation.truncate()))
        .collect();
    let world = &mut *world;
    let sight = std::mem::take(&mut world.sight_blocked);
    world.fog.update(&sight, &viewers, VISION_RADIUS);
    world.sight_blocked = sight;
}
