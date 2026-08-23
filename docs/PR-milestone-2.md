# PR: milestone 2 — labels, owner editing, owner borders, multi-select, wasm demo, polish

Implements `docs/milestone-2-prompt.md` (items 1, 2, 3, 4, 6, 7).

## What changed

**1. Labels** — `src/font.rs`: embedded public-domain 5×7 bitmap font
(printable ASCII) plus a transliteration table for European letters
(æøåäöüß, Latin Extended-A …). `src/labels.rs`: region names placed at the
raster pole of inaccessibility (multi-source BFS distance transform on the
region-id buffer), 2× for large regions with 1× fallback, greedy overlap
avoidance largest-first, ≤20 % water under a label. `place=city|town` nodes
(new `Geometry::Point`, fetched only when m/px allows) render as 2×2 dots +
names. Labels live on `layer::LABEL`, composed after post-fx. Flags:
`--labels`, `--label-color`, `--cities`.

**2. Owner editor** — right-side owner panel (swatch, name, region count;
click = brush, click again = clear); `N` starts typed owner creation
(Enter/Esc), `Delete` removes an empty owner. Click assigns the active owner;
`1-9`/`[`/`]` recolour the active owner, else the selected regions' explicit
colours. Undo/redo (`Ctrl+Z` / `Ctrl+Shift+Z`) snapshots the scenario, so
assignment, creation and recolours all revert. Panel doubles as the legend
(`L` toggles); `--legend` exports `out/<stem>.legend.png`.

**3. Owner borders** — `raster::derive_owner_borders` walks the region-id
buffer: different owners (or owned vs unowned) ⇒ 2 px country border on both
sides; same owner/both unowned ⇒ 1 px region border (`--inner-borders false`
hides). Never against ocean. The OSM boundary *lines* of the politically
filled level are dropped (back with `--osm-borders`); other levels keep
their lines. Overlays are recomposed live on every editor change.

**4. Multi-select & drill-down** — shift+click/drag extends the selection
(inverted 1 px outline), `Esc` clears, status shows count + pixels; edits
apply to the whole selection. `Z` fits the camera to the selection.
`D` unprojects the selection's pixel bbox (`Projection::unproject`) and
generates that bbox at admin level +2 as a new stack entry; `Backspace`
pops back with camera and editor state intact; the scenario is shared and
merged across the stack, `Ctrl+S` saves it all.

**6. Wasm demo** — `cargo build --target wasm32-unknown-unknown` and
`trunk build --release` succeed; CI gained a `wasm` job. On wasm the
pipeline renders a bundled fixture + example scenario (no threads: the job
runs inline; no fs: exports are disabled and Ctrl+S reports the error);
`?bbox=&width=&scale=` page query params are parsed via `web-sys`.

**7. Polish** — `--list-regions [--json]` (sorted by pixels, includes owner);
repeated `--scenario` merges in order, Ctrl+S writes the last; `pattern =
"hatch"|"dots"` + `pattern_color` per region; `--legend`; selection outline.

## Verification

| command | result |
|---|---|
| `cargo run -- --headless --scenario scenarios/example.toml --smooth 1 --legend` | 320×341 px, 5 135 features, 1 region — labels "Sjaelland", "København" dot+name, `out/map.legend.png` |
| `--tiles 2 --bbox 55.55,12.35,55.85,12.85 --width 480` | 480×511 px, 18 809 features, 3 regions, city/town labels |
| Denmark L2 (`--land`, 751 m/px) | "Danmark"/"Sverige" 2× labels, "Deutschland" 1× |
| `--list-regions` on fixture | two rows (id, level, name, pixels); `--json` parses |
| `trunk build --release` | ✅ success |
| viewer smoke test | generates, no panics |

Screenshots: `out/map@3x.png`, `out/oresund.png`, `out/denmark.png`.

**Tests**: 35 total (23 unit, 12 integration) — font glyphs/transliteration/
widths, polylabel-inside-U-shape, overlap refusal, owner model
assign/merge/resolve, derived country border presence, hatch rendering,
label pixels on fixture, list-regions text+JSON, and a second golden
(`small.owners.golden.png`). Both goldens were regenerated deliberately
(labels + derived borders changed the composed output by design).
`cargo clippy --all-targets`: clean.

## Design decisions / cut scope

- The fixture gained a **second** synthetic admin relation sharing an edge
  (both directions) so ring assembly and owner borders are testable offline.
- Undo = scenario snapshots (≤64) rather than per-op inverses — simpler and
  covers every edit type uniformly.
- Editor refreshes recompute all overlays + full texture upload; at ≤512 px
  maps this is a few ms, so per-region dirty rectangles were not needed.
- The in-viewer legend is the owner panel itself; the exported legend is a
  separate PNG (`--legend`) rather than burned into the map.
- Cut: live Overpass fetching on wasm (`ehttp`) and asset-server `--input` —
  the web demo always renders the bundled fixture; native is unchanged.
- Cut: label placement does not shift labels to dodge water, it only vetoes
  mostly-water placements.
