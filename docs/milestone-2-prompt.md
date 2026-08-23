# Prompt: milestone 2 for mapgenart — labels, owner editing, owner borders, multi-select, wasm demo, polish

Implement milestone 2 of `mapgenart` (Rust + Bevy 0.19, this repository) as a single PR. Read `README.md`, `docs/PR-next-milestone.md` and `src/` first; extend the existing architecture (`config.rs` → `osm.rs` → `raster.rs` → `postfx.rs` → `generate.rs` → `viewer.rs`, with `scenario.rs` / `palette.rs`) rather than rewriting. All six items below are in scope; finish every one, keep `cargo build`, `cargo test`, `cargo clippy --all-targets` clean, add tests for each item, and update the README and `docs/` PR description.

## 1. Labels

Add `src/font.rs` with an embedded 5×7 (or similar) bitmap pixel font covering printable ASCII plus the Latin-1/Latin Extended-A letters common in European names (æøåäöüßéèçñłńśźż…); unknown glyphs fall back to a transliteration table or `?`. Add `src/labels.rs`: place a label for every political region (name from the scenario `label`, else OSM `name`) at the polygon's pole of inaccessibility (iterative grid / Mapbox polylabel algorithm, computed on the region-id buffer, not on the geometry), sized 1× or 2× the font depending on region pixel area, skipped when the region is too small to fit; avoid overlaps greedily (largest regions first) and never place a label over water. Also draw `place=city|town` nodes as a dot plus name when metres-per-pixel allows (fetch them in the Overpass query only at those scales). Flags: `--labels/--no-labels` (default on), `--label-color`, `--cities`. Labels are drawn on a separate overlay layer (`layer::LABEL`) after post-fx so smoothing never touches them, and they are exported in the PNGs and shown in the viewer. Tests: glyph lookup, polylabel on a U-shape picks a point inside the shape, overlap avoidance.

## 2. Owner-level painting in the editor

Extend the viewer into an owner-based editor. Add an *owner palette* UI panel (Bevy UI, right side): lists every owner in the scenario with its colour swatch and region count; click an owner to make it the active brush, `N` (or a button) to create a new owner with a typed name (simple text input via keyboard events) and a preset colour; `Delete` removes an empty owner. With an active owner, clicking a region assigns it to that owner (`scenario.regions[id].owner = …`) and recolours it via the owner colour; `Shift`+click/drag assigns multiple regions. Keep `1–9`/`[`/`]` as "recolour the *owner*" when an owner is active, falling back to per-region colour otherwise. Add a legend overlay (owner swatches + names) toggled with `L`, included in the PNG export. Undo/redo must cover owner assignment, owner creation and recolours. Tests on the scenario model for assign/reassign/colour resolution.

## 3. Border hierarchy by ownership

In `raster.rs`, after political fills, compute a border mask from the region-id buffer: between two pixels whose regions have different owners draw the **country** border style (2 px, `palette.border_country`); between regions of the same owner draw the **region** style (1 px) only if `--inner-borders` (default off at admin level ≥ 4, on for none); no border where one side is ocean. Keep the OSM admin-boundary *lines* for levels that have no polygon (or when `--osm-borders` is set), otherwise replace them with the derived borders so the two never double up. Re-derive borders live in the editor when an assignment changes (only recompute the touched regions' bounding boxes). Golden-image test with a two-owner scenario on the fixture.

## 4. Multi-select and drill-down

`Shift`+click adds/removes regions from a selection; `Esc` clears; the status line shows count/total pixels; recolour/assign applies to the whole selection. `Z` (zoom-to-selection) fits the camera to the selected regions' bounding box. `D` (drill down) spawns a *new* generation job for the selection's geographic bbox at the current width with `--admin-level` incremented (e.g. 4 → 6) and opens it in the same window (stack of maps: `Backspace` returns to the parent map, preserving its editor state). Scenario edits made in a drilled-down map are merged into the same in-memory scenario and saved together.

## 6. Wasm build / web demo

Make `cargo build --target wasm32-unknown-unknown` and the existing Trunk workflow (`index.html`, `Trunk.toml`, `.github/workflows/deploy-page.yaml`) work: gate `ureq`/threads behind `cfg(not(target_arch = "wasm32"))`, run generation via `bevy::tasks::AsyncComputeTaskPool` (or `IoTaskPool`) instead of `std::thread` on all targets, and fetch Overpass with `ehttp` or `gloo-net` on wasm (optional — at minimum the web build must load a bundled fixture). Bundle `tests/fixtures/small.json` plus `scenarios/example.toml` as assets for the demo so the page opens straight into the editor; allow `?bbox=&width=` query params on web. Add `--input` loading through Bevy's asset server on wasm. Keep the native path unchanged. CI: add a `wasm` job to `ci.yml` that builds the target.

## 7. Polish

- `--list-regions`: headless flag that fetches/parses and prints `id<TAB>admin_level<TAB>name<TAB>pixels` for every political region (sorted by pixels desc) so scenario files are easy to write; `--list-regions --json` for machine use.
- Scenario layering: `--scenario` may be given multiple times; files are merged in order (later wins per key); `Ctrl+S` writes only the last one.
- Hatching: `Assignment.pattern = "hatch" | "dots"` draws a 1-px diagonal hatch / dot pattern in a second colour (`pattern_color`) over the region fill — for disputed/occupied territory. Applied in raster (below labels), respected by recolour.
- `--legend` writes `out/<stem>.legend.png` (owners + swatches) next to the map and `--grid` + legend compose into `@Nx` export correctly.
- Viewer: show a small crosshair/outline around the selected region(s) (1-px outline in inverted colour, overlay layer) so selection is visible.

## Constraints

No heavy new dependencies (`ehttp`/`gloo-net` for wasm only; nothing else beyond what is already in `Cargo.toml`). Do not regress the existing golden test without regenerating it deliberately (`UPDATE_GOLDEN=1`) and stating why. Verify end-to-end: `cargo run -- --headless --scenario scenarios/example.toml --labels` on the default bbox, `--tiles 2` on `55.55,12.35,55.85,12.85`, `--list-regions` output, and `trunk build --release` for the web target; paste dimensions/feature/region counts and a screenshot path into `docs/PR-milestone-2.md`. Summarise design decisions and any cut scope in the final message.

## Optional staging

1 alone · 2+3+4 together · 6 alone · 7 last.
