//! Barricades: board up a carved door or window. Blocks movement (and sight,
//! for windows); enemies that press against one smash it down.

use super::buildings::OpeningKind;
use super::logic::{Score, SimRng};
use super::population::Noise;
use super::tuning::*;
use super::units::{Dormant, Enemy, Soldier};
use super::world::{GameWorld, StaticWorld};
use avian2d::prelude::*;
use bevy::prelude::*;

/// A built barricade over `opening`.
#[derive(Component)]
pub struct Barricade {
    pub opening: usize,
    pub hp: f32,
}

/// A soldier's order to build (or tear down) the given opening's barricade.
#[derive(Component)]
pub struct BuildTask {
    pub opening: usize,
    pub tear: bool,
    pub t: f32,
    pub hammer_t: f32,
}

/// Which openings are currently barricaded.
#[derive(Resource, Default)]
pub struct Barricades(pub Vec<Option<Entity>>);

/// The map texture needs repainting at these pixel indices.
#[derive(Message)]
pub struct RepaintFx {
    pub opening: usize,
    pub built: bool,
}

pub struct BarricadePlugin;

impl Plugin for BarricadePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Barricades>()
            .add_message::<RepaintFx>()
            .add_systems(
                Update,
                (work_tasks, enemies_smash).run_if(resource_exists::<GameWorld>),
            );
    }
}

/// Toggle the world masks + nav for an opening (build = true → blocked).
pub fn set_masks(world: &mut GameWorld, opening: usize, built: bool) {
    let Some(op) = world.openings.get(opening) else {
        return;
    };
    let pixels = op.pixels.clone();
    for &i in &pixels {
        world.blocked[i] = built;
        // windows: boarding blocks sight both ways; doors block movement only
        if op.kind == OpeningKind::Window {
            world.sight_blocked[i] = built;
        }
    }
    // refresh the affected nav cells (blocked + tight in a small neighbourhood)
    let w = world.w;
    let mut cells: Vec<(i32, i32)> = pixels
        .iter()
        .map(|&i| {
            let (x, y) = ((i as u32 % w) as i32, (i as u32 / w) as i32);
            (x / super::nav::CELL as i32, y / super::nav::CELL as i32)
        })
        .collect();
    cells.sort_unstable();
    cells.dedup();
    let cell = super::nav::CELL as i32;
    for (cx, cy) in cells {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (nx, ny) = (cx + dx, cy + dy);
                let Some(ci) = world.nav.idx(nx, ny) else {
                    continue;
                };
                // recompute blocked from pixels
                let mut b = false;
                for py in (ny * cell)..(ny * cell + cell) {
                    for px in (nx * cell)..(nx * cell + cell) {
                        if px >= 0 && py >= 0 && px < world.w as i32 && py < world.h as i32 {
                            b |= world.blocked[(py as u32 * world.w + px as u32) as usize];
                        }
                    }
                }
                world.nav.blocked[ci] = b;
            }
        }
        // tight: recompute for the centre cell + ring from the new blocked
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (nx, ny) = (cx + dx, cy + dy);
                let Some(ci) = world.nav.idx(nx, ny) else {
                    continue;
                };
                if world.nav.blocked[ci] {
                    world.nav.tight[ci] = true;
                    continue;
                }
                let mut t = false;
                for ty in -1..=1i32 {
                    for tx in -1..=1i32 {
                        t |= world.nav.is_blocked(nx + tx, ny + ty);
                    }
                }
                world.nav.tight[ci] = t;
            }
        }
    }
    // flow field refreshes on its own timer and will route accordingly
}

/// Order the nearest selected soldier to build/tear at `opening`.
pub fn order(commands: &mut Commands, soldier: Entity, opening: usize, tear: bool) {
    commands.entity(soldier).insert(BuildTask {
        opening,
        tear,
        t: 0.0,
        hammer_t: 0.0,
    });
}

