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
            .init_resource::<Paused>()
            .add_systems(
                Update,
                (
                    pause_toggle,
                    camera_control,
                    selection,
                    orders,
                    control_groups,
                )
                    .chain()
                    .run_if(in_state(Phase::Playing).and_then(resource_exists::<GameWorld>)),
            );
    }
}

/// ESC pause menu state (true = paused, overlay shown, virtual time stopped).
#[derive(Resource, Default)]
pub struct Paused(pub bool);

fn pause_toggle(
    keys: Res<ButtonInput<KeyCode>>,
    mut paused: ResMut<Paused>,
    mut pending: ResMut<PendingCommand>,
    mut time: ResMut<Time<Virtual>>,
) {
    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    // ESC first cancels a pending command, then toggles the pause menu
    if !paused.0 && *pending != PendingCommand::None {
        *pending = PendingCommand::None;
        return;
    }
    paused.0 = !paused.0;
    if paused.0 {
        time.pause();
    } else {
        time.unpause();
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
    Barricade,
}

#[derive(Resource, Default)]
pub struct BoxSelect {
    pub start: Option<Vec2>, // screen coords
    pub current: Vec2,
}

pub const CAM_SPEED: f32 = 300.0;
pub const EDGE: f32 = 14.0;

#[allow(clippy::too_many_arguments)]
fn camera_control(
    time: Res<Time<Real>>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<bevy::input::mouse::AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    mut cam: Single<&mut Transform, With<Camera2d>>,
    world: Res<GameWorld>,
    paused: Res<Paused>,
) {
    let mut dir = Vec2::ZERO;
    // keyboard pan: arrows always; W/D too (S also means "stop" and A means
    // "attack-move", so those two only pan via arrows)
    if keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::ArrowUp) {
        dir.y += 1.0;
    }
    if keys.pressed(KeyCode::ArrowDown) {
        dir.y -= 1.0;
    }
    if keys.pressed(KeyCode::ArrowLeft) {
        dir.x -= 1.0;
    }
    if keys.pressed(KeyCode::KeyD) || keys.pressed(KeyCode::ArrowRight) {
        dir.x += 1.0;
    }
    // edge pan (off while the pause menu is open)
    if !paused.0
        && let Some(cursor) = window.cursor_position()
    {
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
    // middle-mouse drag: grab-scroll the map (RTS standard)
    if buttons.pressed(MouseButton::Middle) && motion.delta != Vec2::ZERO {
        cam.translation.x -= motion.delta.x * zoom;
        cam.translation.y += motion.delta.y * zoom;
    }
    // wheel: zoom keeping the point under the cursor fixed
    if scroll.delta.y != 0.0 {
        let factor = (1.0 - scroll.delta.y * 0.12).clamp(0.5, 2.0);
        let z = (zoom * factor).clamp(0.1, 3.0);
        let (cam_ref, cam_tf) = *camera;
        let anchor = window
            .cursor_position()
            .and_then(|c| cam_ref.viewport_to_world_2d(cam_tf, c).ok());
        if let Some(a) = anchor {
            let t = cam.translation.truncate();
            let nt = a - (a - t) * (z / zoom);
            cam.translation.x = nt.x;
            cam.translation.y = nt.y;
        }
        cam.scale = Vec3::new(z, z, 1.0);
    }
    let half = Vec2::new(world.w as f32 / 2.0, world.h as f32 / 2.0);
    cam.translation.x = cam.translation.x.clamp(-half.x, half.x);
    cam.translation.y = cam.translation.y.clamp(-half.y, half.y);
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
    classes: Query<(Entity, &Soldier, &Transform)>,
) {
    let (camera, cam_tf) = *camera;
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let ctrl = keys.pressed(KeyCode::ControlLeft)
        || keys.pressed(KeyCode::ControlRight)
        || keys.pressed(KeyCode::SuperLeft)
        || keys.pressed(KeyCode::SuperRight);
    // F2: select the whole army (StarCraft II convention)
    if keys.just_pressed(KeyCode::F2) {
        for e in &selected {
            commands.entity(e).remove::<Selected>();
        }
        for (e, _) in &soldiers {
            commands.entity(e).insert(Selected);
        }
        return;
    }
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
        // click: nearest soldier under the cursor
        if let Some((e, _)) = soldiers
            .iter()
            .map(|(e, tf)| (e, tf.translation.truncate().distance(w)))
            .filter(|(_, d)| *d < 5.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
        {
            if ctrl {
                // ctrl+click: select every soldier of the same class on screen
                if let Ok((_, s, _)) = classes.get(e) {
                    let class = s.class;
                    for (e2, s2, tf2) in &classes {
                        let on_screen = camera.world_to_viewport(cam_tf, tf2.translation).is_ok();
                        if s2.class == class && on_screen {
                            commands.entity(e2).insert(Selected);
                        }
                    }
                }
            } else {
                commands.entity(e).insert(Selected);
            }
        }
    }
}

/// Formation offsets: grid around the target, spacing by squad size.
pub fn formation_offsets(n: usize) -> Vec<Vec2> {
    let cols = (n as f32).sqrt().ceil() as usize;
    let spacing = 4.5;
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
    mut commands: Commands,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    world: Res<GameWorld>,
    paused: Res<Paused>,
    barricades: Res<super::barricade::Barricades>,
    mut pending: ResMut<PendingCommand>,
    enemies: Query<(&Transform, &Visibility), With<super::units::Enemy>>,
    selected_ents: Query<(Entity, &Transform), (With<Soldier>, With<Selected>)>,
    mut selected: Query<(&Transform, &mut Orders), (With<Soldier>, With<Selected>)>,
) {
    if paused.0 {
        return;
    }
    if keys.just_pressed(KeyCode::KeyA) {
        *pending = PendingCommand::AttackMove;
    }
    if keys.just_pressed(KeyCode::KeyP) {
        *pending = PendingCommand::Patrol;
    }
    if keys.just_pressed(KeyCode::KeyB) {
        *pending = PendingCommand::Barricade;
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
    // right-click on a visible enemy = attack-move onto it (RTS context order)
    let clicked_enemy = issue_right
        && enemies.iter().any(|(tf, vis)| {
            *vis == Visibility::Visible && tf.translation.truncate().distance(target) < 5.0
        });
    let mode = if issue_left {
        *pending
    } else if clicked_enemy {
        PendingCommand::AttackMove
    } else {
        PendingCommand::None
    };
    *pending = PendingCommand::None;
    if mode == PendingCommand::Barricade {
        // find the door/window nearest to the click, send the closest soldier
        let (tx, ty) = world.to_map(target);
        let opening = world
            .openings
            .iter()
            .enumerate()
            .map(|(i, o)| (i, (o.centre.0 - tx).abs() + (o.centre.1 - ty).abs()))
            .filter(|(_, d)| *d < 9.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i);
        if let Some(idx) = opening
            && let Some((soldier, _)) = selected_ents
                .iter()
                .map(|(e, tf)| (e, tf.translation.truncate().distance(target)))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(e, _)| (e, ()))
        {
            let tear = barricades.0.get(idx).map(|b| b.is_some()).unwrap_or(false);
            super::barricade::order(&mut commands, soldier, idx, tear);
        }
        return;
    }
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
            PendingCommand::Barricade | PendingCommand::None => {
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

#[allow(clippy::too_many_arguments)]
fn control_groups(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time<Real>>,
    mut last_recall: Local<Option<(usize, f32)>>,
    mut groups: ResMut<ControlGroups>,
    selected: Query<Entity, (With<Soldier>, With<Selected>)>,
    all: Query<Entity, With<Soldier>>,
    transforms: Query<&Transform, With<Soldier>>,
    mut cam: Single<&mut Transform, (With<Camera2d>, Without<Soldier>)>,
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
            // double-tap: centre the camera on the group (SC2 convention)
            let now = time.elapsed_secs();
            if matches!(*last_recall, Some((j, t)) if j == i && now - t < 0.4) {
                let members: Vec<Vec2> = groups.0[i]
                    .iter()
                    .filter_map(|e| transforms.get(*e).ok())
                    .map(|tf| tf.translation.truncate())
                    .collect();
                if !members.is_empty() {
                    let c = members.iter().copied().sum::<Vec2>() / members.len() as f32;
                    cam.translation.x = c.x;
                    cam.translation.y = c.y;
                }
            }
            *last_recall = Some((i, now));
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
