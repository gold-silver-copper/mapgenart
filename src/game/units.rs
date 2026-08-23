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

#[derive(Component)]
pub struct Enemy {
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
    pub enemy: Handle<Image>,
    pub corpse: Handle<Image>,
    pub supply: Handle<Image>,
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

fn enemy_sprite() -> (Vec<u8>, u32) {
    const N: u32 = 5;
    let mut px = vec![T; (N * N) as usize];
    let c = (N / 2) as i32;
    for y in 0..N as i32 {
        for x in 0..N as i32 {
            let (dx, dy) = (x - c, y - c);
            let d2 = dx * dx + dy * dy;
            px[(y * N as i32 + x) as usize] = if d2 == 0 {
                [150, 40, 40, 255]
            } else if d2 <= 2 {
                [95, 60, 50, 255]
            } else {
                T
            };
        }
    }
    (px.into_iter().flatten().collect(), N)
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
        enemy: image(images, enemy_sprite()),
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

#[allow(clippy::too_many_arguments)]
pub fn spawn_enemy(
    commands: &mut Commands,
    sheets: Option<&SpriteSheets>,
    pos: Vec2,
    hp: f32,
    speed: f32,
    damage: f32,
    alert: Option<Vec2>,
    wander: Vec2,
) -> Entity {
    let mut e = commands.spawn((
        Enemy {
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
        Collider::circle(ENEMY_RADIUS),
        LockedAxes::ROTATION_LOCKED,
        LinearDamping(8.0),
        CollisionLayers::new(Layer::Enemy, [Layer::World, Layer::Unit, Layer::Enemy]),
        Transform::from_translation(pos.extend(4.0)),
        Visibility::Hidden,
    ));
    if let Some(sheets) = sheets {
        e.insert(Sprite::from_image(sheets.enemy.clone()));
    }
    e.id()
}
