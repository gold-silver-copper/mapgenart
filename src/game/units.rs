//! Soldiers, enemies, stats and programmatically generated pixel sprites.

use super::world::Layer;
use avian2d::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Rifleman,
    Gunner,
    Medic,
}

impl Class {
    pub fn stats(self) -> Stats {
        match self {
            Class::Rifleman => Stats {
                hp: 100.0,
                speed: 34.0,
                range: 60.0,
                damage: 12.0,
                cooldown: 0.7,
                heal: 0.0,
            },
            Class::Gunner => Stats {
                hp: 130.0,
                speed: 28.0,
                range: 48.0,
                damage: 3.5,
                cooldown: 0.09,
                heal: 0.0,
            },
            Class::Medic => Stats {
                hp: 80.0,
                speed: 34.0,
                range: 26.0,
                damage: 0.0,
                cooldown: 0.4,
                heal: 9.0,
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Stats {
    pub hp: f32,
    pub speed: f32,
    pub range: f32,
    pub damage: f32,
    pub cooldown: f32,
    /// hp per second restored to nearby allies (medic)
    pub heal: f32,
}

#[derive(Component)]
pub struct Soldier {
    pub class: Class,
    pub stats: Stats,
    pub cooldown: Timer,
}

/// Identity and service record (ranks make a soldier stronger and quieter).
#[derive(Component)]
pub struct Dossier {
    pub name: String,
    pub kills: u32,
    pub shots: u32,
}

impl Dossier {
    pub fn rank(&self) -> u32 {
        crate::game::tuning::RANK_KILLS
            .iter()
            .filter(|k| self.kills >= **k)
            .count() as u32
    }
    pub fn damage_mult(&self) -> f32 {
        1.0 + self.rank() as f32 * crate::game::tuning::RANK_DAMAGE_BONUS
    }
    pub fn noise_mult(&self) -> f32 {
        (1.0 - self.rank() as f32 * crate::game::tuning::RANK_NOISE_CUT).max(0.5)
    }
}

/// Deterministic name generator (feed it SimRng values).
pub fn soldier_name(a: u64, b: u64) -> String {
    const FIRST: [&str; 24] = [
        "Reyes",
        "Okafor",
        "Tran",
        "Silva",
        "Novak",
        "Ito",
        "Marsh",
        "Duarte",
        "Kim",
        "Volkov",
        "Ince",
        "Baptiste",
        "Haas",
        "Oduya",
        "Lindqvist",
        "Marino",
        "Sato",
        "Kelly",
        "Dube",
        "Farah",
        "Quinn",
        "Aldana",
        "Petrov",
        "Nakamura",
    ];
    const NICK: [&str; 12] = [
        "Ace", "Doc", "Flint", "Mole", "Patch", "Ghost", "Brick", "Swift", "Hawk", "Lucky", "Tiny",
        "Rook",
    ];
    format!(
        "{} \"{}\"",
        FIRST[(a % 24) as usize],
        NICK[(b % 12) as usize]
    )
}

/// A sleeping enemy: no physics, no AI — just a body in the world until noise
/// or sight wakes it.
#[derive(Component)]
pub struct Dormant;

/// Enemy archetypes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EnemyKind {
    Shambler,
    /// fragile; screams a huge noise when it dies
    Shrieker,
    /// fast, low hp; wakes only at night or from loud noise
    Runner,
    /// slow tank; the barricade breaker
    Brute,
}

impl EnemyKind {
    /// deterministic pick from a 0..1 roll using the tuning ratios
    pub fn roll(r: f32) -> EnemyKind {
        use crate::game::tuning::*;
        if r < RATIO_BRUTE {
            EnemyKind::Brute
        } else if r < RATIO_BRUTE + RATIO_RUNNER {
            EnemyKind::Runner
        } else if r < RATIO_BRUTE + RATIO_RUNNER + RATIO_SHRIEKER {
            EnemyKind::Shrieker
        } else {
            EnemyKind::Shambler
        }
    }

    /// (hp, speed, damage) from shambler base values
    pub fn stats(self, hp: f32, speed: f32, damage: f32) -> (f32, f32, f32) {
        use crate::game::tuning::*;
        match self {
            EnemyKind::Shambler => (hp, speed, damage),
            EnemyKind::Shrieker => (SHRIEKER_HP, speed * SHRIEKER_SPEED_MULT, damage * 0.7),
            EnemyKind::Runner => (hp * RUNNER_HP_MULT, speed * RUNNER_SPEED_MULT, damage),
            EnemyKind::Brute => (
                hp * BRUTE_HP_MULT,
                speed * BRUTE_SPEED_MULT,
                damage * BRUTE_DAMAGE_MULT,
            ),
        }
    }

    pub fn radius(self) -> f32 {
        match self {
            EnemyKind::Brute => ENEMY_RADIUS * 1.8,
            _ => ENEMY_RADIUS,
        }
    }
}

#[derive(Component)]
pub struct Enemy {
    pub kind: EnemyKind,
    pub damage: f32,
    pub speed: f32,
    pub cooldown: Timer,
    /// last known / suspected target position (world coords)
    pub alert: Option<Vec2>,
    /// current idle-wander direction and time until it changes
    pub wander: Vec2,
    pub wander_t: f32,
    /// unstick: time without progress, and an escape burst overriding flow
    pub stuck_t: f32,
    pub last_pos: Vec2,
    pub burst: Vec2,
    pub burst_t: f32,
}

#[derive(Component)]
pub struct Health {
    pub hp: f32,
    pub max: f32,
}

#[derive(Component, Default)]
pub struct Selected;

/// Current order queue for a soldier.
#[derive(Component, Default)]
pub struct Orders {
    pub waypoints: VecDeque<Vec2>,
    pub attack_move: bool,
    pub hold: bool,
    /// endpoints for patrol (loops)
    pub patrol: Option<(Vec2, Vec2)>,
    /// seconds until the next A* re-plan is allowed (crowd-control)
    pub replan_cooldown: f32,
    /// stuck detection: time spent making no progress toward the waypoint
    pub stuck_t: f32,
    /// cumulative wedged time (decays); drives the escalation ladder
    pub stuck_total: f32,
    pub last_pos: Vec2,
    /// closest we've been to the current waypoint (resets per waypoint)
    pub best_wp_dist: f32,
    pub cur_wp: Vec2,
    /// closest we've been to the final destination (ratchet)
    pub best_goal_dist: f32,
}

#[derive(Component)]
pub struct Corpse {
    pub timer: Timer,
}

#[derive(Component)]
pub struct SupplyDrop {
    pub kind: SupplyKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupplyKind {
    Medkit,
    Ammo,
    Recruit,
}

/// Generated sprite handles.
#[derive(Resource)]
pub struct SpriteSheets {
    pub rifleman: Handle<Image>,
    pub gunner: Handle<Image>,
    pub medic: Handle<Image>,
    /// awake (upright) sprites per archetype
    pub enemy: Handle<Image>,
    pub shrieker: Handle<Image>,
    pub runner: Handle<Image>,
    pub brute: Handle<Image>,
    /// slumped (dormant) sprites per archetype
    pub enemy_asleep: Handle<Image>,
    pub shrieker_asleep: Handle<Image>,
    pub runner_asleep: Handle<Image>,
    pub brute_asleep: Handle<Image>,
    pub corpse: Handle<Image>,
    pub supply: Handle<Image>,
}

impl SpriteSheets {
    pub fn enemy_sprite(&self, kind: EnemyKind, awake: bool) -> Handle<Image> {
        match (kind, awake) {
            (EnemyKind::Shambler, true) => self.enemy.clone(),
            (EnemyKind::Shambler, false) => self.enemy_asleep.clone(),
            (EnemyKind::Shrieker, true) => self.shrieker.clone(),
            (EnemyKind::Shrieker, false) => self.shrieker_asleep.clone(),
            (EnemyKind::Runner, true) => self.runner.clone(),
            (EnemyKind::Runner, false) => self.runner_asleep.clone(),
            (EnemyKind::Brute, true) => self.brute.clone(),
            (EnemyKind::Brute, false) => self.brute_asleep.clone(),
        }
    }
}

pub const UNIT_RADIUS: f32 = 1.4;
pub const ENEMY_RADIUS: f32 = 1.2;
pub const VISION_RADIUS: f32 = 55.0;

const T: [u8; 4] = [0, 0, 0, 0];

/// Build a 7×7 top-down soldier facing +X: shoulders, helmet, weapon.
/// Proportional to the streets: a person is ~1.5 px wide at 2–3 m/px.
fn soldier_sprite(helmet: [u8; 4], body: [u8; 4], gun: bool) -> (Vec<u8>, u32) {
    const N: u32 = 7;
    let mut px = vec![T; (N * N) as usize];
    let set = |px: &mut Vec<[u8; 4]>, x: i32, y: i32, c: [u8; 4]| {
        if (0..N as i32).contains(&x) && (0..N as i32).contains(&y) {
            px[(y * N as i32 + x) as usize] = c;
        }
    };
    let c = (N / 2) as i32;
    // shoulders (perpendicular to facing)
    for dy in -2..=2 {
        set(&mut px, c, c + dy, body);
    }
    set(&mut px, c - 1, c - 1, body);
    set(&mut px, c - 1, c + 1, body);
    // helmet
    set(&mut px, c, c, helmet);
    set(&mut px, c - 1, c, helmet);
    if gun {
        for dx in 1..=2 {
            set(&mut px, c + dx, c - 1, [40, 40, 40, 255]);
        }
    }
    (px.into_iter().flatten().collect(), N)
}

/// Round blob enemy: `n` px square, `core` centre colour, `skin` body colour;
/// `slumped` draws it flattened (a sleeper on the ground).
fn blob_sprite(n: u32, core: [u8; 4], skin: [u8; 4], slumped: bool) -> (Vec<u8>, u32) {
    let mut px = vec![T; (n * n) as usize];
    let c = (n / 2) as i32;
    let r2 = ((n / 2) as i32).pow(2).max(2);
    for y in 0..n as i32 {
        for x in 0..n as i32 {
            let (dx, dy) = (x - c, y - c);
            // slumped: squash vertically, stretch horizontally
            let (ex, ey) = if slumped {
                (dx as f32 * 0.8, dy as f32 * 1.9)
            } else {
                (dx as f32, dy as f32)
            };
            let d2 = (ex * ex + ey * ey) as i32;
            px[(y * n as i32 + x) as usize] = if dx == 0 && dy == 0 {
                core
            } else if d2 <= r2 {
                skin
            } else {
                T
            };
        }
    }
    (px.into_iter().flatten().collect(), n)
}

fn corpse_sprite() -> (Vec<u8>, u32) {
    const N: u32 = 5;
    let mut px = vec![T; (N * N) as usize];
    for (x, y) in [(1, 2), (2, 2), (3, 3), (2, 3), (3, 1)] {
        px[(y * N + x) as usize] = [70, 45, 45, 255];
    }
    (px.into_iter().flatten().collect(), N)
}

fn supply_sprite() -> (Vec<u8>, u32) {
    const N: u32 = 5;
    let mut px = vec![T; (N * N) as usize];
    for y in 0..5u32 {
        for x in 0..5u32 {
            px[(y * N + x) as usize] = [200, 170, 60, 255];
        }
    }
    for i in 0..5u32 {
        px[(2 * N + i) as usize] = [120, 90, 30, 255];
        px[(i * N + 2) as usize] = [120, 90, 30, 255];
    }
    (px.into_iter().flatten().collect(), N)
}

fn image(images: &mut Assets<Image>, data: (Vec<u8>, u32)) -> Handle<Image> {
    let (bytes, n) = data;
    let mut img = Image::new(
        Extent3d {
            width: n,
            height: n,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        bytes,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.sampler = ImageSampler::nearest();
    images.add(img)
}

pub fn make_sprites(images: &mut Assets<Image>) -> SpriteSheets {
    SpriteSheets {
        rifleman: image(
            images,
            soldier_sprite([60, 90, 60, 255], [90, 110, 80, 255], true),
        ),
        gunner: image(
            images,
            soldier_sprite([70, 70, 100, 255], [95, 95, 120, 255], true),
        ),
        medic: image(
            images,
            soldier_sprite([200, 200, 200, 255], [180, 90, 90, 255], false),
        ),
        enemy: image(
            images,
            blob_sprite(5, [150, 40, 40, 255], [95, 60, 50, 255], false),
        ),
        shrieker: image(
            images,
            blob_sprite(5, [200, 200, 220, 255], [170, 165, 185, 255], false),
        ),
        runner: image(
            images,
            blob_sprite(5, [40, 40, 45, 255], [60, 55, 65, 255], false),
        ),
        brute: image(
            images,
            blob_sprite(9, [120, 30, 30, 255], [80, 50, 40, 255], false),
        ),
        enemy_asleep: image(
            images,
            blob_sprite(5, [120, 40, 40, 255], [85, 55, 45, 255], true),
        ),
        shrieker_asleep: image(
            images,
            blob_sprite(5, [180, 180, 200, 255], [150, 145, 165, 255], true),
        ),
        runner_asleep: image(
            images,
            blob_sprite(5, [40, 40, 45, 255], [55, 50, 60, 255], true),
        ),
        brute_asleep: image(
            images,
            blob_sprite(9, [100, 30, 30, 255], [70, 45, 35, 255], true),
        ),
        corpse: image(images, corpse_sprite()),
        supply: image(images, supply_sprite()),
    }
}

// Bundle helpers ------------------------------------------------------------

pub fn spawn_soldier(
    commands: &mut Commands,
    sheets: Option<&SpriteSheets>,
    class: Class,
    pos: Vec2,
) -> Entity {
    let stats = class.stats();
    let mut e = commands.spawn((
        Soldier {
            class,
            stats,
            cooldown: Timer::from_seconds(stats.cooldown, TimerMode::Once),
        },
        Health {
            hp: stats.hp,
            max: stats.hp,
        },
        Orders::default(),
        RigidBody::Dynamic,
        Collider::circle(UNIT_RADIUS),
        LockedAxes::ROTATION_LOCKED,
        LinearDamping(8.0),
        CollisionLayers::new(Layer::Unit, [Layer::World, Layer::Unit, Layer::Enemy]),
        Transform::from_translation(pos.extend(5.0)),
    ));
    if let Some(sheets) = sheets {
        let img = match class {
            Class::Rifleman => sheets.rifleman.clone(),
            Class::Gunner => sheets.gunner.clone(),
            Class::Medic => sheets.medic.clone(),
        };
        e.insert(Sprite::from_image(img));
    }
    e.id()
}

/// A sleeping body: sprite only, no physics — woken via [`wake_enemy`].
#[allow(clippy::too_many_arguments)]
pub fn spawn_dormant(
    commands: &mut Commands,
    sheets: Option<&SpriteSheets>,
    pos: Vec2,
    kind: EnemyKind,
    hp: f32,
    speed: f32,
    damage: f32,
) -> Entity {
    let (hp, speed, damage) = kind.stats(hp, speed, damage);
    let mut e = commands.spawn((
        Enemy {
            kind,
            damage,
            speed,
            cooldown: Timer::from_seconds(0.8, TimerMode::Once),
            alert: None,
            wander: Vec2::ZERO,
            wander_t: 2.0,
            stuck_t: 0.0,
            last_pos: Vec2::ZERO,
            burst: Vec2::ZERO,
            burst_t: 0.0,
        },
        Dormant,
        Health { hp, max: hp },
        Transform::from_translation(pos.extend(4.0)),
        Visibility::Hidden,
    ));
    if let Some(sheets) = sheets {
        e.insert(Sprite::from_image(sheets.enemy_sprite(kind, false)));
    }
    e.id()
}

/// Attach physics + AI to a dormant enemy (it wakes up).
pub fn wake_enemy(commands: &mut Commands, entity: Entity, kind: EnemyKind) {
    let mut e = commands.entity(entity);
    e.remove::<Dormant>().insert((
        RigidBody::Dynamic,
        Collider::circle(kind.radius()),
        LockedAxes::ROTATION_LOCKED,
        LinearDamping(8.0),
        CollisionLayers::new(Layer::Enemy, [Layer::World, Layer::Unit, Layer::Enemy]),
        JustWoke(0.6),
    ));
    if kind == EnemyKind::Brute {
        e.insert(Mass(crate::game::tuning::BRUTE_MASS));
    }
}

/// Just woke up: shows the "!" and the stand-up frame swap for a moment.
#[derive(Component)]
pub struct JustWoke(pub f32);

/// Strip physics from a calmed enemy (it goes back to sleep).
pub fn sleep_enemy(commands: &mut Commands, entity: Entity) {
    commands.entity(entity).insert(Dormant).remove::<(
        RigidBody,
        Collider,
        LockedAxes,
        LinearDamping,
        CollisionLayers,
        LinearVelocity,
        Mass,
        JustWoke,
    )>();
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_enemy(
    commands: &mut Commands,
    sheets: Option<&SpriteSheets>,
    pos: Vec2,
    kind: EnemyKind,
    hp: f32,
    speed: f32,
    damage: f32,
    alert: Option<Vec2>,
    wander: Vec2,
) -> Entity {
    let (hp, speed, damage) = kind.stats(hp, speed, damage);
    let mut e = commands.spawn((
        Enemy {
            kind,
            damage,
            speed,
            cooldown: Timer::from_seconds(0.8, TimerMode::Once),
            alert,
            wander,
            wander_t: 2.0,
            stuck_t: 0.0,
            last_pos: Vec2::ZERO,
            burst: Vec2::ZERO,
            burst_t: 0.0,
        },
        Health { hp, max: hp },
        RigidBody::Dynamic,
        Collider::circle(kind.radius()),
        LockedAxes::ROTATION_LOCKED,
        LinearDamping(8.0),
        CollisionLayers::new(Layer::Enemy, [Layer::World, Layer::Unit, Layer::Enemy]),
        Transform::from_translation(pos.extend(4.0)),
        Visibility::Hidden,
    ));
    if let Some(sheets) = sheets {
        e.insert(Sprite::from_image(sheets.enemy_sprite(kind, true)));
    }
    e.id()
}
