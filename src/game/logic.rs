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
            .init_resource::<Alerts>()
            .add_message::<TracerFx>()
            .add_message::<GameOver>()
            .add_systems(
                Update,
                (
                    soldier_move,
                    enemy_perception,
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

    /// Stateless per-position hash (stable sideways bias per enemy path).
    pub fn hash_dir(v: Vec2) -> u64 {
        let x = (v.x * 8.0) as i64 as u64;
        let y = (v.y * 8.0) as i64 as u64;
        let mut h = x.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ y.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
        h ^= h >> 31;
        h
    }
}

/// Noise / sightings the horde investigates: positions with a time-to-live.
/// Gunfire is loud; enemies that spot a soldier report the position. Enemies
/// have no global knowledge — they only chase alerts and what they see.
#[derive(Resource, Default)]
pub struct Alerts(pub Vec<(Vec2, f32)>);

pub const ALERT_TTL: f32 = 12.0;
/// enemies within this range investigate an alert
pub const ALERT_RADIUS: f32 = 170.0;
/// how far an enemy can see a soldier (with line of sight)
pub const ENEMY_SIGHT: f32 = 48.0;
/// gunfire is heard this far
pub const GUNSHOT_RADIUS: f32 = 110.0;

impl Alerts {
    pub fn push(&mut self, pos: Vec2) {
        // merge with a nearby existing alert instead of piling up
        for (p, ttl) in self.0.iter_mut() {
            if p.distance(pos) < 24.0 {
                *p = pos;
                *ttl = ALERT_TTL;
                return;
            }
        }
        self.0.push((pos, ALERT_TTL));
        if self.0.len() > 64 {
            self.0.remove(0);
        }
    }

    pub fn decay(&mut self, dt: f32) {
        for (_, ttl) in self.0.iter_mut() {
            *ttl -= dt;
        }
        self.0.retain(|(_, ttl)| *ttl > 0.0);
    }

    pub fn nearest(&self, pos: Vec2, max: f32) -> Option<Vec2> {
        self.0
            .iter()
            .map(|(p, _)| *p)
            .filter(|p| p.distance(pos) < max)
            .min_by(|a, b| a.distance_squared(pos).total_cmp(&b.distance_squared(pos)))
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

/// arrival radius for the final destination …
const ARRIVE: f32 = 3.0;
/// … and for intermediate waypoints (looser: they only guide the route, and
/// a wall-adjacent waypoint can be physically unreachable closer than ~3 px)
const ARRIVE_MID: f32 = 4.6;

fn soldier_move(
    time: Res<Time>,
    world: Res<GameWorld>,
    mut q: Query<(
        &Soldier,
        &mut Orders,
        &Transform,
        &mut Position,
        &mut LinearVelocity,
    )>,
) {
    let dt = time.delta_secs();
    // unit positions for snap-collision checks (see the wedge ladder)
    let all_units: Vec<Vec2> = q
        .iter()
        .map(|(_, _, tf, _, _)| tf.translation.truncate())
        .collect();
    for (s, mut orders, tf, mut phys_pos, mut vel) in &mut q {
        orders.replan_cooldown = (orders.replan_cooldown - dt).max(0.0);
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
            let last = orders.waypoints.len() == 1;
            let arrive = if last { ARRIVE } else { ARRIVE_MID };
            if pos.distance(wp) < arrive {
                orders.waypoints.pop_front();
            } else {
                break;
            }
        }
        let Some(&wp) = orders.waypoints.front() else {
            vel.0 = Vec2::ZERO;
            orders.stuck_t = 0.0;
            orders.stuck_total = 0.0;
            orders.best_goal_dist = 0.0;
            continue;
        };
        // stuck detection: measure *progress toward the waypoint*, not raw
        // motion — a unit orbiting an unreachable corner moves plenty while
        // getting nowhere
        orders.last_pos = pos;
        let wp_dist = pos.distance(wp);
        if orders.cur_wp != wp {
            orders.cur_wp = wp;
            orders.best_wp_dist = wp_dist;
        }
        if wp_dist < orders.best_wp_dist - 0.4 {
            orders.best_wp_dist = wp_dist;
            orders.stuck_t = (orders.stuck_t - dt * 2.0).max(0.0);
        } else {
            orders.stuck_t += dt;
        }
        // the wedge meter grows while locally stuck AND not getting closer to
        // the final destination — waypoint pops/replans can't launder it, but
        // an honest detour (goal distance temporarily rising while the unit
        // moves freely) doesn't count either
        let goal = *orders.waypoints.back().unwrap_or(&wp);
        let goal_dist = pos.distance(goal);
        if orders.best_goal_dist == 0.0 || goal_dist < orders.best_goal_dist - 1.5 {
            if orders.best_goal_dist != 0.0 {
                orders.stuck_total = 0.0;
            }
            orders.best_goal_dist = goal_dist;
        } else if orders.stuck_t > 0.25 {
            orders.stuck_total += dt;
        }
        // escalation ladder against wedging
        if orders.stuck_total > 2.5 {
            // last resort after 2.5 s of futility: snap onto the waypoint's
            // cell (a few px — invisible at pixel scale, guaranteed progress)
            orders.stuck_total = 0.0;
            orders.stuck_t = 0.0;
            let (wx, wy) = world.to_map(wp);
            let target_cell = world.nav.cell_of(wx, wy);
            let cell = if world.spawnable_cell(target_cell) {
                Some(target_cell)
            } else {
                world.nearest_spawnable(target_cell)
            };
            if let Some(c) = cell {
                let (cx, cy) = world.nav.centre(c);
                let w = world.to_world(cx, cy);
                // never snap onto another unit: the depenetration impulse can
                // hurl someone through a wall. Wedged-in-a-crowd resolves by
                // itself once the crowd moves; only snap into free space.
                let occupied = all_units
                    .iter()
                    .any(|u| *u != pos && u.distance(w) < UNIT_RADIUS * 2.2);
                if !occupied {
                    if std::env::var("GOAL_DEBUG").is_ok() {
                        eprintln!("SNAP {} -> {:?} (wp {:?})", pos, w, wp);
                    }
                    phys_pos.0 = w; // avian's Position is the source of truth
                    vel.0 = Vec2::ZERO;
                    orders.waypoints.pop_front();
                    orders.replan_cooldown = 0.0;
                }
            }
            continue;
        }
        if orders.stuck_t > 1.5 {
            // skip a waypoint the physics won't let us touch
            orders.stuck_t = 0.9;
            if orders.waypoints.len() > 1 {
                orders.waypoints.pop_front();
            }
        } else if orders.stuck_t > 0.6 && orders.replan_cooldown <= 0.0 {
            // keep the ladder climbing: don't zero stuck_t here
            orders.replan_cooldown = 0.6;
            // replan the whole remaining route from where we actually are
            let goal = *orders.waypoints.back().unwrap_or(&wp);
            let from = world.to_map(pos);
            let to = world.to_map(goal);
            if let Some(path) = world.nav.path(from, to) {
                orders.waypoints = path.iter().map(|p| world.to_world(p.0, p.1)).collect();
            }
            // sideways nudge to break the clinch (alternate sides over time)
            let side = Vec2::new(-(wp - pos).y, (wp - pos).x).normalize_or_zero();
            let sign = if (time.elapsed_secs() * 2.0) as i64 % 2 == 0 {
                1.0
            } else {
                -1.0
            };
            vel.0 = side * sign * s.stats.speed;
            continue;
        }
        let dir = (wp - pos).normalize_or_zero();
        // wall-slide steering: don't grind into corners the path clipped
        let dir = world.slide(pos, dir, UNIT_RADIUS + 1.6);
        vel.0 = dir * s.stats.speed;
        // if the next leg is not directly walkable (physics pushed us off the
        // path), re-plan — rate-limited so a jammed crowd doesn't A* every frame
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
}

// ---------------------------------------------------------------------------
// Enemies

/// Enemies notice soldiers they can see, hear gunfire, and share alerts.
fn enemy_perception(
    time: Res<Time>,
    world: Res<GameWorld>,
    mut alerts: ResMut<Alerts>,
    mut tracer_noise: MessageReader<TracerFx>,
    soldiers: Query<&Transform, With<Soldier>>,
    mut enemies: Query<(&mut Enemy, &Transform)>,
) {
    alerts.decay(time.delta_secs());
    // gunfire is loud
    for t in tracer_noise.read() {
        if !t.heal {
            alerts.push(t.from);
        }
    }
    let squad: Vec<Vec2> = soldiers.iter().map(|t| t.translation.truncate()).collect();
    for (mut e, tf) in &mut enemies {
        let pos = tf.translation.truncate();
        // direct sighting (needs line of sight through walls/windows)
        let seen = squad
            .iter()
            .filter(|s| s.distance(pos) < ENEMY_SIGHT)
            .find(|s| {
                let a = world.to_map(pos);
                let b = world.to_map(**s);
                Fog::line_of_sight(&world.sight_blocked, world.w, world.h, a, b)
            })
            .copied();
        if let Some(s) = seen {
            e.alert = Some(s);
            alerts.push(s); // shriek: nearby enemies join in via the alert map
            continue;
        }
        // reached a stale alert with nothing there → forget it
        if let Some(a) = e.alert
            && pos.distance(a) < 10.0
        {
            e.alert = None;
        }
        // pick up nearby noise
        if e.alert.is_none() {
            e.alert = alerts.nearest(pos, ALERT_RADIUS);
        }
    }
}

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
    alerts: Res<Alerts>,
) {
    timer.0.tick(time.delta());
    if !timer.0.just_finished() && !world.flow.dir.is_empty() {
        return;
    }
    // the horde converges on what it heard/saw, not on live positions
    let goals: Vec<(f32, f32)> = alerts.0.iter().map(|(p, _)| world.to_map(*p)).collect();
    if goals.is_empty() {
        world.flow.dir.clear();
        return;
    }
    world.flow = FlowField::compute(&world.nav, &goals);
}

const ENEMY_CONTACT: f32 = UNIT_RADIUS + ENEMY_RADIUS + 1.5;
const ENEMY_CHASE: f32 = 30.0;

fn enemy_ai(
    time: Res<Time>,
    world: Res<GameWorld>,
    mut rng: ResMut<SimRng>,
    mut enemies: Query<(&mut Enemy, &Transform, &mut LinearVelocity)>,
    mut soldiers: Query<(&Transform, &mut Health), (With<Soldier>, Without<Enemy>)>,
) {
    let targets: Vec<Vec2> = soldiers
        .iter()
        .map(|(t, _)| t.translation.truncate())
        .collect();
    let dt = time.delta_secs();
    for (mut e, tf, mut vel) in &mut enemies {
        e.cooldown.tick(time.delta());
        let pos = tf.translation.truncate();
        // melee whatever is in reach, regardless of alert state
        let nearest = targets
            .iter()
            .copied()
            .min_by(|a, b| a.distance_squared(pos).total_cmp(&b.distance_squared(pos)));
        if let Some(t) = nearest {
            let d = pos.distance(t);
            if d < ENEMY_CONTACT {
                vel.0 = Vec2::ZERO;
                if e.cooldown.is_finished() {
                    e.cooldown.reset();
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
            // close-range chase only when it can actually see the target
            if d < ENEMY_CHASE {
                let a = world.to_map(pos);
                let b = world.to_map(t);
                if Fog::line_of_sight(&world.sight_blocked, world.w, world.h, a, b) {
                    let d = world.slide(pos, (t - pos).normalize_or_zero(), ENEMY_RADIUS + 1.4);
                    vel.0 = d * e.speed;
                    continue;
                }
            }
        }
        let dir = if e.alert.is_some() {
            // investigate: follow the flow field toward the alert cluster,
            // with a per-enemy sideways bias so hordes spread out
            let (mx, my) = world.to_map(pos);
            let (fx, fy) = world.flow.sample(&world.nav, mx, my);
            let flow = Vec2::new(fx, -fy);
            if flow == Vec2::ZERO {
                // no field (alert expired) — walk straight at the memory
                e.alert
                    .map(|a| (a - pos).normalize_or_zero())
                    .unwrap_or(Vec2::ZERO)
            } else {
                let side = Vec2::new(-flow.y, flow.x)
                    * ((SimRng::hash_dir(pos) % 200) as f32 / 1000.0 - 0.1);
                (flow + side).normalize_or_zero()
            }
        } else {
            // idle wander: drift, turn now and then, bounce off walls
            e.wander_t -= dt;
            let ahead = pos + e.wander * 6.0;
            if e.wander_t <= 0.0 || e.wander == Vec2::ZERO || !world.walkable_world(ahead) {
                let a = rng.f32() * std::f32::consts::TAU;
                e.wander = Vec2::new(a.cos(), a.sin());
                e.wander_t = 1.5 + rng.f32() * 3.0;
            }
            e.wander * 0.45
        };
        let dir = world.slide(pos, dir, ENEMY_RADIUS + 1.4);
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
    mut alerts: ResMut<Alerts>,
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
                    &mut alerts,
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
    alerts: &mut Alerts,
    soldiers: &Query<&Transform, With<Soldier>>,
    n: u32,
    hp: f32,
    speed: f32,
    damage: f32,
) -> u32 {
    let squad: Vec<Vec2> = soldiers.iter().map(|t| t.translation.truncate()).collect();
    // seed a couple of noisy "scent" alerts so the flow field routes the
    // horde roughly toward the squad without perfect knowledge
    if !squad.is_empty() {
        let centroid = squad.iter().copied().sum::<Vec2>() / squad.len() as f32;
        for _ in 0..3 {
            let noise = Vec2::new(rng.f32() - 0.5, rng.f32() - 0.5) * 140.0;
            alerts.push(centroid + noise);
        }
    }
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
        let Some(c) = world.nearest_spawnable(c) else {
            continue;
        };
        let (wx, wy) = world.nav.centre(c);
        x = wx;
        y = wy;
        let pos = world.to_world(x, y);
        if squad.iter().any(|s| s.distance(pos) < 90.0) {
            continue; // don't spawn on top of the squad
        }
        // the horde smells the living: a rough (noisy) idea of the squad
        let centroid = squad.iter().copied().sum::<Vec2>() / squad.len().max(1) as f32;
        let noise = Vec2::new(rng.f32() - 0.5, rng.f32() - 0.5) * 160.0;
        let jitter = 0.8 + rng.f32() * 0.45;
        let a = rng.f32() * std::f32::consts::TAU;
        spawn_enemy(
            commands,
            sheets,
            pos,
            hp,
            speed * jitter,
            damage,
            Some(centroid + noise),
            Vec2::new(a.cos(), a.sin()),
        );
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
        // fall back to a random open spot
        let c = (
            (rng.f32() * world.nav.w as f32) as i32,
            (rng.f32() * world.nav.h as f32) as i32,
        );
        world.nearest_spawnable(c).map(|c| world.nav.centre(c))
    } else {
        let (x, y, _) = world.pois[rng.range(world.pois.len())];
        world
            .nearest_spawnable(world.nav.cell_of(x, y))
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
            .any(|(t, _)| t.translation.truncate().distance(dp) < 5.0);
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
