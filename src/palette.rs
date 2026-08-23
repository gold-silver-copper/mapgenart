//! Colour palette. The defaults approximate the classic Q-BAM alt-history
//! template (light blue seas, pale land, thin dark borders) with a few extra
//! muted tones for OSM land-cover. Override any entry with `--palette file.toml`
//! (see `palettes/qbam.toml`).

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer};
use std::path::Path;

pub type Rgba = [u8; 4];

pub const fn hex(hex: u32) -> Rgba {
    [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8, 255]
}

/// Parse `#rrggbb` / `rrggbb` / `#rrggbbaa`.
pub fn parse_hex(s: &str) -> Result<Rgba> {
    let t = s.trim().trim_start_matches('#');
    let v = u32::from_str_radix(t, 16).with_context(|| format!("bad colour `{s}`"))?;
    match t.len() {
        6 => Ok(hex(v)),
        8 => Ok([(v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8]),
        _ => bail!("colour `{s}` must be 6 or 8 hex digits"),
    }
}

pub fn to_hex(c: Rgba) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn de_rgba<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Rgba, D::Error> {
    let s = String::deserialize(d)?;
    parse_hex(&s).map_err(serde::de::Error::custom)
}

macro_rules! palette {
    ($($name:ident = $val:expr),* $(,)?) => {
        #[derive(Debug, Clone, Deserialize, PartialEq)]
        #[serde(default)]
        pub struct Palette {
            $(#[serde(deserialize_with = "de_rgba")] pub $name: Rgba,)*
        }
        impl Default for Palette {
            fn default() -> Self {
                Palette { $($name: hex($val),)* }
            }
        }
        impl Palette {
            /// Every colour in the palette (for quantisation).
            pub fn colours(&self) -> Vec<Rgba> {
                vec![$(self.$name,)*]
            }
        }
    };
}

palette! {
    ocean = 0x9EC7F3,
    lake = 0x9EC7F3,
    river = 0x7FB2EA,
    land = 0xF2EEE3,
    shoreline = 0x6F9ED0,
    forest = 0xB5D39A,
    grass = 0xCFE3B3,
    farmland = 0xE8E2C4,
    sand = 0xF0E4AE,
    wetland = 0xC4DACF,
    urban = 0xDDD4CC,
    industrial = 0xD2CACE,
    building = 0x9A8C82,
    road_major = 0xC98F62,
    road_minor = 0xB7ACA0,
    rail = 0x7B7B7B,
    border_country = 0x2B2B2B,
    border_region = 0x8A8A8A,
    border_local = 0xBDBDBD,
    grid = 0x00000030,
}

impl Palette {
    pub fn load(path: &Path) -> Result<Palette> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing palette {}", path.display()))
    }

    /// Deterministic pastel colour for an unassigned political region.
    pub fn region_colour(id: i64) -> Rgba {
        let mut h = (id as u64) ^ 0x9E37_79B9_7F4A_7C15;
        h ^= h >> 33;
        h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
        h ^= h >> 33;
        let hue = (h % 360) as f32;
        hsl(hue, 0.45, 0.80)
    }

    /// Nine hand-picked Q-BAM-ish political colours, bound to keys 1–9 in the editor.
    pub const PRESETS: [Rgba; 9] = [
        hex(0xE8A3A3), // red
        hex(0xA3C4E8), // blue
        hex(0xA8DBA8), // green
        hex(0xF2DA9A), // yellow
        hex(0xD6B3E8), // purple
        hex(0xF2C199), // orange
        hex(0xA3E0DC), // teal
        hex(0xD9D9D9), // grey
        hex(0xC8B59A), // tan
    ];
}

/// HSL → RGBA (hue in degrees).
pub fn hsl(h: f32, s: f32, l: f32) -> Rgba {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h % 360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
        255,
    ]
}

/// Rotate a colour's hue by `deg` degrees (used by `[` / `]` in the editor).
pub fn rotate_hue(c: Rgba, deg: f32) -> Rgba {
    let (r, g, b) = (
        c[0] as f32 / 255.0,
        c[1] as f32 / 255.0,
        c[2] as f32 / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d < 1e-6 {
        return hsl((deg).rem_euclid(360.0), 0.45, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        60.0 * (((g - b) / d) % 6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    hsl((h + deg).rem_euclid(360.0), s, l)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        assert_eq!(parse_hex("#9ec7f3").unwrap(), hex(0x9EC7F3));
        assert_eq!(to_hex(hex(0x9EC7F3)), "#9ec7f3");
    }

    #[test]
    fn toml_override() {
        let p: Palette = toml::from_str("ocean = \"#000000\"").unwrap();
        assert_eq!(p.ocean, [0, 0, 0, 255]);
        assert_eq!(p.land, Palette::default().land);
    }

    #[test]
    fn region_colour_is_deterministic() {
        assert_eq!(Palette::region_colour(42), Palette::region_colour(42));
        assert_ne!(Palette::region_colour(42), Palette::region_colour(43));
    }
}
