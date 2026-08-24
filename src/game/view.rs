//! Everything visual: map + fog textures, gizmo overlays (selection rings,
//! health bars, tracers, box select), minimap, HUD, menu / game-over screens,
//! F12 screenshots. Kept apart from `logic` so headless runs skip it all.

use super::control::{BoxSelect, Paused};
use super::economy::Stockpile;
use super::logic::{DeathFeed, Score, TracerFx};
use super::objectives::Objectives;
use super::population::NoiseMeter;
use super::units::*;
use super::world::{GameWorld, StaticWorld};
use super::{DayNight, MapLoad, Phase};
use crate::config::MapConfig;
use crate::generate::Generated;
use crate::raster::Canvas;
use crate::ui_font::{UiFont, UiFontPlugin};
use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::PrimaryWindow;

pub struct ViewPlugin;

impl Plugin for ViewPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(UiFontPlugin)
            .init_resource::<Tracers>()
            .add_systems(Startup, spawn_camera)
            .add_systems(OnEnter(Phase::Menu), (cleanup_session, menu_ui))
            .add_systems(Update, menu_input.run_if(in_state(Phase::Menu)))
            .add_systems(OnExit(Phase::Menu), despawn_ui)
            .add_systems(OnEnter(Phase::Loading), loading_ui)
            .add_systems(Update, loading_status.run_if(in_state(Phase::Loading)))
            .add_systems(OnExit(Phase::Loading), despawn_ui)
            .add_systems(OnEnter(Phase::Playing), hud_ui)
            .add_systems(
                Update,
                (
                    fog_texture,
                    enemy_visibility,
                    collect_tracers,
                    draw_gizmos,
                    hud_update,
                    minimap_update,
                    minimap_click,
                    screenshot_key,
                    pause_menu,
                    barricade_repaint,
                    night_tint,
                    objective_markers,
                    name_labels,
                )
                    .run_if(in_state(Phase::Playing).and_then(resource_exists::<GameWorld>)),
            )
            .add_systems(OnExit(Phase::Playing), despawn_ui)
            .add_systems(OnEnter(Phase::GameOver), game_over_ui)
            .add_systems(Update, game_over_input.run_if(in_state(Phase::GameOver)))
            .add_systems(OnExit(Phase::GameOver), despawn_ui);
    }
}

#[derive(Component)]
struct Ui;

#[derive(Component)]
struct MapVisual;

#[derive(Resource)]
struct FogOverlay {
    image: Handle<Image>,
}

/// Map texture handle + original pixels under built barricades.
#[derive(Resource)]
struct MapImage {
    handle: Handle<Image>,
    saved: std::collections::HashMap<usize, Vec<(usize, [u8; 4])>>,
}

#[derive(Resource)]
struct Minimap {
    image: Handle<Image>,
    base: Canvas,
    scale: u32,
    timer: Timer,
}

#[derive(Component)]
struct MinimapNode;

#[derive(Component)]
struct HudText;

#[derive(Resource, Default)]
struct Tracers(Vec<(Vec2, Vec2, bool, f32)>);

fn spawn_camera(mut commands: Commands) {
    commands.spawn((Camera2d, Msaa::Off));
}

// ---------------------------------------------------------------------------
// Session setup / teardown

