# PR: next milestone — political fills, scale, post-fx, editor, QoL, tests

Implements everything in `docs/next-milestone-prompt.md`.

## What changed

**1. Political fills + scenario file** — `boundary=administrative` relations are
assembled into polygons (`Kind::Region(level)`; broken chains are force-closed
with a warning). Regions are filled *below* land cover, borders on top.
`--scenario file.toml` (`src/scenario.rs`) maps relation ids or names to
`{owner, color, label}`; owner colours live in `[owners]`; unassigned regions
get a deterministic hash pastel. `--admin-level N` (default 4, falls back to 2),
`--no-political`. Example: `scenarios/example.toml`.

**2. Continent scale** — `--tiles N` splits the bbox into N×N Overpass queries
(cached, retry/backoff on 429/502/503/504, elements de-duplicated by id).
`--land file.geojson` uses Natural Earth / osmdata land polygons as the
land/ocean base; otherwise the coastline vote fill is used. `Detail::for_scale`
drops streams/minor roads/rail/buildings/local borders as m/px grows and the
Overpass query shrinks accordingly.

**3. Post-fx** (`src/postfx.rs`) — `--smooth K`, `--min-feature N`,
`--shoreline` (default on), `--quantize N`. Canvas now carries a per-pixel
layer tag so line layers are never smoothed.

**4. Editor** (`src/viewer.rs`) — per-pixel region-id buffer; click selects,
status shows name/id/level/owner; `1–9` presets, `[`/`]` hue rotate,
`Ctrl+S` writes the scenario, `Ctrl+Z` undo. Recolouring touches only the
region's base pixels and re-uploads the texture — no regeneration. Progress
messages ("Fetching tile i/N …") stream from the worker thread.

**5. QoL** — `Palette` struct with TOML override (`--palette`,
`palettes/qbam.toml`), `--grid`, Overpass byte/elapsed logging, removed
`mobile/` + iOS/Android workflows + Windows installer (web + desktop release
flow kept, names updated).

**6. Tests** — `tests/fixtures/small.json` (204 KB crop of the Copenhagen
response + a synthetic 3-way admin relation), `tests/pipeline.rs`: full
pipeline, ocean/land presence, closed rings, political fill + scenario colour,
tiling math, golden image (`small.golden.png`, 0.5 % tolerance,
`UPDATE_GOLDEN=1`). Unit tests for postfx, palette, scenario, land GeoJSON,
ring assembly, raster. 23 tests total; `cargo clippy --all-targets` is clean.

## Verification

| command | result |
|---|---|
| `cargo run -- --headless` (default Copenhagen) | 320×341 px (20 m/px), 5 134 features, 1 region (L4) |
| `--tiles 2 --bbox 55.55,12.35,55.85,12.85 --width 480 --smooth 1` | 480×511 px (65 m/px), 18 796 features, 3 regions (L4) |
| `--bbox 54.4,7.8,58.0,14.6 --width 560 --tiles 2 --land ne_50m_land.geojson --admin-level 2` | 560×533 px (751 m/px), 27 763 features, 5 regions (L2) |
| viewer (`./target/debug/mapgenart --scenario scenarios/example.toml`) | generates, exports, no panics; 504 retry path exercised live |

## Design decisions

- Region fills paint only non-ocean pixels, and land cover never paints over
  ocean (nature reserves / ports extend into water). Borders skip ocean pixels
  (no maritime boundaries — Q-BAM convention).
- Ring assembly matches endpoints exactly (same OSM node ⇒ same coords).
- Quantize keeps palette colours first, then most frequent image colours.
- New dependency: `toml` only; GeoJSON is hand-parsed with `serde_json`.

## Scope notes

- Borders + region fills at admin levels 5–8 are only fetched when
  m/px < 150 (query size). Level 7 (e.g. Danish municipalities) is not queried.
- Shoreline outline is applied after smoothing; recolouring a region after
  `--smooth` only touches pixels still tagged as region base.
- No git history existed in the repo, so nothing was committed.
