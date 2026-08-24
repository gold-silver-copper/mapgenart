//! End-to-end pipeline: config → Overpass (tiles) → features → canvas →
//! post-fx → overlays (patterns, owner borders, labels) → PNG.

use crate::config::MapConfig;
use crate::labels::{self, LabelOptions};
use crate::osm::{self, Feature, Progress};
use crate::palette::{self, Palette, Rgba};
use crate::postfx::{self, PostFx};
use crate::raster::{self, Canvas, Detail, Overlay, RenderOptions, Rendered, layer};
use crate::scenario::{Pattern, Scenario};
use crate::{font, land};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub struct Generated {
    pub rendered: Rendered,
    /// Base canvas + patterns + derived borders + labels, ready for display.
    pub composed: Canvas,
    pub features: Vec<Feature>,
    pub palette: Palette,
    pub scenario: Scenario,
    pub metres_per_pixel: f64,
    /// The bbox / admin level this map was generated with (drill-down state).
    pub bbox_string: String,
    pub admin_level_requested: u8,
}

impl Generated {
    pub fn canvas(&self) -> &Canvas {
        &self.composed
    }
}

/// Load palette + merged scenarios named in the config.
pub fn load_style(cfg: &MapConfig) -> Result<(Palette, Scenario)> {
    let palette = match &cfg.palette {
        Some(p) => Palette::load(p)?,
        None => Palette::default(),
    };
    #[cfg(target_arch = "wasm32")]
    let scenario = {
        let _ = &cfg.scenario;
        toml::from_str(include_str!("../scenarios/example.toml")).unwrap_or_default()
    };
    #[cfg(not(target_arch = "wasm32"))]
    let scenario = Scenario::load_all(&cfg.scenario)?;
    Ok((palette, scenario))
}

pub fn generate(cfg: &MapConfig) -> Result<Generated> {
    generate_with_progress(cfg, &|m| log::info!("{m}"))
}

pub fn generate_with_progress(cfg: &MapConfig, progress: Progress) -> Result<Generated> {
    let bbox = cfg.bbox()?;
    let (palette, scenario) = load_style(cfg)?;
    let tiles = osm::load_tiles(cfg, &bbox, progress)?;
    progress("Parsing …".into());
    let features = osm::parse_many(&tiles)?;
    log::info!(
        "parsed {} features from {} tile(s)",
        features.len(),
        tiles.len()
    );
    let land = match &cfg.land {
        Some(p) => Some(land::load(p)?),
        None => None,
    };
    progress("Rendering …".into());
    render_features(cfg, features, &palette, scenario, land.as_ref())
}

/// Owner list and per-region owner index (`u32::MAX` = unowned).
pub fn owner_index(rendered: &Rendered, scenario: &Scenario) -> (Vec<String>, Vec<u32>) {
    let mut owners: Vec<String> = scenario.owners.keys().cloned().collect();
    for r in &rendered.regions {
        if let Some(o) = scenario.owner_of(r.id, r.name.as_deref())
            && !owners.iter().any(|x| x == o)
        {
            owners.push(o.to_string());
        }
    }
    let of = rendered
        .regions
        .iter()
        .map(|r| {
            scenario
                .owner_of(r.id, r.name.as_deref())
                .and_then(|o| owners.iter().position(|x| x == o))
                .map(|i| i as u32)
                .unwrap_or(u32::MAX)
        })
        .collect();
    (owners, of)
}

/// Hatch / dot patterns for regions that declare one in the scenario.
pub fn pattern_overlay(rendered: &Rendered, scenario: &Scenario) -> Overlay {
    let mut ov: Overlay = vec![None; rendered.region_ids.len()];
    let w = rendered.canvas.width as usize;
    let patterns: Vec<Option<(Pattern, Rgba)>> = rendered
        .regions
        .iter()
        .map(|r| scenario.pattern_for(r.id, r.name.as_deref()))
        .collect();
    if patterns.iter().all(Option::is_none) {
        return ov;
    }
    for (i, id) in rendered.region_ids.iter().enumerate() {
        if *id == u32::MAX || rendered.canvas.tags[i] != layer::REGION {
            continue;
        }
        if let Some((kind, colour)) = patterns[*id as usize] {
            let (x, y) = (i % w, i / w);
            let on = match kind {
                Pattern::Hatch => (x + y) % 4 == 0,
                Pattern::Dots => x % 3 == 1 && y % 3 == 1,
            };
            if on {
                ov[i] = Some(colour);
            }
        }
    }
    ov
}

/// Recompute overlays + composition (used by the pipeline and the editor).
pub fn compose(
    rendered: &Rendered,
    features: &[Feature],
    scenario: &Scenario,
    cfg: &MapConfig,
    pal: &Palette,
    mpp: f64,
) -> Canvas {
    let patterns = pattern_overlay(rendered, scenario);
    let (_, owner_of) = owner_index(rendered, scenario);
    let borders = if rendered.regions.is_empty() || cfg.osm_borders {
        vec![None; rendered.region_ids.len()]
    } else {
        raster::derive_owner_borders(rendered, &owner_of, pal, cfg.inner_borders)
    };
    let label_colour = palette::parse_hex(&cfg.label_color).unwrap_or([58, 58, 58, 255]);
    let labels = if cfg.labels || cfg.cities {
        labels::build(
            rendered,
            features,
            scenario,
            &LabelOptions {
                regions: cfg.labels,
                cities: cfg.cities,
                colour: label_colour,
                metres_per_pixel: mpp,
            },
        )
    } else {
        vec![None; rendered.region_ids.len()]
    };
    raster::compose(
        &rendered.canvas,
        &[
            (&patterns, layer::REGION),
            (&borders, layer::LINE),
            (&labels, layer::LABEL),
        ],
    )
}

