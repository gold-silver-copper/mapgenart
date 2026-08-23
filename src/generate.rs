//! End-to-end pipeline: config → Overpass (tiles) → features → canvas → post-fx → PNG.

use crate::config::MapConfig;
use crate::osm::{self, Progress};
use crate::palette::Palette;
use crate::postfx::{self, PostFx};
use crate::raster::{self, Canvas, Detail, RenderOptions, Rendered};
use crate::scenario::Scenario;
use crate::{land, osm::Feature};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub struct Generated {
    pub rendered: Rendered,
    pub feature_count: usize,
    pub palette: Palette,
    pub scenario: Scenario,
    pub metres_per_pixel: f64,
}

impl Generated {
    pub fn canvas(&self) -> &Canvas {
        &self.rendered.canvas
    }
}

/// Load palette + scenario named in the config (defaults when absent).
pub fn load_style(cfg: &MapConfig) -> Result<(Palette, Scenario)> {
    let palette = match &cfg.palette {
        Some(p) => Palette::load(p)?,
        None => Palette::default(),
    };
    let scenario = match &cfg.scenario {
        Some(p) if p.exists() => Scenario::load(p)?,
        Some(p) => {
            log::info!(
                "scenario {} does not exist yet; starting empty",
                p.display()
            );
            Scenario::default()
        }
        None => Scenario::default(),
    };
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
    let generated = render_features(cfg, &features, &palette, scenario, land.as_ref())?;
    Ok(generated)
}

/// Render already-parsed features (shared by the CLI pipeline and tests).
pub fn render_features(
    cfg: &MapConfig,
    features: &[Feature],
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
    };
    let mut rendered = raster::render(features, bbox, width, &opts);
    let fx = PostFx {
        smooth: cfg.smooth,
        min_feature: cfg.min_feature,
        shoreline: cfg.shoreline,
        quantize: cfg.quantize,
    };
    postfx::apply(&mut rendered.canvas, &fx, palette);
    Ok(Generated {
        rendered,
        feature_count: features.len(),
        palette: palette.clone(),
        scenario,
        metres_per_pixel: mpp,
    })
}

/// Write `path` (1×) and `path@{scale}x` (nearest-neighbour upscale, optional grid).
pub fn export(
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

pub fn save_png(canvas: &Canvas, path: &Path) -> Result<()> {
    let img = image::RgbaImage::from_raw(canvas.width, canvas.height, canvas.to_rgba_bytes())
        .context("canvas buffer size mismatch")?;
    img.save_with_format(path, image::ImageFormat::Png)
        .with_context(|| format!("writing {}", path.display()))
}
