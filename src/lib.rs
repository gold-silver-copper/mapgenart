#![allow(clippy::type_complexity)]

pub mod config;
pub mod font;
pub mod generate;
pub mod labels;
pub mod land;
pub mod osm;
pub mod palette;
pub mod postfx;
pub mod raster;
pub mod scenario;
pub mod viewer;

use bevy::prelude::*;

pub use config::MapConfig;

/// Bevy plugin that hosts the interactive map viewer. Insert a [`MapConfig`]
/// resource before adding it (or let it fall back to defaults).
pub struct MapGenPlugin;

impl Plugin for MapGenPlugin {
    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<MapConfig>() {
            use clap::Parser;
            app.insert_resource(MapConfig::parse_from::<[&str; 1], _>(["mapgenart"]));
        }
        app.add_plugins(viewer::ViewerPlugin);
    }
}
