//! Everything visual: map + fog textures, gizmo overlays (selection rings,
//! health bars, tracers, box select), minimap, HUD, menu / game-over screens,
//! F12 screenshots. Kept apart from `logic` so headless runs skip it all.

use super::control::BoxSelect;
use super::logic::{Score, SquadBuffs, TracerFx, WaveState};
use super::units::*;
use super::world::{GameWorld, StaticWorld};
use super::{MapLoad, Phase};
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
        ui.spawn(text(&font, "real-time tactics on real-world ruins", 16.0));
        ui.spawn(text(&font, &format!("map: {}", cfg.bbox), 14.0));
        ui.spawn(text(&font, "", 8.0));
        ui.spawn(text(&font, "Enter – deploy the squad", 20.0));
        ui.spawn(text(
            &font,
            "left drag: select · right click: move · A: attack-move · S/H: stop/hold · P: patrol",
            13.0,
        ));
        ui.spawn(text(
            &font,
            "Ctrl+1-9: control groups · wheel: zoom · F12: screenshot",
            13.0,
        ));
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
    let (kills, waves) = score.map(|s| (s.kills, s.waves_survived)).unwrap_or((0, 0));
    commands.entity(root).with_children(|ui| {
        ui.spawn(text(&font, "THE LIGHT GOES OUT", 44.0));
        ui.spawn(text(
            &font,
            &format!("waves survived: {waves} · kills: {kills}"),
            20.0,
        ));
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
    wave: Res<WaveState>,
    buffs: Res<SquadBuffs>,
    soldiers: Query<&Health, With<Soldier>>,
    enemies: Query<(), With<Enemy>>,
    selected: Query<(), (With<Soldier>, With<Selected>)>,
    mut q: Query<&mut Text, With<HudText>>,
) {
    let squad = soldiers.iter().count();
    let hp: f32 = soldiers.iter().map(|h| h.hp).sum();
    let next = match &wave.countdown {
        Some(t) => format!(
            "next wave in {:.0}s",
            (t.duration() - t.elapsed()).as_secs_f32()
        ),
        None => format!("{} hostiles", enemies.iter().count()),
    };
    for mut t in &mut q {
        t.0 = format!(
            "wave {} · {next} · squad {squad} ({:.0} hp) · {} selected · kills {} · dmg +{:.0}%",
            wave.wave,
            hp,
            selected.iter().count(),
            score.kills,
            buffs.damage_mult * 100.0
        );
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
    for (i, s) in world.fog.state.iter().enumerate() {
        data[i * 4 + 3] = match *s {
            super::fog::VISIBLE => 0,
            super::fog::EXPLORED => 110,
            _ => 255,
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
                UNIT_RADIUS + 2.0,
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
    let w = 8.0;
    let frac = (hp.hp / hp.max).clamp(0.0, 1.0);
    let y = p.y + UNIT_RADIUS + 4.0;
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
            match world.fog.state[src] {
                super::fog::UNEXPLORED => frame.pixels[i] = [8, 8, 10, 255],
                super::fog::EXPLORED => {
                    let p = &mut frame.pixels[i];
                    for c in p.iter_mut().take(3) {
                        *c /= 2;
                    }
                }
                _ => {}
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
