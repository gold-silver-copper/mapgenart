//! Player input: camera, selection (click, box, shift), control groups and
//! orders (move, attack-move, stop, hold, patrol).

use super::Phase;
use super::units::{Orders, Selected, Soldier};
use super::world::GameWorld;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use std::collections::VecDeque;

pub struct ControlPlugin;

impl Plugin for ControlPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ControlGroups>()
            .init_resource::<PendingCommand>()
            .init_resource::<BoxSelect>()
            .add_systems(
                Update,
                (camera_control, selection, orders, control_groups)
                    .chain()
                    .run_if(in_state(Phase::Playing).and_then(resource_exists::<GameWorld>)),
            );
    }
}

#[derive(Resource, Default)]
pub struct ControlGroups(pub [Vec<Entity>; 10]);

/// A pending mode from the keyboard (A = attack-move, P = patrol).
#[derive(Resource, Default, PartialEq, Eq, Clone, Copy)]
pub enum PendingCommand {
    #[default]
    None,
    AttackMove,
    Patrol,
}

#[derive(Resource, Default)]
pub struct BoxSelect {
    pub start: Option<Vec2>, // screen coords
    pub current: Vec2,
}

pub const CAM_SPEED: f32 = 300.0;
pub const EDGE: f32 = 14.0;

fn camera_control(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut cam: Single<&mut Transform, With<Camera2d>>,
    world: Res<GameWorld>,
) {
    let mut dir = Vec2::ZERO;
    if keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::ArrowUp) {
        dir.y += 1.0;
    }
    if keys.pressed(KeyCode::KeyS) && !keys.pressed(KeyCode::ControlLeft)
        || keys.pressed(KeyCode::ArrowDown)
    {
        // S alone is "stop"; only pan when held with shift? — use arrows/W A D
    }
    if keys.pressed(KeyCode::ArrowDown) {
        dir.y -= 1.0;
    }
    if keys.pressed(KeyCode::ArrowLeft) {
        dir.x -= 1.0;
    }
    if keys.pressed(KeyCode::ArrowRight) {
        dir.x += 1.0;
    }
    if let Some(cursor) = window.cursor_position() {
        if cursor.x < EDGE {
            dir.x -= 1.0;
        }
        if cursor.x > window.width() - EDGE {
            dir.x += 1.0;
        }
        if cursor.y < EDGE {
            dir.y += 1.0;
        }
        if cursor.y > window.height() - EDGE {
            dir.y -= 1.0;
        }
    }
    let zoom = cam.scale.x;
    cam.translation += (dir.normalize_or_zero() * CAM_SPEED * zoom * time.delta_secs()).extend(0.0);
    let half = Vec2::new(world.w as f32 / 2.0, world.h as f32 / 2.0);
    cam.translation.x = cam.translation.x.clamp(-half.x, half.x);
    cam.translation.y = cam.translation.y.clamp(-half.y, half.y);
    if scroll.delta.y != 0.0 {
        let factor = (1.0 - scroll.delta.y * 0.1).clamp(0.5, 2.0);
        let z = (zoom * factor).clamp(0.15, 3.0);
        cam.scale = Vec3::new(z, z, 1.0);
    }
}

fn cursor_world(window: &Window, camera: &Camera, cam_tf: &GlobalTransform) -> Option<Vec2> {
    let cursor = window.cursor_position()?;
    camera.viewport_to_world_2d(cam_tf, cursor).ok()
}

