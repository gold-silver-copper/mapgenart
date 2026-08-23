// disable console on windows for release builds
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use bevy::DefaultPlugins;
use bevy::asset::AssetMetaCheck;
use bevy::ecs::system::NonSendMarker;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy::winit::WINIT_WINDOWS;
use clap::Parser;
use mapgenart::{MapConfig, MapGenPlugin};
use std::io::Cursor;
use winit::window::Icon;

fn main() {
    #[allow(unused_mut)]
    let mut cfg = MapConfig::parse();
    #[cfg(target_arch = "wasm32")]
    apply_query_params(&mut cfg);
    if cfg.headless || cfg.list_regions {
        if let Err(e) = run_headless(&cfg) {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    App::new()
        .insert_resource(cfg)
        .insert_resource(ClearColor(Color::linear_rgb(0.12, 0.12, 0.14)))
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Map Art Generator".to_string(),
                        // Bind to canvas included in `index.html`
                        canvas: Some("#bevy".to_owned()),
                        fit_canvas_to_parent: true,
                        // Tells wasm not to override default event handling, like F5 and Ctrl+R
                        prevent_default_event_handling: false,
                        ..default()
                    }),
                    ..default()
                })
                .set(AssetPlugin {
                    meta_check: AssetMetaCheck::Never,
                    ..default()
                }),
        )
        .add_plugins(MapGenPlugin)
        .add_systems(Startup, set_window_icon)
        .run();
}

/// On the web build, allow `?bbox=S,W,N,E&width=N&scale=N` in the page URL.
#[cfg(target_arch = "wasm32")]
fn apply_query_params(cfg: &mut MapConfig) {
    // the web demo renders the bundled fixture; default the bbox to match it
    cfg.bbox = "55.674,12.588,55.686,12.602".to_string();
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(search) = window.location().search() else {
        return;
    };
    let Ok(params) = web_sys::UrlSearchParams::new_with_str(&search) else {
        return;
    };
    if let Some(b) = params.get("bbox") {
        cfg.bbox = b;
    }
    if let Some(w) = params.get("width").and_then(|w| w.parse().ok()) {
        cfg.width = w;
    }
    if let Some(s) = params.get("scale").and_then(|s| s.parse().ok()) {
        cfg.scale = s;
    }
}

fn run_headless(cfg: &MapConfig) -> anyhow::Result<()> {
    let generated = mapgenart::generate::generate(cfg)?;
    if cfg.list_regions {
        println!(
            "{}",
            mapgenart::generate::list_regions(&generated, cfg.json)
        );
        return Ok(());
    }
    let canvas = generated.canvas();
    let paths = mapgenart::generate::export(&generated, cfg)?;
    println!(
        "{}x{} px ({:.0} m/px) from {} features, {} political regions{} -> {}",
        canvas.width,
        canvas.height,
        generated.metres_per_pixel,
        generated.features.len(),
        generated.rendered.regions.len(),
        generated
            .rendered
            .admin_level_used
            .map(|l| format!(" (admin_level {l})"))
            .unwrap_or_default(),
        paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(())
}

// Sets the icon on windows and X11
fn set_window_icon(
    primary_window: Single<Entity, With<PrimaryWindow>>,
    _non_send_marker: NonSendMarker,
) -> Result {
    WINIT_WINDOWS.with_borrow(|windows| {
        let Some(primary) = windows.get_window(*primary_window) else {
            return Err(BevyError::from("No primary window!"));
        };
        let icon_buf = Cursor::new(include_bytes!(
            "../build/macos/AppIcon.iconset/icon_256x256.png"
        ));
        if let Ok(image) = image::load(icon_buf, image::ImageFormat::Png) {
            let image = image.into_rgba8();
            let (width, height) = image.dimensions();
            let rgba = image.into_raw();
            let icon = Icon::from_rgba(rgba, width, height).unwrap();
            primary.set_window_icon(Some(icon));
        };

        Ok(())
    })
}
