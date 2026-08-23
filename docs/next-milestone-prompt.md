# Prompt: next milestone for mapgenart (single PR)

Implement the next milestone of `mapgenart` (Rust + Bevy 0.19, in this repository) as a single PR. Read `README.md` and `src/` first; keep the existing architecture (`config.rs` → `osm.rs` → `raster.rs` → `generate.rs` → `viewer.rs`) and extend it rather than rewriting. All six features below are in scope; finish every one, keep `cargo build`, `cargo test`, and `cargo clippy` clean, and update the README.

## 1. Political fills + scenario file

Assemble `boundary=administrative` relations into polygons (reuse `assemble_rings`; handle multiple outers, inners, and chains broken by missing ways — force-close and log a warning). Add a `Kind::Region { admin_level }` feature. Fill regions with a flat colour *below* land-cover fills, then draw borders on top with the existing line styles. Introduce a scenario file (`--scenario path.toml`, TOML via `serde`): maps OSM relation IDs (and optionally names) to `{ owner, color, label }`; unassigned regions get a deterministic colour from a hash of their ID. Write an example `scenarios/example.toml`. Make `--admin-level N` select which level is used for political fills (default 4 country/region; country-level 2 falls back when 4 has no coverage).

## 2. Continent-scale support

Add `--tiles N` to split the bbox into an N×N grid of Overpass queries, fetched sequentially with caching and retry/backoff on HTTP 429/504, stitched into one feature set (dedupe elements by ID). Add a land/water fallback for when no coastline is in a tile: accept `--land <geojson>` (Natural Earth or osmdata.openstreetmap.de land polygons, GeoJSON parsed with `serde_json`) and use it for the land/ocean base when present, otherwise keep the existing coastline vote fill. Automatically reduce detail at wide scales (skip streams/minor roads/buildings when metres-per-pixel exceeds thresholds).

## 3. Pixel-art post-processing

New `src/postfx.rs`, applied after rendering and configurable via flags: `--smooth K` (K iterations of 3×3 majority-vote filter, ignoring line layers), `--min-feature N` (remove connected blobs smaller than N px, filling them with the surrounding majority colour), `--shoreline` (1px darker outline where land meets water, default on), and `--quantize N` (snap to the nearest of the N palette colours).

## 4. Interactive editor in the Bevy viewer

Keep a per-pixel region-ID buffer alongside the canvas. Left-click selects the region under the cursor (account for camera pan/zoom); show its name/ID/owner in the status text; keys `1–9`/`[`/`]` cycle its colour; `Ctrl+S` writes the current assignments back to the scenario file; `Ctrl+Z` undoes the last edit. Re-render only the affected region (recolour pixels by region ID) — no full regeneration.

## 5. Quality of life

`--palette file.toml` to override colours in `palette.rs` (turn the constants into a `Palette` struct with a `Default` holding the current values). `--grid` draws 1px grid lines in the `@Nx` export. Overpass progress: log bytes received and elapsed time; show "Fetching tile i/N…" in the viewer status. Remove `mobile/` and the iOS/Android/Windows-installer CI workflows; keep the web + desktop release workflow.

## 6. Tests

Check in a small fixture (`tests/fixtures/small.json`, ≤300 KB — crop the cached Copenhagen response to a sub-bbox) and add integration tests: full pipeline renders without panic, coastline vote fill produces both land and ocean, region assembly yields closed rings, and a golden-image test comparing `tests/fixtures/small.golden.png` with a pixel-mismatch tolerance (regenerate goldens via `UPDATE_GOLDEN=1`). Unit-test `postfx` filters on tiny canvases.

## Constraints

No new heavy dependencies beyond `toml` (and `geojson` only if it is truly lighter than hand-parsing with `serde_json`). Don't break `--headless`. Verify end-to-end with `cargo run -- --headless` on the default bbox and on a `--tiles 2` bbox, and include the generated PNGs' dimensions/feature counts in the PR description. Summarise design decisions and any scope you had to cut in the final message.

## Optional staging

If reviewing in stages is preferred, split at the numbered boundaries: 1+4 together, 2 alone, 3+5+6 together.
