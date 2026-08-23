//! Bevy front-end: runs the generator on a background thread, shows the
//! result as a nearest-neighbour sprite, and offers pan/zoom/export plus a
//! small political-region editor (click to select, keys to recolour,
//! Ctrl+S to write the scenario file, Ctrl+Z to undo).

use crate::config::MapConfig;
use crate::generate::{self, Generated};
use crate::palette::{self, Palette, Rgba};
use crate::raster::{self, Rendered};
use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};
use std::thread;

pub struct ViewerPlugin;

impl Plugin for ViewerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MapJob>()
            .add_systems(Startup, (setup, start_job))
            .add_systems(
                Update,
                (poll_job, pan_zoom, select_region, hotkeys, update_status).chain(),
            );
    }
}

enum JobMessage {
    Progress(String),
    Done(Box<anyhow::Result<Generated>>),
}

#[derive(Resource, Default)]
struct MapJob {
    rx: Option<Mutex<Receiver<JobMessage>>>,
    status: String,
    /// Transient message shown on the second status line.
    notice: String,
}

/// The currently displayed map plus editor state.
#[derive(Resource)]
struct CurrentMap {
    rendered: Rendered,
    palette: Palette,
    scenario: crate::scenario::Scenario,
    image: Handle<Image>,
    selected: Option<usize>,
    undo: Vec<(usize, Rgba)>,
}

#[derive(Component)]
struct MapSprite;

#[derive(Component)]
struct StatusText;

#[derive(Component)]
struct NoticeText;

const HELP: &str = "drag: pan · wheel: zoom · click: select region · 1-9 / [ ]: recolour · Ctrl+S: save scenario · Ctrl+Z: undo · E: export · R: refetch · 0: reset view";

fn setup(mut commands: Commands) {
    commands.spawn((Camera2d, Msaa::Off));
    let text = |bottom: f32| Node {
        position_type: PositionType::Absolute,
        left: Val::Px(8.0),
        bottom: Val::Px(bottom),
        ..default()
    };
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(13.0),
            ..default()
        },
        TextColor(Color::WHITE),
        text(8.0),
        StatusText,
    ));
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(13.0),
            ..default()
        },
        TextColor(Color::srgb(1.0, 0.9, 0.5)),
        text(26.0),
        NoticeText,
    ));
}

fn start_job(cfg: Res<MapConfig>, mut job: ResMut<MapJob>) {
    spawn_job(&cfg, &mut job);
}

fn spawn_job(cfg: &MapConfig, job: &mut MapJob) {
    let (tx, rx) = channel();
    job.status = format!("Generating {} …", cfg.bbox);
    job.rx = Some(Mutex::new(rx));
    let cfg = cfg.clone();
    thread::spawn(move || {
        let progress_tx = tx.clone();
        let progress = move |m: String| {
            let _ = progress_tx.send(JobMessage::Progress(m));
        };
        let result = generate::generate_with_progress(&cfg, &progress);
        let _ = tx.send(JobMessage::Done(Box::new(result)));
    });
}

