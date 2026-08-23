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
    /// Optional fill pattern: "hatch" (diagonal lines) or "dots".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// Second colour used by `pattern` (hex; defaults to a darkened fill).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern_color: Option<String>,
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

    /// Assign a region (by id) to an owner, clearing any per-region colour so
    /// the owner colour shows. Returns the previous assignment.
    pub fn assign_owner(&mut self, id: i64, owner: Option<&str>) -> Assignment {
        let e = self.regions.entry(id.to_string()).or_default();
        let prev = e.clone();
        e.owner = owner.map(str::to_string);
        e.color = None;
        prev
    }

    /// The owner a region resolves to (id first, then name).
    pub fn owner_of(&self, id: i64, name: Option<&str>) -> Option<&str> {
        self.assignment(id, name).and_then(|a| a.owner.as_deref())
    }

    /// Set / change an owner's colour.
    pub fn set_owner_colour(&mut self, owner: &str, colour: Rgba) {
        self.owners
            .insert(owner.to_string(), palette::to_hex(colour));
    }

    pub fn owner_colour(&self, owner: &str) -> Option<Rgba> {
        self.owners
            .get(owner)
            .and_then(|c| palette::parse_hex(c).ok())
    }

    /// Merge `other` over `self` (later wins per key).
    pub fn merge(&mut self, other: Scenario) {
        self.owners.extend(other.owners);
        self.regions.extend(other.regions);
    }

    /// Load and merge several scenario files in order.
    pub fn load_all(paths: &[std::path::PathBuf]) -> Result<Scenario> {
        let mut out = Scenario::default();
        for p in paths {
            if p.exists() {
                out.merge(Scenario::load(p)?);
            } else {
                log::info!(
                    "scenario {} does not exist yet; starting empty",
                    p.display()
                );
            }
        }
        Ok(out)
    }

    /// Pattern for a region, if any: (kind, colour).
    pub fn pattern_for(&self, id: i64, name: Option<&str>) -> Option<(Pattern, Rgba)> {
        let a = self.assignment(id, name)?;
        let kind = match a.pattern.as_deref()? {
            "hatch" => Pattern::Hatch,
            "dots" => Pattern::Dots,
            other => {
                log::warn!("unknown pattern `{other}`");
                return None;
            }
        };
        let base = self.colour_for(id, name);
        let colour = a
            .pattern_color
            .as_deref()
            .and_then(|c| palette::parse_hex(c).ok())
            .unwrap_or([
                (base[0] as u32 * 3 / 4) as u8,
                (base[1] as u32 * 3 / 4) as u8,
                (base[2] as u32 * 3 / 4) as u8,
                255,
            ]);
        Some((kind, colour))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    Hatch,
    Dots,
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