fn image_from_canvas(images: &mut Assets<Image>, c: &Canvas) -> Handle<Image> {
    let mut img = Image::new(
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
    img.sampler = ImageSampler::nearest();
    images.add(img)
}

/// Called from `setup_session`: the map sprite, fog overlay and minimap.
pub fn spawn_map_visuals(
    commands: &mut Commands,
    g: &Generated,
    world: &GameWorld,
    images: &mut Assets<Image>,
) {
    let map = image_from_canvas(images, &g.composed);
    commands.insert_resource(MapImage {
        handle: map.clone(),
        saved: Default::default(),
    });
    commands.spawn((
        Sprite::from_image(map),
        Transform::from_xyz(0.0, 0.0, 0.0),
        MapVisual,
    ));
    // fog overlay: full-map RGBA, starts opaque black
    let (w, h) = (world.w, world.h);
    let mut fog_img = Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0u8; (w * h * 4) as usize],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    fog_img.sampler = ImageSampler::nearest();
    // start fully opaque
    if let Some(data) = fog_img.data.as_mut() {
        for px in data.chunks_mut(4) {
            px[3] = 255;
        }
    }
    let fog = images.add(fog_img);
    if std::env::var("MAPGEN_NOFOG").is_err() {
        commands.spawn((
            Sprite::from_image(fog.clone()),
            Transform::from_xyz(0.0, 0.0, 10.0),
            MapVisual,
        ));
    }
    commands.insert_resource(FogOverlay { image: fog });
    // minimap base: map downsampled to ≤180 px wide
    let scale = (world.w / 180).max(1);
    let (mw, mh) = (world.w / scale, world.h / scale);
    let mut base = Canvas::new(mw, mh, [0, 0, 0, 255]);
    for y in 0..mh {
        for x in 0..mw {
            let src = ((y * scale) * world.w + x * scale) as usize;
            base.pixels[(y * mw + x) as usize] = g.composed.pixels[src];
        }
    }
    let mm = image_from_canvas(images, &base);
    commands.insert_resource(Minimap {
        image: mm,
        base,
        scale,
        timer: Timer::from_seconds(0.2, TimerMode::Repeating),
    });
}

#[allow(clippy::type_complexity)]
fn cleanup_session(
    mut commands: Commands,
    entities: Query<
        Entity,
        Or<(
            With<StaticWorld>,
            With<Soldier>,
            With<Enemy>,
            With<Corpse>,
            With<SupplyDrop>,
            With<MapVisual>,
        )>,
    >,
    mut cam: Query<&mut Transform, With<Camera2d>>,
) {
    for e in &entities {
        commands.entity(e).despawn();
    }
    commands.remove_resource::<GameWorld>();
    commands.remove_resource::<FogOverlay>();
    commands.remove_resource::<Minimap>();
    commands.remove_resource::<MapImage>();
    commands.remove_resource::<super::economy::LootSites>();
    commands.remove_resource::<super::objectives::Objectives>();
    for mut tf in &mut cam {
        *tf = Transform::IDENTITY;
    }
}

fn despawn_ui(mut commands: Commands, ui: Query<Entity, With<Ui>>) {
    for e in &ui {
        commands.entity(e).despawn();
    }
}

// ---------------------------------------------------------------------------
// Menu / loading / game over

fn text(font: &UiFont, t: &str, size: f32) -> (Text, TextFont, TextColor) {
    (Text::new(t), font.text_font(size), TextColor(Color::WHITE))
}

fn centre_column(commands: &mut Commands) -> Entity {
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: Val::Px(10.0),
                ..default()
            },
            Ui,
        ))
        .id()
}

fn menu_ui(mut commands: Commands, cfg: Res<MapConfig>, font: Res<UiFont>) {
    let root = centre_column(&mut commands);
    commands.entity(root).with_children(|ui| {
        ui.spawn(text(&font, "LAST LIGHT", 52.0));
        ui.spawn(text(&font, "the city sleeps. loot it quietly, reach the evac, get out.", 16.0));
        ui.spawn(text(&font, &format!("map: {}", cfg.bbox), 14.0));
        ui.spawn(text(&font, "", 8.0));
        ui.spawn(text(&font, "Enter – deploy the squad", 20.0));
        ui.spawn(text(
            &font,
            "left drag: select · right click: move/attack · middle drag: pan · wheel: zoom to cursor",
            13.0,
        ));
        ui.spawn(text(&font, "Esc in game shows every control", 13.0));
    });
}

fn menu_input(keys: Res<ButtonInput<KeyCode>>, mut next: ResMut<NextState<Phase>>) {
    // MAPGEN_AUTOSTART=1 skips the menu (used by automated smoke tests)
    if keys.just_pressed(KeyCode::Enter)
        || keys.just_pressed(KeyCode::Space)
        || std::env::var("MAPGEN_AUTOSTART").is_ok()
    {
        next.set(Phase::Loading);
    }
}

fn loading_ui(mut commands: Commands, font: Res<UiFont>) {
    let root = centre_column(&mut commands);
    commands.entity(root).with_children(|ui| {
        ui.spawn(text(&font, "scavenging maps …", 24.0));
        ui.spawn((text(&font, "", 15.0), HudText));
    });
}

