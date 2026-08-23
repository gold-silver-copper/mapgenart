# mapgenart

Pixel-art, Q-BAM-flavoured alt-history maps generated from real
OpenStreetMap data, written in Rust with [Bevy](https://bevyengine.org).

Pipeline: `bbox` → Overpass API (optionally tiled) → tagged OSM geometry →
software rasterizer (Mercator, even-odd polygon fill, Bresenham lines,
coastline-vote ocean detection or external land polygons, political fills from
admin relations) → pixel-art post-fx → nearest-neighbour sprite in a Bevy
window with a region editor + PNG export.

## Run

```sh
# default: central Copenhagen, 320 px wide, shown at 3×
cargo run --features dev

# political fills coloured by a scenario file, smoothing, no roads
cargo run --features dev -- --scenario scenarios/example.toml --smooth 1 --no-roads

# no window – just write out/map.png and out/map@3x.png
cargo run -- --headless --bbox 45.40,12.28,45.46,12.38

# continent scale: 2×2 Overpass tiles, Natural Earth land polygons, country fills
cargo run -- --headless --bbox 54.4,7.8,58.0,14.6 --width 560 --tiles 2 \
    --land data/ne_50m_land.geojson --admin-level 2 --smooth 1
```

### Flags

| flag | meaning |
|------|---------|
| `--bbox S,W,N,E` | area in degrees (Overpass order) |
| `--width N` | map width in px (height follows Mercator aspect) |
| `--scale N` | on-screen zoom and `@Nx` export factor |
| `--tiles N` | split the bbox into N×N Overpass queries (retry/backoff on 429/504) |
| `--land file.geojson` | land polygons (Natural Earth / osmdata.openstreetmap.de) as land/ocean base |
| `--scenario file.toml` | owners / colours / labels for admin regions (see `scenarios/example.toml`) |
| `--admin-level N` | admin level for political fills (default 4, falls back to 2) |
| `--no-political` | disable political fills |
| `--smooth K` | K passes of 3×3 majority-vote smoothing (lines untouched) |
| `--min-feature N` | remove same-colour blobs under N px |
| `--shoreline false` | disable the 1px shoreline outline (on by default) |
| `--quantize N` | snap to the N most frequent palette colours |
| `--palette file.toml` | override colours (see `palettes/qbam.toml`) |
| `--grid` | 1px grid in the `@Nx` export |
| `--buildings`, `--no-roads` | include footprints / drop roads+rail |
| `--input file.json` | render a saved Overpass response (the only option on wasm) |
| `--headless` | generate + export and exit |

Fine detail (streams, minor roads, buildings, local borders) is dropped
automatically as metres-per-pixel grows; the Overpass query shrinks with it.
Overpass responses are cached in `.cache/` (`--no-cache` to refetch).

### Viewer / editor

drag: pan · wheel: zoom · **click** a region to select it · `1`–`9` preset
colours · `[` / `]` rotate hue · `Ctrl+S` write assignments back to the
scenario file (`scenarios/edited.toml` if none given) · `Ctrl+Z` undo ·
`E` export · `R` refetch · `0` reset view.

## Layout

- `src/config.rs` – CLI flags (`clap`) / `MapConfig`, bbox tiling, metres-per-pixel
- `src/osm.rs` – scale-aware Overpass query, tiles + cache + retry, parsing, multipolygon/ring assembly, classification
- `src/land.rs` – GeoJSON land polygons
- `src/raster.rs` – projection, canvas with per-pixel layer tags, ocean vote fill, political fills + region-id buffer, draw order
- `src/postfx.rs` – smoothing, min-feature, shoreline, quantize
- `src/palette.rs` – `Palette` (TOML-overridable), region hash colours, presets
- `src/scenario.rs` – scenario TOML load/save/resolve
- `src/generate.rs` – pipeline + PNG export
- `src/viewer.rs` – Bevy plugin (background generation, pan/zoom, editor)
- `tests/pipeline.rs` – integration + golden-image tests on `tests/fixtures/small.json` (`UPDATE_GOLDEN=1` to regenerate)

Project scaffold based on [bevy_game_template](https://github.com/NiklasEi/bevy_game_template)
(web + desktop release workflow kept).
