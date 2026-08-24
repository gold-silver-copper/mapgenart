use anyhow::{Context, Result, bail};
use bevy::prelude::Resource;
use clap::Parser;
use std::path::PathBuf;

/// Geographic bounding box in WGS84 degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BBox {
    pub south: f64,
    pub west: f64,
    pub north: f64,
    pub east: f64,
}

impl BBox {
    /// Parse `south,west,north,east` (Overpass order).
    pub fn parse(s: &str) -> Result<Self> {
        let parts: Vec<f64> = s
            .split(',')
            .map(|p| p.trim().parse::<f64>())
            .collect::<Result<_, _>>()
            .with_context(|| format!("invalid bbox `{s}`, expected south,west,north,east"))?;
        if parts.len() != 4 {
            bail!("bbox needs exactly 4 numbers: south,west,north,east");
        }
        let b = BBox {
            south: parts[0],
            west: parts[1],
            north: parts[2],
            east: parts[3],
        };
        if b.south >= b.north || b.west >= b.east {
            bail!("bbox must satisfy south < north and west < east");
        }
        Ok(b)
    }

    pub fn overpass(&self) -> String {
        format!("{},{},{},{}", self.south, self.west, self.north, self.east)
    }
}

/// Command line / runtime configuration for the map generator.
#[derive(Parser, Debug, Clone, Resource)]
#[command(
    name = "mapgenart",
    about = "Pixel-art Q-BAM style maps from OpenStreetMap data"
)]
pub struct MapConfig {
    /// Bounding box: south,west,north,east (degrees). Default: central Copenhagen.
    #[arg(long, default_value = "55.655,12.540,55.715,12.640")]
    pub bbox: String,

    /// Width of the generated map in pixels (height follows from the Mercator aspect ratio).
    #[arg(long, default_value_t = 320)]
    pub width: u32,

    /// Integer upscale factor for the on-screen view and the `@Nx` export.
    #[arg(long, default_value_t = 3)]
    pub scale: u32,

    /// Load a previously fetched Overpass JSON file instead of querying the network.
    #[arg(long)]
    pub input: Option<PathBuf>,

    /// Where to write the PNG export (a second `@Nx` upscaled file is written next to it).
    #[arg(long, default_value = "out/map.png")]
    pub output: PathBuf,

    /// Overpass API endpoint.
    #[arg(long, default_value = "https://overpass-api.de/api/interpreter")]
    pub overpass: String,

    /// Directory where raw Overpass responses are cached (keyed by bbox + query).
    #[arg(long, default_value = ".cache")]
    pub cache_dir: PathBuf,

    /// Ignore cached Overpass responses and re-download.
    #[arg(long)]
    pub no_cache: bool,

    /// Also fetch and draw building footprints (slow / large for dense cities).
    #[arg(long)]
    pub buildings: bool,

    /// Skip road and railway lines for a cleaner Q-BAM look.
    #[arg(long)]
    pub no_roads: bool,

    /// Generate + export the PNG and exit without opening a window.
    #[arg(long)]
    pub headless: bool,

    // --- political fills -------------------------------------------------
    /// Scenario file(s) (TOML) assigning owners/colours to OSM admin
    /// relations. May be given multiple times; later files win per key and
    /// Ctrl+S in the editor writes only the last one.
    #[arg(long)]
    pub scenario: Vec<PathBuf>,

    /// Admin level used for political fills (2 = countries, 4 = states/regions, ...).
    /// Falls back to level 2 when no relation of the requested level is present.
    #[arg(long, default_value_t = 4)]
    pub admin_level: u8,

    /// Disable political (admin region) fills entirely.
    #[arg(long)]
    pub no_political: bool,

    // --- scale ----------------------------------------------------------
    /// Split the bbox into an N×N grid of Overpass queries (for large areas).
    #[arg(long, default_value_t = 1)]
    pub tiles: u32,

    /// GeoJSON land polygons (Natural Earth / osmdata.openstreetmap.de) used as
    /// the land/ocean base instead of in-bbox coastline detection.
    #[arg(long)]
    pub land: Option<PathBuf>,