fn loading_status(load: Res<MapLoad>, mut q: Query<&mut Text, With<HudText>>) {
    for mut t in &mut q {
        t.0 = load.status.clone();
    }
}

fn game_over_ui(mut commands: Commands, score: Option<Res<Score>>, font: Res<UiFont>) {
    let root = centre_column(&mut commands);
    commands.entity(root).with_children(|ui| {
        let (title, sub) = match score.as_ref().map(|s| s.victory) {
            Some(true) => ("EXTRACTED", "the squad made it out"),
            _ => ("THE LIGHT GOES OUT", "no one is coming"),
        };
        ui.spawn(text(&font, title, 44.0));
        ui.spawn(text(&font, sub, 15.0));
        if let Some(s) = &score {
            ui.spawn(text(
                &font,
                &format!(
                    "{}:{:02} survived · {} kills · {} shots · {} barricades · loudest {:.0}",
                    s.time as u32 / 60,
                    s.time as u32 % 60,
                    s.kills,
                    s.shots,
                    s.barricades_built,
                    s.loudest
                ),
                17.0,
            ));
            if !s.fallen.is_empty() {
                ui.spawn(text(&font, "the fallen:", 14.0));
                for f in s.fallen.iter().take(10) {
                    ui.spawn(text(&font, f, 13.0));
                }
            }
        }
        ui.spawn(text(&font, "R – try again · M – menu", 16.0));
    });
}

fn game_over_input(keys: Res<ButtonInput<KeyCode>>, mut next: ResMut<NextState<Phase>>) {
    if keys.just_pressed(KeyCode::KeyR) {
        next.set(Phase::Menu); // menu clears; loading right after
    }
    if keys.just_pressed(KeyCode::KeyM) {
        next.set(Phase::Menu);
    }
}

// ---------------------------------------------------------------------------
// HUD

const CONTROLS: [(&str, &str); 17] = [
    ("left click / drag", "select soldier / box-select"),
    ("shift + click / drag", "add to selection"),
    ("ctrl + click", "select all of that class on screen"),
    ("F2", "select the whole squad"),
    ("right click", "move (formation) / attack-move an enemy"),
    ("A + left click", "attack-move"),
    ("S / H / P + click", "stop / hold position / patrol"),
    (
        "B + click opening",
        "board up a door/window (again: tear down)",
    ),
    ("ctrl + 1-9", "assign control group"),
    ("1-9 (double-tap)", "recall group (centre camera)"),
    ("middle-mouse drag", "pan the map"),
    ("arrows / W D / edge", "pan the map"),
    ("wheel", "zoom to cursor"),
    ("minimap click", "jump the camera"),
    ("F12", "screenshot"),
    ("Esc", "cancel command / this menu"),
    ("M (game over)", "back to the main menu"),
];

#[derive(Component)]
struct PauseMenu;

/// ESC pause menu: freezes the simulation and lists every control.
fn pause_menu(
    mut commands: Commands,
    paused: Res<Paused>,
    font: Res<UiFont>,
    existing: Query<Entity, With<PauseMenu>>,
) {
    if !paused.is_changed() {
        return;
    }
    for e in &existing {
        commands.entity(e).despawn();
    }
    if !paused.0 {
        return;
    }
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: Val::Px(4.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.02, 0.04, 0.82)),
            GlobalZIndex(50),
            PauseMenu,
            Ui,
        ))
        .with_children(|ui| {
            ui.spawn(text(&font, "PAUSED", 34.0));
            ui.spawn(text(&font, "", 10.0));
            for (key, what) in CONTROLS {
                ui.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(12.0),
                    width: Val::Px(540.0),
                    justify_content: JustifyContent::SpaceBetween,
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        Text::new(key),
                        font.text_font(15.0),
                        TextColor(Color::srgb(1.0, 0.85, 0.4)),
                    ));
                    row.spawn((
                        Text::new(what),
                        font.text_font(15.0),
                        TextColor(Color::srgb(0.85, 0.85, 0.85)),
                    ));
                });
            }
            ui.spawn(text(&font, "", 10.0));
            ui.spawn(text(&font, "Esc – resume", 16.0));
        });
}

