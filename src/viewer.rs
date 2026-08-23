//! Bevy front-end: background generation, pan/zoom, and an owner-based
//! alt-history editor — click regions to select/assign, an owner palette
//! panel, multi-select, zoom-to-selection, drill-down into sub-levels,
//! undo/redo and scenario save.

use crate::config::MapConfig;
use crate::generate::{self, Generated};
use crate::osm::Feature;
use crate::palette::{self, Palette, Rgba};
use crate::raster::{self, Overlay, Rendered};
use crate::scenario::Scenario;
use crate::ui_font::{UiFont, UiFontPlugin};
use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};

pub struct ViewerPlugin;

impl Plugin for ViewerPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(UiFontPlugin)
            .init_resource::<MapJob>()
            .init_resource::<Editor>()
            .init_resource::<MapStack>()
            .add_systems(Startup, (setup, start_job))
            .add_systems(
                Update,
                (
                    poll_job,
                    owner_buttons,
                    typing_input,
                    pan_zoom,
                    select_region,
                    hotkeys,
                    rebuild_panel,
                    update_status,
                )
                    .chain(),
            );
    }
}

const PANEL_W: f32 = 190.0;
const HELP: &str = "click: select/assign · shift: multi · 1-9/[ ]: colour · N: owner · L: legend · Z: zoom sel · D: drill down · Bksp: back · Ctrl+S/Z: save/undo · E: export · R: refetch";

// ---------------------------------------------------------------------------
// Resources

enum JobMessage {
    Progress(String),
    Done(Box<anyhow::Result<Generated>>),
}

#[derive(Resource, Default)]
struct MapJob {
    rx: Option<Mutex<Receiver<JobMessage>>>,
    /// Result becomes a new stack entry (drill-down) instead of replacing.
    push: bool,
    status: String,
    notice: String,
}

/// One map in the drill-down stack.
struct MapState {
    cfg: MapConfig,
    rendered: Rendered,
    features: Vec<Feature>,
    palette: Palette,
    mpp: f64,
    image: Handle<Image>,
    camera: Transform,
}

#[derive(Resource, Default)]
struct MapStack(Vec<MapState>);

#[derive(Resource, Default)]
struct Editor {
    scenario: Scenario,
    active_owner: Option<String>,
    selection: Vec<usize>,
    undo: Vec<Scenario>,
    redo: Vec<Scenario>,
    /// New-owner name being typed (`N` starts, Enter confirms, Esc cancels).
    typing: Option<String>,
    panel_visible: bool,
    panel_dirty: bool,
}