    // --- post-processing ---------------------------------------------------
    /// Iterations of a 3×3 majority-vote smoothing filter (0 = off).
    #[arg(long, default_value_t = 0)]
    pub smooth: u32,

    /// Remove same-colour blobs smaller than this many pixels (0 = off).
    #[arg(long, default_value_t = 0)]
    pub min_feature: u32,

    /// Draw a 1px darker outline where land meets water (on by default; `--shoreline false` to disable).
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub shoreline: bool,

    /// Quantize the image to its N most frequent palette colours (0 = off).
    #[arg(long, default_value_t = 0)]
    pub quantize: u32,

    // --- quality of life --------------------------------------------------
    /// TOML file overriding palette colours (see `palettes/qbam.toml`).
    #[arg(long)]
    pub palette: Option<PathBuf>,

    /// Draw 1px grid lines between source pixels in the `@Nx` export.
    #[arg(long)]
    pub grid: bool,

    // --- labels & borders ---------------------------------------------------
    /// Draw region name labels (`--labels false` to disable).
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub labels: bool,

    /// Label colour (hex).
    #[arg(long, default_value = "#3a3a3a")]
    pub label_color: String,

    /// Draw city/town dots and names (scale permitting).
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub cities: bool,

    /// Draw thin borders between regions of the same owner (`--inner-borders false` to hide).
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub inner_borders: bool,

    /// Keep the raw OSM admin-boundary lines for the politically filled level
    /// instead of the derived owner borders.
    #[arg(long)]
    pub osm_borders: bool,

    /// Also write `out/<stem>.legend.png` with owner swatches.
    #[arg(long)]
    pub legend: bool,

    /// Print every political region (id, level, name, pixels) and exit.
    #[arg(long)]
    pub list_regions: bool,

    /// With --list-regions: emit JSON.
    #[arg(long)]
    pub json: bool,

    // --- game -------------------------------------------------------------
    /// Open the map editor instead of the game.
    #[arg(long)]
    pub edit: bool,

    /// Run the game simulation headless for N fixed ticks, print a summary
    /// and exit (CI smoke test).
    #[arg(long)]
    pub sim_ticks: Option<u32>,

    /// Starting squad size.
    #[arg(long, default_value_t = 8)]
    pub squad: u32,

    /// Render buildings as walls + floors and carve procedural doors/windows
    /// (the game turns this on by default).
    #[arg(long)]
    pub enterable: bool,

    /// Override the sleeping population size (default: scaled to map area).
    #[arg(long)]
    pub population: Option<u32>,
}

impl MapConfig {
    pub fn bbox(&self) -> Result<BBox> {
        BBox::parse(&self.bbox)
    }
}

impl BBox {
    /// Split into an `n × n` grid of sub-boxes, row-major from the south-west.
    pub fn tiles(&self, n: u32) -> Vec<BBox> {
        let n = n.max(1);
        let dlat = (self.north - self.south) / n as f64;
        let dlon = (self.east - self.west) / n as f64;
        let mut out = Vec::with_capacity((n * n) as usize);
        for r in 0..n {
            for c in 0..n {
                out.push(BBox {
                    south: self.south + dlat * r as f64,
                    west: self.west + dlon * c as f64,
                    north: if r + 1 == n {
                        self.north
                    } else {
                        self.south + dlat * (r + 1) as f64
                    },
                    east: if c + 1 == n {
                        self.east
                    } else {
                        self.west + dlon * (c + 1) as f64
                    },
                });
            }
        }
        out
    }

    /// Approximate metres per pixel for a canvas of `width` px spanning this box.
    pub fn metres_per_pixel(&self, width: u32) -> f64 {
        let mid_lat = ((self.north + self.south) / 2.0).to_radians();
        let metres = (self.east - self.west).to_radians() * 6_371_000.0 * mid_lat.cos();
        metres / width.max(1) as f64
    }
}