fn hud_ui(mut commands: Commands, font: Res<UiFont>) {
    commands.spawn((
        Text::new(""),
        font.text_font(14.0),
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(8.0),
            top: Val::Px(6.0),
            ..default()
        },
        Ui,
        HudText,
    ));
    // minimap container (image filled once Minimap exists)
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(8.0),
            bottom: Val::Px(8.0),
            width: Val::Px(180.0),
            height: Val::Px(180.0),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
        Ui,
        MinimapNode,
        Interaction::default(),
    ));
}

#[allow(clippy::too_many_arguments)]
fn hud_update(
    score: Res<Score>,
    stock: Res<Stockpile>,
    noise: Res<NoiseMeter>,
    daynight: Option<Res<DayNight>>,
    objectives: Res<Objectives>,
    mut feed: MessageReader<DeathFeed>,
    time: Res<Time>,
    mut feed_line: Local<Option<(String, f32)>>,
    soldiers: Query<&Health, With<Soldier>>,
    awake: Query<(), (With<Enemy>, Without<super::units::Dormant>)>,
    selected: Query<(), (With<Soldier>, With<Selected>)>,
    mut q: Query<&mut Text, With<HudText>>,
) {
    for d in feed.read() {
        *feed_line = Some((d.0.clone(), time.elapsed_secs()));
    }
    if let Some((_, t0)) = *feed_line
        && time.elapsed_secs() - t0 > 6.0
    {
        *feed_line = None;
    }
    let squad = soldiers.iter().count();
    let hp: f32 = soldiers.iter().map(|h| h.hp).sum();
    let dial = match daynight.as_deref() {
        Some(d) if d.is_night => "☾ night",
        _ => "☀ day",
    };
    let noise_bar = match noise.0 as u32 {
        0 => "quiet",
        1..=25 => "low",
        26..=60 => "LOUD",
        _ => "DEAFENING",
    };
    let objective = objectives
        .current()
        .map(|o| {
            let what = match o.kind {
                super::objectives::ObjectiveKind::Search => "search",
                super::objectives::ObjectiveKind::Extract => "extract:",
            };
            if objectives.alarm_fired && !o.done {
                format!(
                    "{what} {} — hold {:.0}s",
                    o.name,
                    (super::tuning::EXTRACT_HOLD_S - objectives.hold).max(0.0)
                )
            } else {
                format!("{what} {}", o.name)
            }
        })
        .unwrap_or_else(|| "extract!".into());
    for mut t in &mut q {
        let mut line = format!(
            "{dial} · {objective} · squad {squad} ({hp:.0} hp, {} sel) · ammo {:.0} · meds {:.0} · scrap {:.0} · noise {noise_bar} · kills {}",
            selected.iter().count(),
            stock.ammo,
            stock.meds,
            stock.scrap,
            score.kills,
        );
        if stock.ammo < 30.0 {
            line.push_str("  ⚠ LOW AMMO");
        }
        if let Some((f, _)) = &*feed_line {
            line.push_str(&format!("   {f}"));
        }
        let _ = &awake;
        t.0 = line;
    }
}

// ---------------------------------------------------------------------------
// Fog & enemy visibility

fn fog_texture(world: Res<GameWorld>, fog: Res<FogOverlay>, mut images: ResMut<Assets<Image>>) {
    if !world.is_changed() {
        return;
    }
    let Some(mut img) = images.get_mut(&fog.image) else {
        return;
    };
    let Some(data) = img.data.as_mut() else {
        return;
    };
    // the terrain always shows through — fog only darkens it (deeper where
    // never scouted) and hides units, StarCraft-style
    for (i, s) in world.fog.state.iter().enumerate() {
        data[i * 4 + 3] = match *s {
            super::fog::VISIBLE => 0,
            super::fog::EXPLORED => 90,
            _ => 150,
        };
    }
}