impl Editor {
    fn checkpoint(&mut self) {
        self.undo.push(self.scenario.clone());
        if self.undo.len() > 64 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
}

#[derive(Component)]
struct MapSprite;

#[derive(Component)]
struct StatusText;

#[derive(Component)]
struct NoticeText;

#[derive(Component)]
struct OwnerPanel;

#[derive(Component)]
struct OwnerButton(String);

// ---------------------------------------------------------------------------
// Setup / job handling

fn setup(
    mut commands: Commands,
    cfg: Res<MapConfig>,
    mut editor: ResMut<Editor>,
    font: Res<UiFont>,
) {
    commands.spawn((Camera2d, Msaa::Off));
    let text = |bottom: f32| Node {
        position_type: PositionType::Absolute,
        left: Val::Px(8.0),
        bottom: Val::Px(bottom),
        max_width: Val::Percent(90.0),
        ..default()
    };
    commands.spawn((
        Text::new(""),
        font.text_font(13.0),
        TextColor(Color::WHITE),
        text(8.0),
        StatusText,
    ));
    commands.spawn((
        Text::new(""),
        font.text_font(13.0),
        TextColor(Color::srgb(1.0, 0.9, 0.5)),
        text(26.0),
        NoticeText,
    ));
    editor.panel_visible = true;
    editor.panel_dirty = true;
    match generate::load_style(&cfg) {
        Ok((_, scenario)) => editor.scenario = scenario,
        Err(e) => error!("loading scenario: {e:#}"),
    }
}

fn start_job(cfg: Res<MapConfig>, mut job: ResMut<MapJob>) {
    spawn_job(cfg.clone(), &mut job, false);
}

fn spawn_job(cfg: MapConfig, job: &mut MapJob, push: bool) {
    let (tx, rx) = channel();
    job.status = format!("Generating {} …", cfg.bbox);
    job.rx = Some(Mutex::new(rx));
    job.push = push;
    let task = move || {
        let progress_tx = tx.clone();
        let progress = move |m: String| {
            let _ = progress_tx.send(JobMessage::Progress(m));
        };
        let result = generate::generate_with_progress(&cfg, &progress);
        let _ = tx.send(JobMessage::Done(Box::new(result)));
    };
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::spawn(task);
    #[cfg(target_arch = "wasm32")]
    task();
}

fn make_image(canvas: &raster::Canvas) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: canvas.width,
            height: canvas.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        canvas.to_rgba_bytes(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    image
}

#[allow(clippy::too_many_arguments)]
fn poll_job(
    mut commands: Commands,
    mut job: ResMut<MapJob>,
    mut stack: ResMut<MapStack>,
    mut editor: ResMut<Editor>,
    cfg: Res<MapConfig>,
    mut images: ResMut<Assets<Image>>,
    existing: Query<Entity, With<MapSprite>>,
    mut cam: Single<&mut Transform, With<Camera2d>>,
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
    let push = job.push;
    job.rx = None;
    job.push = false;
    match result {
        Err(e) => {
            job.status = format!("error: {e:#}");
            error!("{e:#}");
        }
        Ok(g) => {
            // remember camera on the parent before drilling down
            if push {
                if let Some(top) = stack.0.last_mut() {
                    top.camera = **cam;
                }
            } else {
                stack.0.clear();
            }
            // scenario continues across the stack; merge what the job loaded
            let mut scenario = std::mem::take(&mut editor.scenario);
            scenario.merge(g.scenario.clone());
            editor.scenario = scenario;
            let state = MapState {
                cfg: cfg_for(&cfg, &g),
                rendered: g.rendered,
                features: g.features,
                palette: g.palette,
                mpp: g.metres_per_pixel,
                image: images.add(make_image(&g.composed)),
                camera: Transform::IDENTITY,
            };
            for e in &existing {
                commands.entity(e).despawn();
            }
            commands.spawn((
                Sprite::from_image(state.image.clone()),
                Transform::from_scale(Vec3::splat(cfg.scale.max(1) as f32)),
                MapSprite,
            ));
            **cam = Transform::IDENTITY;
            let msg = if cfg!(target_arch = "wasm32") {
                let c = &g.composed;
                format!(
                    "{}×{} px, {} features, {} regions – web demo (bundled data)",
                    c.width,
                    c.height,
                    state.features.len(),
                    state.rendered.regions.len()
                )
            } else {
                let c = &g.composed;
                match generate::export_canvas(
                    c,
                    &state.cfg.output,
                    cfg.scale,
                    cfg.grid,
                    &state.palette,
                ) {
                    Ok(paths) => format!(
                        "{}×{} px ({:.0} m/px), {} features, {} regions{} – exported {}",
                        c.width,
                        c.height,
                        state.mpp,
                        state.features.len(),
                        state.rendered.regions.len(),
                        state
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
                }
            };
            stack.0.push(state);
            editor.selection.clear();
            editor.panel_dirty = true;
            job.status = msg;
            job.notice = HELP.to_string();
            // colours may differ once the merged scenario applies
            let map = stack.0.last_mut().unwrap();
            refresh_map(map, &editor, &mut images);
        }
    }
}

/// Per-map config: the drill-down job carried its own bbox/admin level in the
/// job cfg, which `generate` echoes back through `Generated` – reconstruct.
fn cfg_for(base: &MapConfig, g: &Generated) -> MapConfig {
    let mut c = base.clone();
    c.bbox = g.bbox_string.clone();
    c.admin_level = g.admin_level_requested;
    c
}

/// Re-resolve region colours from the scenario, recompute overlays and update
/// the GPU texture. `Editor.selection` is drawn as an inverted outline.
fn refresh_map(map: &mut MapState, editor: &Editor, images: &mut Assets<Image>) {
    for idx in 0..map.rendered.regions.len() {
        let info = &map.rendered.regions[idx];
        let colour = editor.scenario.colour_for(info.id, info.name.as_deref());
        if colour != info.colour {
            raster::recolour_region(&mut map.rendered, idx, colour);
        }
    }
    let mut composed = generate::compose(
        &map.rendered,
        &map.features,
        &editor.scenario,
        &map.cfg,
        &map.palette,
        map.mpp,
    );
    if !editor.selection.is_empty() {
        let outline = selection_outline(&map.rendered, &editor.selection);
        for (i, c) in outline.iter().enumerate() {
            if let Some(c) = c {
                composed.pixels[i] = *c;
            }
        }
    }
    if let Some(mut img) = images.get_mut(&map.image) {
        img.data = Some(composed.to_rgba_bytes());
    }
}

/// 1-px inverted outline around the selected regions.
fn selection_outline(r: &Rendered, selection: &[usize]) -> Overlay {
    let (w, h) = (r.canvas.width as i32, r.canvas.height as i32);
    let mut ov: Overlay = vec![None; r.region_ids.len()];
    let selected = |i: usize| {
        let id = r.region_ids[i];
        id != u32::MAX && selection.contains(&(id as usize))
    };
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if !selected(i) {
                continue;
            }
            let edge = x == 0
                || y == 0
                || x == w - 1
                || y == h - 1
                || [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]
                    .into_iter()
                    .any(|(nx, ny)| !selected((ny * w + nx) as usize));
            if edge {
                let p = r.canvas.pixels[i];
                ov[i] = Some([255 - p[0], 255 - p[1], 255 - p[2], 255]);
            }
        }
    }
    ov
}