fn make_image(rendered: &Rendered) -> Image {
    let c = &rendered.canvas;
    let mut image = Image::new(
        Extent3d {
            width: c.width,
            height: c.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        c.to_rgba_bytes(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    image
}

fn poll_job(
    mut commands: Commands,
    mut job: ResMut<MapJob>,
    cfg: Res<MapConfig>,
    mut images: ResMut<Assets<Image>>,
    existing: Query<Entity, With<MapSprite>>,
) {
    let mut done = None;
    let mut progress = Vec::new();
    let mut disconnected = false;
    if let Some(rx) = &job.rx {
        let rx = rx.lock().unwrap();
        loop {
            match rx.try_recv() {
                Ok(JobMessage::Progress(m)) => progress.push(m),
                Ok(JobMessage::Done(r)) => {
                    done = Some(*r);
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
    } else {
        return;
    }
    if let Some(m) = progress.pop() {
        job.status = m;
    }
    if disconnected {
        job.rx = None;
        job.status = "generator thread died".into();
        return;
    }
    let Some(result) = done else { return };
    job.rx = None;
    match result {
        Err(e) => {
            job.status = format!("error: {e:#}");
            error!("{e:#}");
        }
        Ok(generated) => {
            for e in &existing {
                commands.entity(e).despawn();
            }
            let handle = images.add(make_image(&generated.rendered));
            commands.spawn((
                Sprite::from_image(handle.clone()),
                Transform::from_scale(Vec3::splat(cfg.scale.max(1) as f32)),
                MapSprite,
            ));
            let canvas = generated.canvas();
            let msg = match generate::export(
                canvas,
                &cfg.output,
                cfg.scale,
                cfg.grid,
                &generated.palette,
            ) {
                Ok(paths) => format!(
                    "{}×{} px ({:.0} m/px), {} features, {} regions{} – exported {}",
                    canvas.width,
                    canvas.height,
                    generated.metres_per_pixel,
                    generated.feature_count,
                    generated.rendered.regions.len(),
                    generated
                        .rendered
                        .admin_level_used
                        .map(|l| format!(" (L{l})"))
                        .unwrap_or_default(),
                    paths
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                Err(e) => format!("export failed: {e:#}"),
            };
            job.status = msg;
            job.notice = HELP.to_string();
            commands.insert_resource(CurrentMap {
                rendered: generated.rendered,
                palette: generated.palette,
                scenario: generated.scenario,
                image: handle,
                selected: None,
                undo: Vec::new(),
            });
        }
    }
}

fn pan_zoom(
    mut cam: Single<&mut Transform, With<Camera2d>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
) {
    if buttons.pressed(MouseButton::Left) && motion.delta != Vec2::ZERO {
        let s = cam.scale.x;
        cam.translation.x -= motion.delta.x * s;
        cam.translation.y += motion.delta.y * s;
    }
    if scroll.delta.y != 0.0 {
        let step = match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y * 0.1,
            MouseScrollUnit::Pixel => scroll.delta.y * 0.01,
        };
        let factor = (1.0 - step).clamp(0.5, 2.0);
        let new_scale = (cam.scale.x * factor).clamp(0.05, 20.0);
        cam.scale = Vec3::new(new_scale, new_scale, 1.0);
    }
}

/// Track how far the mouse moved while the button was held, so a drag does
/// not count as a click.
#[derive(Default)]
struct DragState {
    travelled: f32,
}

#[allow(clippy::too_many_arguments)]
fn select_region(
    mut drag: Local<DragState>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    cfg: Res<MapConfig>,
    mut job: ResMut<MapJob>,
    current: Option<ResMut<CurrentMap>>,
) {
    if buttons.just_pressed(MouseButton::Left) {
        drag.travelled = 0.0;
    }
    if buttons.pressed(MouseButton::Left) {
        drag.travelled += motion.delta.length();
    }
    if !buttons.just_released(MouseButton::Left) || drag.travelled > 4.0 {
        return;
    }
    let Some(mut current) = current else { return };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let (camera, cam_tf) = *camera;
    let Ok(world) = camera.viewport_to_world_2d(cam_tf, cursor) else {
        return;
    };
    let s = cfg.scale.max(1) as f32;
    let c = &current.rendered.canvas;
    let px = (world.x / s + c.width as f32 / 2.0).floor() as i32;
    let py = (c.height as f32 / 2.0 - world.y / s).floor() as i32;
    let Some(i) = c.idx(px, py) else {
        current.selected = None;
        job.notice = HELP.to_string();
        return;
    };
    let id = current.rendered.region_ids[i];
    if id == u32::MAX {
        current.selected = None;
        job.notice = format!("({px},{py}) no region here · {HELP}");
        return;
    }
    current.selected = Some(id as usize);
    let r = &current.rendered.regions[id as usize];
    let owner = current
        .scenario
        .assignment(r.id, r.name.as_deref())
        .and_then(|a| a.owner.clone())
        .unwrap_or_else(|| "—".into());
    job.notice = format!(
        "selected: {} (relation {}, admin_level {}, owner {}, {}) – {} px · 1-9/[ ] recolour",
        r.name.as_deref().unwrap_or("unnamed"),
        r.id,
        r.admin_level,
        owner,
        palette::to_hex(r.colour),
        r.pixels
    );
}

const DIGITS: [KeyCode; 9] = [
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

fn hotkeys(
    keys: Res<ButtonInput<KeyCode>>,
    cfg: Res<MapConfig>,
    mut job: ResMut<MapJob>,
    current: Option<ResMut<CurrentMap>>,
    mut images: ResMut<Assets<Image>>,
    mut cam: Single<&mut Transform, With<Camera2d>>,
) {
    if keys.just_pressed(KeyCode::Digit0) {
        **cam = Transform::IDENTITY;
    }
    if keys.just_pressed(KeyCode::KeyR) && job.rx.is_none() {
        let mut cfg = cfg.clone();
        cfg.no_cache = true;
        spawn_job(&cfg, &mut job);
        return;
    }
    let Some(mut current) = current else { return };
    let ctrl = keys.pressed(KeyCode::ControlLeft)
        || keys.pressed(KeyCode::ControlRight)
        || keys.pressed(KeyCode::SuperLeft)
        || keys.pressed(KeyCode::SuperRight);

    if keys.just_pressed(KeyCode::KeyE) {
        job.notice = match generate::export(
            &current.rendered.canvas,
            &cfg.output,
            cfg.scale,
            cfg.grid,
            &current.palette,
        ) {
            Ok(paths) => format!(
                "exported {}",
                paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Err(e) => format!("export failed: {e:#}"),
        };
    }

    // --- editor -----------------------------------------------------------
    if ctrl && keys.just_pressed(KeyCode::KeyS) {
        let path: PathBuf = cfg
            .scenario
            .clone()
            .unwrap_or_else(|| PathBuf::from("scenarios/edited.toml"));
        job.notice = match current.scenario.save(&path) {
            Ok(()) => format!("saved scenario to {}", path.display()),
            Err(e) => format!("save failed: {e:#}"),
        };
        return;
    }
    if ctrl && keys.just_pressed(KeyCode::KeyZ) {
        if let Some((idx, prev)) = current.undo.pop() {
            apply_colour(&mut current, idx, prev, &mut images);
            job.notice = format!("undo → region {} back to {}", idx, palette::to_hex(prev));
        } else {
            job.notice = "nothing to undo".into();
        }
        return;
    }
    let Some(idx) = current.selected else { return };
    let mut new_colour: Option<Rgba> = None;
    for (i, k) in DIGITS.iter().enumerate() {
        if keys.just_pressed(*k) {
            new_colour = Some(Palette::PRESETS[i]);
        }
    }
    if keys.just_pressed(KeyCode::BracketLeft) {
        new_colour = Some(palette::rotate_hue(
            current.rendered.regions[idx].colour,
            -30.0,
        ));
    }
    if keys.just_pressed(KeyCode::BracketRight) {
        new_colour = Some(palette::rotate_hue(
            current.rendered.regions[idx].colour,
            30.0,
        ));
    }
    if let Some(c) = new_colour {
        let prev = current.rendered.regions[idx].colour;
        current.undo.push((idx, prev));
        apply_colour(&mut current, idx, c, &mut images);
        let r = &current.rendered.regions[idx];
        job.notice = format!(
            "{} → {} (unsaved: Ctrl+S)",
            r.name.as_deref().unwrap_or("region"),
            palette::to_hex(c)
        );
    }
}

/// Recolour a region in the canvas, the scenario and the GPU texture.
fn apply_colour(current: &mut CurrentMap, idx: usize, colour: Rgba, images: &mut Assets<Image>) {
    raster::recolour_region(&mut current.rendered, idx, colour);
    let rid = current.rendered.regions[idx].id;
    current.scenario.set_colour(rid, colour);
    if let Some(mut img) = images.get_mut(&current.image) {
        img.data = Some(current.rendered.canvas.to_rgba_bytes());
    }
}

fn update_status(
    job: Res<MapJob>,
    mut status: Single<&mut Text, (With<StatusText>, Without<NoticeText>)>,
    mut notice: Single<&mut Text, (With<NoticeText>, Without<StatusText>)>,
) {
    if job.is_changed() {
        status.0 = job.status.clone();
        notice.0 = job.notice.clone();
    }
}