#[allow(clippy::too_many_arguments)]
fn selection(
    mut commands: Commands,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    mut boxsel: ResMut<BoxSelect>,
    soldiers: Query<(Entity, &Transform), With<Soldier>>,
    selected: Query<Entity, With<Selected>>,
) {
    let (camera, cam_tf) = *camera;
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    if buttons.just_pressed(MouseButton::Left)
        && let Some(c) = window.cursor_position()
    {
        boxsel.start = Some(c);
        boxsel.current = c;
    }
    if buttons.pressed(MouseButton::Left)
        && let Some(c) = window.cursor_position()
    {
        boxsel.current = c;
    }
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    let Some(start) = boxsel.start.take() else {
        return;
    };
    let end = boxsel.current;
    let is_box = start.distance(end) > 6.0;
    if !shift {
        for e in &selected {
            commands.entity(e).remove::<Selected>();
        }
    }
    if is_box {
        let (Ok(a), Ok(b)) = (
            camera.viewport_to_world_2d(cam_tf, start),
            camera.viewport_to_world_2d(cam_tf, end),
        ) else {
            return;
        };
        let (min, max) = (a.min(b), a.max(b));
        for (e, tf) in &soldiers {
            let p = tf.translation.truncate();
            if p.x >= min.x && p.x <= max.x && p.y >= min.y && p.y <= max.y {
                commands.entity(e).insert(Selected);
            }
        }
    } else if let Some(w) = cursor_world(&window, camera, cam_tf) {
        // click: nearest soldier within 8 px
        if let Some((e, _)) = soldiers
            .iter()
            .map(|(e, tf)| (e, tf.translation.truncate().distance(w)))
            .filter(|(_, d)| *d < 8.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
        {
            commands.entity(e).insert(Selected);
        }
    }
}

/// Formation offsets: grid around the target, spacing by squad size.
pub fn formation_offsets(n: usize) -> Vec<Vec2> {
    let cols = (n as f32).sqrt().ceil() as usize;
    let spacing = 8.0;
    (0..n)
        .map(|i| {
            let (r, c) = (i / cols, i % cols);
            Vec2::new(
                (c as f32 - (cols - 1) as f32 / 2.0) * spacing,
                ((r as f32) - ((n.div_ceil(cols) - 1) as f32) / 2.0) * -spacing,
            )
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn orders(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    world: Res<GameWorld>,
    mut pending: ResMut<PendingCommand>,
    mut selected: Query<(&Transform, &mut Orders), (With<Soldier>, With<Selected>)>,
) {
    if keys.just_pressed(KeyCode::KeyA) {
        *pending = PendingCommand::AttackMove;
    }
    if keys.just_pressed(KeyCode::KeyP) {
        *pending = PendingCommand::Patrol;
    }
    if keys.just_pressed(KeyCode::Escape) {
        *pending = PendingCommand::None;
    }
    if keys.just_pressed(KeyCode::KeyS) {
        for (_, mut o) in &mut selected {
            *o = Orders::default();
        }
    }
    if keys.just_pressed(KeyCode::KeyH) {
        for (_, mut o) in &mut selected {
            *o = Orders {
                hold: true,
                ..default()
            };
        }
    }
    let issue_left = *pending != PendingCommand::None && buttons.just_pressed(MouseButton::Left);
    let issue_right = buttons.just_pressed(MouseButton::Right);
    if !issue_left && !issue_right {
        return;
    }
    let (camera, cam_tf) = *camera;
    let Some(target) = cursor_world(&window, camera, cam_tf) else {
        return;
    };
    let mode = if issue_left {
        *pending
    } else {
        PendingCommand::None
    };
    *pending = PendingCommand::None;
    let n = selected.iter().count();
    if n == 0 {
        return;
    }
    let offsets = formation_offsets(n);
    for (i, (tf, mut o)) in selected.iter_mut().enumerate() {
        let goal = target + offsets[i];
        let goal = if world.walkable_world(goal) {
            goal
        } else {
            target
        };
        let from = world.to_map(tf.translation.truncate());
        let to = world.to_map(goal);
        let path = world
            .nav
            .path(from, to)
            .map(|p| {
                p.iter()
                    .map(|q| world.to_world(q.0, q.1))
                    .collect::<VecDeque<_>>()
            })
            .unwrap_or_else(|| VecDeque::from([goal]));
        let hold = false;
        match mode {
            PendingCommand::Patrol => {
                let here = tf.translation.truncate();
                *o = Orders {
                    waypoints: path,
                    attack_move: true,
                    hold,
                    patrol: Some((goal, here)),
                    ..default()
                };
            }
            PendingCommand::AttackMove => {
                *o = Orders {
                    waypoints: path,
                    attack_move: true,
                    hold,
                    patrol: None,
                    ..default()
                };
            }
            PendingCommand::None => {
                *o = Orders {
                    waypoints: path,
                    attack_move: false,
                    hold,
                    patrol: None,
                    ..default()
                };
            }
        }
    }
}

fn control_groups(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut groups: ResMut<ControlGroups>,
    selected: Query<Entity, (With<Soldier>, With<Selected>)>,
    all: Query<Entity, With<Soldier>>,
) {
    const DIGITS: [KeyCode; 10] = [
        KeyCode::Digit0,
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ];
    let ctrl = keys.pressed(KeyCode::ControlLeft)
        || keys.pressed(KeyCode::ControlRight)
        || keys.pressed(KeyCode::SuperLeft)
        || keys.pressed(KeyCode::SuperRight);
    for (i, key) in DIGITS.iter().enumerate() {
        if !keys.just_pressed(*key) {
            continue;
        }
        if ctrl {
            groups.0[i] = selected.iter().collect();
        } else {
            for e in &all {
                commands.entity(e).remove::<Selected>();
            }
            groups.0[i].retain(|e| all.get(*e).is_ok());
            for e in &groups.0[i] {
                commands.entity(*e).insert(Selected);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formation_spreads_units() {
        let offs = formation_offsets(8);
        assert_eq!(offs.len(), 8);
        for i in 0..offs.len() {
            for j in i + 1..offs.len() {
                assert!(offs[i].distance(offs[j]) > 4.0, "{i} vs {j} overlap");
            }
        }
    }
}