/// Render already-parsed features (shared by the CLI pipeline and tests).
pub fn render_features(
    cfg: &MapConfig,
    features: Vec<Feature>,
    palette: &Palette,
    scenario: Scenario,
    land: Option<&land::LandPolygons>,
) -> Result<Generated> {
    let bbox = cfg.bbox()?;
    let width = cfg.width.max(8);
    let mpp = bbox.metres_per_pixel(width);
    let mut detail = Detail::for_scale(mpp);
    if cfg.no_roads {
        detail.minor_roads = false;
        detail.major_roads = false;
        detail.rail = false;
    }
    if !cfg.buildings {
        detail.buildings = false;
    }
    let opts = RenderOptions {
        palette,
        scenario: &scenario,
        detail,
        political_level: if cfg.no_political {
            None
        } else {
            Some(cfg.admin_level)
        },
        land,
        osm_borders: cfg.osm_borders,
        enterable_buildings: cfg.enterable,
    };
    let mut rendered = raster::render(&features, bbox, width, &opts);
    let fx = PostFx {
        smooth: cfg.smooth,
        min_feature: cfg.min_feature,
        shoreline: cfg.shoreline,
        quantize: cfg.quantize,
    };
    postfx::apply(&mut rendered.canvas, &fx, palette);
    let composed = compose(&rendered, &features, &scenario, cfg, palette, mpp);
    Ok(Generated {
        rendered,
        composed,
        features,
        palette: palette.clone(),
        scenario,
        metres_per_pixel: mpp,
        bbox_string: cfg.bbox.clone(),
        admin_level_requested: cfg.admin_level,
    })
}

/// Write `path` (1×) and `path@{scale}x` (nearest-neighbour upscale, optional
/// grid); with `legend` also `path.legend.png`.
pub fn export(g: &Generated, cfg: &MapConfig) -> Result<Vec<PathBuf>> {
    let mut written = export_canvas(g.canvas(), &cfg.output, cfg.scale, cfg.grid, &g.palette)?;
    if cfg.legend {
        let (owners, _) = owner_index(&g.rendered, &g.scenario);
        if !owners.is_empty() {
            let stem = cfg
                .output
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("map");
            let path = cfg.output.with_file_name(format!("{stem}.legend.png"));
            save_png(&legend_canvas(&owners, &g.scenario, &g.palette), &path)?;
            written.push(path);
        }
    }
    Ok(written)
}

pub fn export_canvas(
    canvas: &Canvas,
    path: &Path,
    scale: u32,
    grid: bool,
    pal: &Palette,
) -> Result<Vec<PathBuf>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let mut written = vec![];
    save_png(canvas, path)?;
    written.push(path.to_path_buf());
    if scale > 1 {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("map");
        let scaled = path.with_file_name(format!("{stem}@{scale}x.png"));
        let mut up = canvas.upscale(scale);
        if grid {
            up.draw_grid(scale, pal.grid);
        }
        save_png(&up, &scaled)?;
        written.push(scaled);
    }
    Ok(written)
}

/// Small legend image: one swatch + owner name per row.
pub fn legend_canvas(owners: &[String], scenario: &Scenario, pal: &Palette) -> Canvas {
    let row_h = font::text_height(1) + 4;
    let width = owners
        .iter()
        .map(|o| font::text_width(o, 1))
        .max()
        .unwrap_or(0)
        + 22;
    let mut c = Canvas::new(
        width.max(40) as u32,
        (owners.len() * row_h + 4) as u32,
        pal.land,
    );
    for (i, owner) in owners.iter().enumerate() {
        let y0 = (i * row_h + 2) as i32;
        let colour = scenario
            .owner_colour(owner)
            .unwrap_or_else(|| Palette::region_colour(i as i64));
        for dy in 0..(row_h as i32 - 2) {
            for dx in 0..12 {
                c.set(2 + dx, y0 + dy, colour);
            }
        }
        let w = c.width as i32;
        let h = c.height as i32;
        font::render(owner, 1, |px, py| {
            let (x, y) = (18 + px as i32, y0 + 1 + py as i32);
            if x < w && y < h {
                c.set(x, y, [40, 40, 40, 255]);
            }
        });
    }
    c
}

pub fn save_png(canvas: &Canvas, path: &Path) -> Result<()> {
    let img = image::RgbaImage::from_raw(canvas.width, canvas.height, canvas.to_rgba_bytes())
        .context("canvas buffer size mismatch")?;
    img.save_with_format(path, image::ImageFormat::Png)
        .with_context(|| format!("writing {}", path.display()))
}

/// `--list-regions`: print id, admin level, name and pixel count per region.
pub fn list_regions(g: &Generated, json: bool) -> String {
    let mut rows: Vec<_> = g.rendered.regions.iter().collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.pixels));
    if json {
        let items: Vec<serde_json::Value> = rows
            .iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.id,
                    "admin_level": r.admin_level,
                    "name": r.name,
                    "pixels": r.pixels,
                    "owner": g.scenario.owner_of(r.id, r.name.as_deref()),
                })
            })
            .collect();
        serde_json::to_string_pretty(&items).unwrap()
    } else {
        rows.iter()
            .map(|r| {
                format!(
                    "{}\t{}\t{}\t{}",
                    r.id,
                    r.admin_level,
                    r.name.as_deref().unwrap_or("-"),
                    r.pixels
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
