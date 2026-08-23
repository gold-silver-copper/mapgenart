//! Alt-history scenarios: a TOML file assigning owners / colours / labels to
//! OSM administrative relations, keyed by relation id or by name.
//!
//! ```toml
//! [owners]
//! "Kalmar Union" = "#d6b3e8"
//!
//! [regions.2192363]            # relation id
//! owner = "Kalmar Union"
//! label = "Zealand"
//!
//! [regions."Region Hovedstaden"] # or by OSM name
//! color = "#f2c199"
//! ```

use crate::palette::{self, Rgba};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Assignment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Hex colour; wins over the owner's colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Scenario {
    /// Owner name → hex colour.
    #[serde(default)]
    pub owners: BTreeMap<String, String>,
    /// Relation id (as string) or region name → assignment.
    #[serde(default)]
    pub regions: BTreeMap<String, Assignment>,
}

impl Scenario {
    pub fn load(path: &Path) -> Result<Scenario> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing scenario {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let text = toml::to_string_pretty(self).context("serialising scenario")?;
        std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
    }

    /// Look up by id first, then by name.
    pub fn assignment(&self, id: i64, name: Option<&str>) -> Option<&Assignment> {
        self.regions
            .get(&id.to_string())
            .or_else(|| name.and_then(|n| self.regions.get(n)))
    }

    /// Resolve the fill colour for a region: explicit colour, else owner colour,
    /// else a deterministic hash colour.
    pub fn colour_for(&self, id: i64, name: Option<&str>) -> Rgba {
        if let Some(a) = self.assignment(id, name) {
            if let Some(c) = a.color.as_deref().and_then(|c| palette::parse_hex(c).ok()) {
                return c;
            }
            if let Some(c) = a
                .owner
                .as_deref()
                .and_then(|o| self.owners.get(o))
                .and_then(|c| palette::parse_hex(c).ok())
            {
                return c;
            }
        }
        palette::Palette::region_colour(id)
    }

    /// Record an explicit colour for a region (by id), creating the entry.
    pub fn set_colour(&mut self, id: i64, colour: Rgba) {
        let e = self.regions.entry(id.to_string()).or_default();
        e.color = Some(palette::to_hex(colour));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_owner_then_hash() {
        let s: Scenario = toml::from_str(
            r##"
            [owners]
            "A" = "#ff0000"
            [regions.1]
            owner = "A"
            [regions."Foo"]
            color = "#00ff00"
            "##,
        )
        .unwrap();
        assert_eq!(s.colour_for(1, None), [255, 0, 0, 255]);
        assert_eq!(s.colour_for(2, Some("Foo")), [0, 255, 0, 255]);
        assert_eq!(s.colour_for(3, None), palette::Palette::region_colour(3));
    }

    #[test]
    fn roundtrip() {
        let mut s = Scenario::default();
        s.set_colour(5, [1, 2, 3, 255]);
        let text = toml::to_string(&s).unwrap();
        let back: Scenario = toml::from_str(&text).unwrap();
        assert_eq!(s, back);
    }
}
