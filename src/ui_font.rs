//! Embedded UI font: an Iosevka subset (Latin, Latin-Extended, punctuation,
//! arrows, geometric shapes — SIL OFL 1.1, see `assets/fonts/LICENSE.md`),
//! compiled into the binary so every label renders without tofu on any
//! platform, wasm included.

use bevy::prelude::*;
use bevy::text::Font;

static IOSEVKA: &[u8] = include_bytes!("../assets/fonts/Iosevka-Subset.ttf");

/// Handle to the embedded font; use [`UiFont::text_font`] for `TextFont`s.
#[derive(Resource, Clone)]
pub struct UiFont(pub Handle<Font>);

impl UiFont {
    pub fn text_font(&self, px: f32) -> TextFont {
        TextFont {
            font: self.0.clone().into(),
            font_size: FontSize::Px(px),
            ..default()
        }
    }
}

pub struct UiFontPlugin;

impl Plugin for UiFontPlugin {
    fn build(&self, app: &mut App) {
        if app.world().contains_resource::<UiFont>() {
            return; // game + editor may both add this plugin
        }
        let font = Font::from_bytes(IOSEVKA.to_vec());
        let handle = app.world_mut().resource_mut::<Assets<Font>>().add(font);
        app.insert_resource(UiFont(handle));
    }
}