fn enemy_visibility(
    world: Res<GameWorld>,
    mut enemies: Query<(&Transform, &mut Visibility), With<Enemy>>,
) {
    for (tf, mut vis) in &mut enemies {
        let (x, y) = world.to_map(tf.translation.truncate());
        *vis = if world.fog.is_visible(x, y) {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

// ---------------------------------------------------------------------------
// Gizmos: tracers, selection, health bars, box select

fn collect_tracers(
    mut reader: MessageReader<TracerFx>,
    mut tracers: ResMut<Tracers>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs();
    for t in reader.read() {
        tracers.0.push((t.from, t.to, t.heal, now));
    }
    tracers.0.retain(|(_, _, _, t0)| now - t0 < 0.09);
}

#[allow(clippy::too_many_arguments)]
fn draw_gizmos(
    mut gizmos: Gizmos,
    tracers: Res<Tracers>,
    boxsel: Res<BoxSelect>,
    world: Res<GameWorld>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    soldiers: Query<(&Transform, &Health, Option<&Selected>), With<Soldier>>,
    enemies: Query<(&Transform, &Health, &Visibility), With<Enemy>>,
    drops: Query<&Transform, With<SupplyDrop>>,
) {
    for (from, to, heal, _) in &tracers.0 {
        let c = if *heal {
            Color::srgb(0.3, 1.0, 0.5)
        } else {
            Color::srgb(1.0, 0.9, 0.4)
        };
        gizmos.line_2d(*from, *to, c);
    }
    for (tf, hp, sel) in &soldiers {
        let p = tf.translation.truncate();
        if sel.is_some() {
            gizmos.circle_2d(
                Isometry2d::from_translation(p),
                UNIT_RADIUS + 1.2,
                Color::srgb(0.3, 1.0, 0.3),
            );
        }
        health_bar(&mut gizmos, p, hp, Color::srgb(0.2, 0.9, 0.2));
    }
    for (tf, hp, vis) in &enemies {
        if *vis == Visibility::Hidden {
            continue;
        }
        if hp.hp < hp.max {
            health_bar(
                &mut gizmos,
                tf.translation.truncate(),
                hp,
                Color::srgb(0.9, 0.2, 0.2),
            );
        }
    }
    for tf in &drops {
        let p = tf.translation.truncate();
        let (x, y) = world.to_map(p);
        if world.fog.state
            [(y as u32 * world.w + x as u32).min(world.fog.state.len() as u32 - 1) as usize]
            != super::fog::UNEXPLORED
        {
            gizmos.circle_2d(
                Isometry2d::from_translation(p),
                6.0,
                Color::srgb(1.0, 0.85, 0.3),
            );
        }
    }
    // box select rectangle (screen → world)
    if let (Some(start), current) = (boxsel.start, boxsel.current) {
        let (camera, cam_tf) = *camera;
        if let (Ok(a), Ok(b)) = (
            camera.viewport_to_world_2d(cam_tf, start),
            camera.viewport_to_world_2d(cam_tf, current),
        ) {
            let centre = (a + b) / 2.0;
            let size = (a - b).abs();
            gizmos.rect_2d(
                Isometry2d::from_translation(centre),
                size,
                Color::srgb(0.4, 1.0, 0.4),
            );
        }
    }
}

fn health_bar(gizmos: &mut Gizmos, p: Vec2, hp: &Health, colour: Color) {
    let w = 4.0;
    let frac = (hp.hp / hp.max).clamp(0.0, 1.0);
    let y = p.y + UNIT_RADIUS + 2.0;
    gizmos.line_2d(
        Vec2::new(p.x - w / 2.0, y),
        Vec2::new(p.x - w / 2.0 + w * frac, y),
        colour,
    );
}

// ---------------------------------------------------------------------------
// Minimap

#[allow(clippy::too_many_arguments)]
fn minimap_update(
    time: Res<Time>,
    world: Res<GameWorld>,
    mut mm: ResMut<Minimap>,
    mut images: ResMut<Assets<Image>>,
    mut node: Query<Entity, (With<MinimapNode>, Without<ImageNode>)>,
    mut commands: Commands,
    soldiers: Query<&Transform, With<Soldier>>,
    enemies: Query<(&Transform, &Visibility), With<Enemy>>,
) {
    for e in &mut node {
        commands.entity(e).insert(ImageNode::new(mm.image.clone()));
    }
    mm.timer.tick(time.delta());
    if !mm.timer.just_finished() {
        return;
    }
    let scale = mm.scale;
    let (mw, mh) = (mm.base.width, mm.base.height);
    let mut frame = mm.base.clone();
    // fog
    for y in 0..mh {
        for x in 0..mw {
            let src = ((y * scale) * world.w + x * scale) as usize;
            let i = (y * mw + x) as usize;
            // terrain stays readable on the minimap too; fog just darkens it
            let dim = match world.fog.state[src] {
                super::fog::UNEXPLORED => Some(5u16),
                super::fog::EXPLORED => Some(3),
                _ => None,
            };
            if let Some(k) = dim {
                let p = &mut frame.pixels[i];
                for c in p.iter_mut().take(3) {
                    *c = (*c as u16 * (8 - k) / 8) as u8;
                }
            }
        }
    }
    let mut blip = |wpos: Vec2, c: [u8; 4]| {
        let (x, y) = world.to_map(wpos);
        let (x, y) = ((x / scale as f32) as i32, (y / scale as f32) as i32);
        for dy in -1..=1 {
            for dx in -1..=1 {
                frame.set(x + dx, y + dy, c);
            }
        }
    };
    for tf in &soldiers {
        blip(tf.translation.truncate(), [80, 255, 80, 255]);
    }
    for (tf, vis) in &enemies {
        if *vis == Visibility::Visible {
            blip(tf.translation.truncate(), [255, 60, 60, 255]);
        }
    }
    if let Some(mut img) = images.get_mut(&mm.image) {
        img.data = Some(frame.to_rgba_bytes());
    }
}

fn minimap_click(
    interaction: Query<&Interaction, With<MinimapNode>>,
    window: Single<&Window, With<PrimaryWindow>>,
    world: Res<GameWorld>,
    mut cam: Query<&mut Transform, With<Camera2d>>,
) {
    let Ok(Interaction::Pressed) = interaction.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    // minimap rect: right/bottom 8px, 180×180
    let (w, h) = (window.width(), window.height());
    let rel = Vec2::new(
        (cursor.x - (w - 188.0)) / 180.0,
        (cursor.y - (h - 188.0)) / 180.0,
    );
    if !(0.0..=1.0).contains(&rel.x) || !(0.0..=1.0).contains(&rel.y) {
        return;
    }
    let target = world.to_world(rel.x * world.w as f32, rel.y * world.h as f32);
    for mut tf in &mut cam {
        tf.translation.x = target.x;
        tf.translation.y = target.y;
    }
}

// ---------------------------------------------------------------------------

fn screenshot_key(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut auto_done: Local<bool>,
    mut n: Local<u32>,
) {
    let auto = std::env::var("MAPGEN_AUTOSHOT")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .map(|t| time.elapsed_secs() > t && !*auto_done)
        .unwrap_or(false);
    if auto {
        *auto_done = true;
    }
    if keys.just_pressed(KeyCode::F12) || auto {
        *n += 1;
        std::fs::create_dir_all("out").ok();
        let path = format!("out/screenshot-{:03}.png", *n);
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
    }
}

/// Repaint the map texture when barricades go up or come down.
fn barricade_repaint(
    world: Res<GameWorld>,
    mut map: ResMut<MapImage>,
    mut images: ResMut<Assets<Image>>,
    mut msgs: MessageReader<super::barricade::RepaintFx>,
) {
    for m in msgs.read() {
        let Some(op) = world.openings.get(m.opening) else {
            continue;
        };
        let handle = map.handle.clone();
        let Some(mut img) = images.get_mut(&handle) else {
            continue;
        };
        let Some(data) = img.data.as_mut() else {
            continue;
        };
        if m.built {
            let mut saved = Vec::new();
            for &i in &op.pixels {
                let o = i * 4;
                saved.push((i, [data[o], data[o + 1], data[o + 2], data[o + 3]]));
                // plank brown
                data[o] = 122;
                data[o + 1] = 84;
                data[o + 2] = 46;
                data[o + 3] = 255;
            }
            map.saved.insert(m.opening, saved);
        } else if let Some(saved) = map.saved.remove(&m.opening) {
            for (i, px) in saved {
                let o = i * 4;
                data[o..o + 4].copy_from_slice(&px);
            }
        }
    }
}

#[derive(Component)]
struct NightShade;

/// Blue-dark tint at night (between the map and the fog overlay).
fn night_tint(
    mut commands: Commands,
    daynight: Option<Res<DayNight>>,
    world: Res<GameWorld>,
    mut shade: Query<&mut Sprite, With<NightShade>>,
) {
    let night = daynight.map(|d| d.is_night).unwrap_or(false);
    let target = if night { 0.30 } else { 0.0 };
    if let Ok(mut sp) = shade.single_mut() {
        let a = sp.color.alpha();
        sp.color.set_alpha(a + (target - a) * 0.02);
    } else {
        commands.spawn((
            Sprite {
                color: Color::srgba(0.05, 0.08, 0.25, 0.0),
                custom_size: Some(Vec2::new(world.w as f32, world.h as f32)),
                ..default()
            },
            Transform::from_xyz(0.0, 0.0, 9.0),
            NightShade,
            MapVisual,
        ));
    }
}

/// Flag ring on the current objective; an edge arrow when it's off-screen.
fn objective_markers(
    mut gizmos: Gizmos,
    objectives: Res<super::objectives::Objectives>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    window: Single<&Window, With<PrimaryWindow>>,
    time: Res<Time>,
) {
    let Some(current) = objectives.current() else {
        return;
    };
    let colour = match current.kind {
        super::objectives::ObjectiveKind::Search => Color::srgb(0.4, 0.8, 1.0),
        super::objectives::ObjectiveKind::Extract => Color::srgb(0.4, 1.0, 0.5),
    };
    let pulse = 1.0 + (time.elapsed_secs() * 3.0).sin() * 0.15;
    let r = match current.kind {
        super::objectives::ObjectiveKind::Extract => super::tuning::EXTRACT_RADIUS,
        _ => super::tuning::MID_OBJECTIVE_RADIUS,
    };
    gizmos.circle_2d(Isometry2d::from_translation(current.pos), r * pulse, colour);
    gizmos.circle_2d(Isometry2d::from_translation(current.pos), 1.5, colour);
    // off-screen edge arrow
    let (cam, cam_tf) = *camera;
    if let Ok(screen) = cam.world_to_viewport(cam_tf, current.pos.extend(0.0)) {
        let (w, h) = (window.width(), window.height());
        if screen.x < 0.0 || screen.y < 0.0 || screen.x > w || screen.y > h {
            let clamped = Vec2::new(
                screen.x.clamp(30.0, w - 30.0),
                screen.y.clamp(30.0, h - 30.0),
            );
            if let (Ok(a), Ok(b)) = (
                cam.viewport_to_world_2d(cam_tf, clamped),
                cam.viewport_to_world_2d(
                    cam_tf,
                    clamped + (screen - clamped).normalize_or_zero() * 14.0,
                ),
            ) {
                gizmos.arrow_2d(a, b, colour);
            }
        }
    }
}

#[derive(Component)]
struct NameTag(Entity);

/// Soldier names float above them when zoomed in close.
#[allow(clippy::type_complexity)]
fn name_labels(
    mut commands: Commands,
    font: Res<UiFont>,
    cam: Single<&Transform, With<Camera2d>>,
    soldiers: Query<(Entity, &Transform, &Dossier), With<Soldier>>,
    mut tags: Query<
        (Entity, &NameTag, &mut Transform, &mut Visibility),
        (Without<Soldier>, Without<Camera2d>),
    >,
) {
    let close = cam.scale.x < 0.55;
    let mut have: std::collections::HashSet<Entity> = Default::default();
    for (te, tag, mut ttf, mut vis) in &mut tags {
        match soldiers.get(tag.0) {
            Ok((_, stf, _)) => {
                have.insert(tag.0);
                ttf.translation =
                    stf.translation.truncate().extend(20.0) + Vec3::new(0.0, 6.5, 0.0);
                *vis = if close {
                    Visibility::Visible
                } else {
                    Visibility::Hidden
                };
            }
            Err(_) => commands.entity(te).despawn(),
        }
    }
    if close {
        for (se, stf, dossier) in &soldiers {
            if !have.contains(&se) {
                let rank = "|".repeat(dossier.rank() as usize);
                commands.spawn((
                    Text2d::new(format!("{} {rank}", dossier.name)),
                    font.text_font(10.0),
                    TextColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
                    Transform::from_translation(
                        stf.translation.truncate().extend(20.0) + Vec3::new(0.0, 6.5, 0.0),
                    )
                    .with_scale(Vec3::splat(0.16)),
                    NameTag(se),
                    MapVisual,
                ));
            }
        }
    }
}