#[allow(clippy::too_many_arguments)]
fn work_tasks(
    time: Res<Time>,
    mut commands: Commands,
    mut world: ResMut<GameWorld>,
    mut barricades: ResMut<Barricades>,
    mut stock: ResMut<super::economy::Stockpile>,
    mut score: ResMut<Score>,
    mut noise: MessageWriter<Noise>,
    mut repaint: MessageWriter<RepaintFx>,
    mut workers: Query<
        (
            Entity,
            &Transform,
            &mut BuildTask,
            &mut super::units::Orders,
        ),
        With<Soldier>,
    >,
) {
    let dt = time.delta_secs();
    if barricades.0.len() < world.openings.len() {
        barricades.0.resize(world.openings.len(), None);
    }
    for (ent, tf, mut task, mut orders) in &mut workers {
        let Some(op) = world.openings.get(task.opening) else {
            commands.entity(ent).remove::<BuildTask>();
            continue;
        };
        let target = world.to_world(op.centre.0, op.centre.1);
        let pos = tf.translation.truncate();
        if pos.distance(target) > 7.0 {
            // walk into reach first
            if orders.waypoints.is_empty() {
                let from = world.to_map(pos);
                let path = world.nav.path(from, op.centre);
                if let Some(p) = path {
                    orders.waypoints = p.iter().map(|q| world.to_world(q.0, q.1)).collect();
                } else {
                    commands.entity(ent).remove::<BuildTask>(); // unreachable
                }
            }
            continue;
        }
        orders.waypoints.clear();
        // channel
        task.t += dt;
        task.hammer_t += dt;
        if task.hammer_t >= 1.0 {
            task.hammer_t = 0.0;
            noise.write(Noise {
                pos,
                radius: NOISE_HAMMER,
            });
        }
        let need = if task.tear {
            BARRICADE_TEAR_S
        } else {
            BARRICADE_BUILD_S
        };
        if task.t < need {
            continue;
        }
        let idx = task.opening;
        if task.tear {
            if let Some(b) = barricades.0[idx].take() {
                commands.entity(b).despawn();
                set_masks(&mut world, idx, false);
                stock.scrap += BARRICADE_REFUND;
                repaint.write(RepaintFx {
                    opening: idx,
                    built: false,
                });
            }
        } else if barricades.0[idx].is_none() && stock.scrap >= BARRICADE_SCRAP {
            stock.scrap -= BARRICADE_SCRAP;
            set_masks(&mut world, idx, true);
            let b = commands
                .spawn((
                    Barricade {
                        opening: idx,
                        hp: BARRICADE_HP,
                    },
                    RigidBody::Static,
                    Collider::rectangle(4.0, 4.0),
                    CollisionLayers::new(
                        super::world::Layer::World,
                        [super::world::Layer::Unit, super::world::Layer::Enemy],
                    ),
                    Transform::from_translation(target.extend(2.0)),
                    StaticWorld,
                ))
                .id();
            barricades.0[idx] = Some(b);
            score.barricades_built += 1;
            repaint.write(RepaintFx {
                opening: idx,
                built: true,
            });
        }
        commands.entity(ent).remove::<BuildTask>();
    }
}

#[allow(clippy::too_many_arguments)]
fn enemies_smash(
    time: Res<Time>,
    mut commands: Commands,
    mut world: ResMut<GameWorld>,
    mut barricades: ResMut<Barricades>,
    mut rng: ResMut<SimRng>,
    mut noise: MessageWriter<Noise>,
    mut repaint: MessageWriter<RepaintFx>,
    mut wall: Query<(Entity, &mut Barricade, &Transform)>,
    mut enemies: Query<(&mut Enemy, &Transform), Without<Dormant>>,
) {
    for (bent, mut b, btf) in &mut wall {
        let bpos = btf.translation.truncate();
        for (mut e, etf) in &mut enemies {
            if e.alert.is_none() {
                continue;
            }
            if etf.translation.truncate().distance(bpos) < 5.5 && e.cooldown.is_finished() {
                e.cooldown.reset();
                b.hp -= BARRICADE_ENEMY_DMG;
                if rng.f32() < 0.4 {
                    noise.write(Noise {
                        pos: bpos,
                        radius: 14.0,
                    });
                }
            }
        }
        let _ = time.delta_secs();
        if b.hp <= 0.0 {
            let idx = b.opening;
            commands.entity(bent).despawn();
            barricades.0[idx] = None;
            set_masks(&mut world, idx, false);
            noise.write(Noise {
                pos: bpos,
                radius: 30.0,
            }); // splintering crash
            repaint.write(RepaintFx {
                opening: idx,
                built: false,
            });
        }
    }
}