// ---------------------------------------------------------------------------
// Input

fn pan_zoom(
    mut cam: Single<&mut Transform, With<Camera2d>>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
) {
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    if buttons.pressed(MouseButton::Left) && !shift && motion.delta != Vec2::ZERO {
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

#[derive(Default)]
struct DragState {
    travelled: f32,
    /// Region already painted during this shift-drag.
    painted: Option<usize>,
}

/// Map pixel under the cursor, if any.
fn cursor_pixel(
    window: &Window,
    camera: &Camera,
    cam_tf: &GlobalTransform,
    cfg: &MapConfig,
    r: &Rendered,
) -> Option<usize> {
    let cursor = window.cursor_position()?;
    if cursor.x > window.width() - PANEL_W {
        return None; // over the owner panel
    }
    let world = camera.viewport_to_world_2d(cam_tf, cursor).ok()?;
    let s = cfg.scale.max(1) as f32;
    let px = (world.x / s + r.canvas.width as f32 / 2.0).floor() as i32;
    let py = (r.canvas.height as f32 / 2.0 - world.y / s).floor() as i32;
    r.canvas.idx(px, py)
}

#[allow(clippy::too_many_arguments)]
fn select_region(
    mut drag: Local<DragState>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    cfg: Res<MapConfig>,
    mut job: ResMut<MapJob>,
    mut editor: ResMut<Editor>,
    mut stack: ResMut<MapStack>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(map) = stack.0.last_mut() else {
        return;
    };
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let (camera, cam_tf) = *camera;

    if buttons.just_pressed(MouseButton::Left) {
        drag.travelled = 0.0;
        drag.painted = None;
    }
    if buttons.pressed(MouseButton::Left) {
        drag.travelled += motion.delta.length();
    }

    // shift+drag: continuous select/assign
    if shift && buttons.pressed(MouseButton::Left) {
        if let Some(i) = cursor_pixel(&window, camera, cam_tf, &cfg, &map.rendered) {
            let id = map.rendered.region_ids[i];
            if id != u32::MAX && drag.painted != Some(id as usize) {
                drag.painted = Some(id as usize);
                apply_click(id as usize, true, map, &mut editor, &mut job, &mut images);
            }
        }
        return;
    }

    // plain click (not a pan)
    if !buttons.just_released(MouseButton::Left) || drag.travelled > 4.0 {
        return;
    }
    let Some(i) = cursor_pixel(&window, camera, cam_tf, &cfg, &map.rendered) else {
        return;
    };
    let id = map.rendered.region_ids[i];
    if id == u32::MAX {
        editor.selection.clear();
        job.notice = HELP.to_string();
        refresh_map(map, &editor, &mut images);
        return;
    }
    apply_click(id as usize, false, map, &mut editor, &mut job, &mut images);
}

fn apply_click(
    idx: usize,
    additive: bool,
    map: &mut MapState,
    editor: &mut Editor,
    job: &mut MapJob,
    images: &mut Assets<Image>,
) {
    if additive {
        if !editor.selection.contains(&idx) {
            editor.selection.push(idx);
        }
    } else {
        editor.selection = vec![idx];
    }
    let info = map.rendered.regions[idx].clone();
    if let Some(owner) = editor.active_owner.clone() {
        editor.checkpoint();
        editor.scenario.assign_owner(info.id, Some(&owner));
        editor.panel_dirty = true;
        job.notice = format!(
            "{} → {owner} (unsaved: Ctrl+S) · {} selected",
            info.name.as_deref().unwrap_or("region"),
            editor.selection.len()
        );
    } else {
        let owner = editor
            .scenario
            .owner_of(info.id, info.name.as_deref())
            .unwrap_or("—")
            .to_string();
        let px: usize = editor
            .selection
            .iter()
            .map(|i| map.rendered.regions[*i].pixels)
            .sum();
        job.notice = format!(
            "selected {} region(s), {px} px · {} (relation {}, L{}, owner {owner}, {})",
            editor.selection.len(),
            info.name.as_deref().unwrap_or("unnamed"),
            info.id,
            info.admin_level,
            palette::to_hex(info.colour),
        );
    }
    refresh_map(map, editor, images);
}

/// Text input while naming a new owner.
fn typing_input(
    mut evr: MessageReader<KeyboardInput>,
    mut editor: ResMut<Editor>,
    mut job: ResMut<MapJob>,
    mut stack: ResMut<MapStack>,
    mut images: ResMut<Assets<Image>>,
) {
    if editor.typing.is_none() {
        evr.clear();
        return;
    }
    for ev in evr.read() {
        if ev.state != ButtonState::Pressed {
            continue;
        }
        let Some(buf) = editor.typing.as_mut() else {
            break;
        };
        match &ev.logical_key {
            Key::Character(c) if buf.len() < 40 => {
                buf.push_str(c);
            }
            Key::Space => buf.push(' '),
            Key::Backspace => {
                buf.pop();
            }
            Key::Enter => {
                let name = buf.trim().to_string();
                editor.typing = None;
                if name.is_empty() {
                    job.notice = HELP.to_string();
                } else {
                    editor.checkpoint();
                    let n = editor.scenario.owners.len();
                    let colour = Palette::PRESETS[n % Palette::PRESETS.len()];
                    editor.scenario.set_owner_colour(&name, colour);
                    editor.active_owner = Some(name.clone());
                    editor.panel_dirty = true;
                    job.notice =
                        format!("owner `{name}` created and active – click regions to assign");
                    if let Some(map) = stack.0.last_mut() {
                        refresh_map(map, &editor, &mut images);
                    }
                }
                break;
            }
            Key::Escape => {
                editor.typing = None;
                job.notice = HELP.to_string();
                break;
            }
            _ => {}
        }
        if let Some(buf) = &editor.typing {
            job.notice = format!("new owner: {buf}_  (Enter to create, Esc to cancel)");
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn hotkeys(
    keys: Res<ButtonInput<KeyCode>>,
    cfg: Res<MapConfig>,
    mut job: ResMut<MapJob>,
    mut editor: ResMut<Editor>,
    mut stack: ResMut<MapStack>,
    mut images: ResMut<Assets<Image>>,
    mut cam: Single<&mut Transform, With<Camera2d>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut sprite: Query<&mut Sprite, With<MapSprite>>,
) {
    if editor.typing.is_some() {
        return; // typing consumes the keyboard
    }
    let ctrl = keys.pressed(KeyCode::ControlLeft)
        || keys.pressed(KeyCode::ControlRight)
        || keys.pressed(KeyCode::SuperLeft)
        || keys.pressed(KeyCode::SuperRight);
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);

    if keys.just_pressed(KeyCode::Digit0) && editor.selection.is_empty() {
        **cam = Transform::IDENTITY;
    }
    if keys.just_pressed(KeyCode::KeyR) && job.rx.is_none() {
        if let Some(map) = stack.0.last() {
            let mut c = map.cfg.clone();
            c.no_cache = true;
            spawn_job(c, &mut job, false);
        }
        return;
    }
    if keys.just_pressed(KeyCode::KeyN) {
        editor.typing = Some(String::new());
        job.notice = "new owner: _  (Enter to create, Esc to cancel)".into();
        return;
    }
    if keys.just_pressed(KeyCode::KeyL) {
        editor.panel_visible = !editor.panel_visible;
        editor.panel_dirty = true;
    }
    if keys.just_pressed(KeyCode::Escape) {
        editor.selection.clear();
        editor.active_owner = None;
        editor.panel_dirty = true;
        job.notice = HELP.to_string();
        if let Some(map) = stack.0.last_mut() {
            refresh_map(map, &editor, &mut images);
        }
    }
    if keys.just_pressed(KeyCode::Delete) {
        if let Some(owner) = editor.active_owner.clone() {
            let used = stack.0.last().map(|m| {
                m.rendered.regions.iter().any(|r| {
                    editor.scenario.owner_of(r.id, r.name.as_deref()) == Some(owner.as_str())
                })
            });
            if used == Some(true) {
                job.notice = format!("owner `{owner}` still has regions – reassign first");
            } else {
                editor.checkpoint();
                editor.scenario.owners.remove(&owner);
                editor.active_owner = None;
                editor.panel_dirty = true;
                job.notice = format!("owner `{owner}` removed");
            }
        }
        return;
    }
    if keys.just_pressed(KeyCode::Backspace) && stack.0.len() > 1 {
        let popped = stack.0.pop().unwrap();
        images.remove(&popped.image);
        let depth = stack.0.len();
        let parent = stack.0.last_mut().unwrap();
        if let Ok(mut sp) = sprite.single_mut() {
            sp.image = parent.image.clone();
        }
        **cam = parent.camera;
        editor.selection.clear();
        refresh_map(parent, &editor, &mut images);
        job.notice = format!("back to {} (depth {depth})", parent.cfg.bbox);
        return;
    }

    let Some(map) = stack.0.last_mut() else {
        return;
    };

    if keys.just_pressed(KeyCode::KeyE) {
        let composed = generate::compose(
            &map.rendered,
            &map.features,
            &editor.scenario,
            &map.cfg,
            &map.palette,
            map.mpp,
        );
        job.notice = match generate::export_canvas(
            &composed,
            &map.cfg.output,
            cfg.scale,
            cfg.grid,
            &map.palette,
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
    if ctrl && keys.just_pressed(KeyCode::KeyS) {
        let path: PathBuf = cfg
            .scenario
            .last()
            .cloned()
            .unwrap_or_else(|| PathBuf::from("scenarios/edited.toml"));
        job.notice = match editor.scenario.save(&path) {
            Ok(()) => format!("saved scenario to {}", path.display()),
            Err(e) => format!("save failed: {e:#}"),
        };
        return;
    }
    if ctrl && keys.just_pressed(KeyCode::KeyZ) {
        let restored = if shift {
            editor.redo.pop()
        } else {
            editor.undo.pop()
        };
        if let Some(s) = restored {
            let current = std::mem::replace(&mut editor.scenario, s);
            if shift {
                editor.undo.push(current);
            } else {
                editor.redo.push(current);
            }
            editor.panel_dirty = true;
            refresh_map(map, &editor, &mut images);
            job.notice = if shift { "redo" } else { "undo" }.to_string();
        } else {
            job.notice = "nothing to undo/redo".into();
        }
        return;
    }

    // Z: zoom to selection, D: drill down
    if keys.just_pressed(KeyCode::KeyZ)
        && !ctrl
        && !editor.selection.is_empty()
        && let Some((min, max)) = selection_pixel_bbox(&map.rendered, &editor.selection)
    {
        let s = cfg.scale.max(1) as f32;
        let (w, h) = (
            map.rendered.canvas.width as f32,
            map.rendered.canvas.height as f32,
        );
        let cx = ((min.0 + max.0) as f32 / 2.0 - w / 2.0) * s;
        let cy = (h / 2.0 - (min.1 + max.1) as f32 / 2.0) * s;
        let ext_x = ((max.0 - min.0 + 1) as f32) * s;
        let ext_y = ((max.1 - min.1 + 1) as f32) * s;
        let zoom = (ext_x / window.width().max(1.0)).max(ext_y / window.height().max(1.0)) * 1.15;
        cam.translation = Vec3::new(cx, cy, cam.translation.z);
        cam.scale = Vec3::new(zoom.clamp(0.05, 20.0), zoom.clamp(0.05, 20.0), 1.0);
    }
    if keys.just_pressed(KeyCode::KeyD) && job.rx.is_none() && !editor.selection.is_empty() {
        if let Some((min, max)) = selection_pixel_bbox(&map.rendered, &editor.selection) {
            let pad = 2.0;
            let a = map
                .rendered
                .proj
                .unproject([min.0 as f64 - pad, max.1 as f64 + pad]);
            let b = map
                .rendered
                .proj
                .unproject([max.0 as f64 + pad, min.1 as f64 - pad]);
            let mut c = map.cfg.clone();
            c.bbox = format!("{:.5},{:.5},{:.5},{:.5}", a[1], a[0], b[1], b[0]);
            c.admin_level = match map.rendered.admin_level_used.unwrap_or(c.admin_level) {
                l @ 0..=3 => (l + 2).max(4),
                l => (l + 2).min(8),
            };
            job.notice = format!(
                "drilling into {} at admin_level {} …",
                c.bbox, c.admin_level
            );
            spawn_job(c, &mut job, true);
        }
        return;
    }

    // recolour: active owner if set, else selected regions
    let mut new_colour: Option<Rgba> = None;
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
    for (i, k) in DIGITS.iter().enumerate() {
        if keys.just_pressed(*k) {
            new_colour = Some(Palette::PRESETS[i]);
        }
    }
    let current_colour = {
        let editor = &editor;
        let map = &*map;
        move || -> Rgba {
            if let Some(o) = &editor.active_owner {
                editor
                    .scenario
                    .owner_colour(o)
                    .unwrap_or(Palette::PRESETS[0])
            } else if let Some(idx) = editor.selection.first() {
                map.rendered.regions[*idx].colour
            } else {
                Palette::PRESETS[0]
            }
        }
    };
    if keys.just_pressed(KeyCode::BracketLeft) {
        new_colour = Some(palette::rotate_hue(current_colour(), -30.0));
    }
    if keys.just_pressed(KeyCode::BracketRight) {
        new_colour = Some(palette::rotate_hue(current_colour(), 30.0));
    }
    if let Some(c) = new_colour {
        if editor.active_owner.is_none() && editor.selection.is_empty() {
            return;
        }
        editor.checkpoint();
        if let Some(owner) = editor.active_owner.clone() {
            editor.scenario.set_owner_colour(&owner, c);
            job.notice = format!("{owner} → {} (unsaved: Ctrl+S)", palette::to_hex(c));
        } else {
            for idx in editor.selection.clone() {
                let id = map.rendered.regions[idx].id;
                editor.scenario.set_colour(id, c);
            }
            job.notice = format!(
                "{} region(s) → {} (unsaved: Ctrl+S)",
                editor.selection.len(),
                palette::to_hex(c)
            );
        }
        editor.panel_dirty = true;
        refresh_map(map, &editor, &mut images);
    }
}

fn selection_pixel_bbox(r: &Rendered, selection: &[usize]) -> Option<((i32, i32), (i32, i32))> {
    let w = r.canvas.width as i32;
    let mut min = (i32::MAX, i32::MAX);
    let mut max = (i32::MIN, i32::MIN);
    for (i, id) in r.region_ids.iter().enumerate() {
        if *id != u32::MAX && selection.contains(&(*id as usize)) {
            let (x, y) = (i as i32 % w, i as i32 / w);
            min = (min.0.min(x), min.1.min(y));
            max = (max.0.max(x), max.1.max(y));
        }
    }
    (min.0 != i32::MAX).then_some((min, max))
}

// ---------------------------------------------------------------------------
// Owner panel UI

fn owner_buttons(
    interactions: Query<(&Interaction, &OwnerButton), Changed<Interaction>>,
    mut editor: ResMut<Editor>,
    mut job: ResMut<MapJob>,
) {
    for (interaction, button) in &interactions {
        if *interaction == Interaction::Pressed {
            if button.0 == "\u{0}new" {
                editor.typing = Some(String::new());
                job.notice = "new owner: _  (Enter to create, Esc to cancel)".into();
            } else if editor.active_owner.as_deref() == Some(button.0.as_str()) {
                editor.active_owner = None;
                job.notice = "owner brush cleared".into();
            } else {
                editor.active_owner = Some(button.0.clone());
                job.notice = format!("owner `{}` active – click regions to assign", button.0);
            }
            editor.panel_dirty = true;
        }
    }
}

fn rebuild_panel(
    mut commands: Commands,
    mut editor: ResMut<Editor>,
    stack: Res<MapStack>,
    panels: Query<Entity, With<OwnerPanel>>,
    font: Res<UiFont>,
) {
    if !editor.panel_dirty {
        return;
    }
    editor.panel_dirty = false;
    for e in &panels {
        commands.entity(e).despawn();
    }
    if !editor.panel_visible {
        return;
    }
    let counts = |owner: &str| -> usize {
        stack
            .0
            .last()
            .map(|m| {
                m.rendered
                    .regions
                    .iter()
                    .filter(|r| editor.scenario.owner_of(r.id, r.name.as_deref()) == Some(owner))
                    .count()
            })
            .unwrap_or(0)
    };
    let mut root = commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(0.0),
            top: Val::Px(0.0),
            width: Val::Px(PANEL_W),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(Val::Px(6.0)),
            row_gap: Val::Px(4.0),
            ..default()
        },
        BackgroundColor(Color::srgba(0.1, 0.1, 0.12, 0.85)),
        OwnerPanel,
    ));
    root.with_children(|ui| {
        ui.spawn((
            Text::new("Owners  (L to hide)"),
            font.text_font(13.0),
            TextColor(Color::srgb(0.8, 0.8, 0.8)),
        ));
        for (owner, colour_hex) in editor.scenario.owners.clone() {
            let active = editor.active_owner.as_deref() == Some(owner.as_str());
            let swatch = palette::parse_hex(&colour_hex).unwrap_or([200, 200, 200, 255]);
            ui.spawn((
                Button,
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(6.0),
                    padding: UiRect::all(Val::Px(3.0)),
                    ..default()
                },
                BackgroundColor(if active {
                    Color::srgba(0.35, 0.35, 0.45, 1.0)
                } else {
                    Color::srgba(0.2, 0.2, 0.22, 1.0)
                }),
                OwnerButton(owner.clone()),
            ))
            .with_children(|b| {
                b.spawn((
                    Node {
                        width: Val::Px(14.0),
                        height: Val::Px(14.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgb_u8(swatch[0], swatch[1], swatch[2])),
                ));
                b.spawn((
                    Text::new(format!("{owner} ({})", counts(&owner))),
                    font.text_font(12.0),
                    TextColor(Color::WHITE),
                ));
            });
        }
        ui.spawn((
            Button,
            Node {
                padding: UiRect::all(Val::Px(3.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.2, 0.3, 0.2, 1.0)),
            OwnerButton("\u{0}new".to_string()),
        ))
        .with_children(|b| {
            b.spawn((
                Text::new("+ new owner (N)"),
                font.text_font(12.0),
                TextColor(Color::WHITE),
            ));
        });
    });
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
