# mapgenart — Last Light

A real-time tactics game played on **pixel-art maps generated from real
OpenStreetMap data** — StarCraft-style squad control against growing hordes,
set in post-apocalyptic San Francisco (or any place on earth). No base
building: just your soldiers, the streets, and the waves. Rust + Bevy 0.19 +
Avian physics.

The original map generator/editor lives on behind `--edit`.

## Play

```sh
cargo run --features dev            # post-apocalyptic downtown SF (bundled map)
cargo run -- --bbox 48.85,2.33,48.87,2.36 --width 900   # play Paris instead
cargo run -- --sim-ticks 1500 --input tests/fixtures/small.json \
    --bbox 55.674,12.588,55.686,12.602 --width 160       # headless CI smoke run
cargo run -- --edit                 # the map editor (milestone 2)
```

The first custom-bbox run fetches OSM from Overpass and caches it in
`.cache/`; the bundled SF map (`assets/maps/sf.json`) loads instantly.

### Controls

| input | action |
|---|---|
| left click / drag | select soldier / box-select |
| shift | add to selection |
| right click | move (formation) |
| `A` + left click | attack-move |
| `S` / `H` / `P` + click | stop / hold position / patrol |
| Ctrl+`1`–`9` / `1`–`9` | assign / recall control group |
| WASD-arrows / edge / minimap click | pan camera · wheel: zoom |
| `F12` | screenshot to `out/screenshot-NNN.png` |
| Enter | start (menu) · `R` retry (game over) |

### The game

- Your squad (riflemen, gunners, a medic) spawns mid-map. Waves of infected
  spawn at the map edges and hunt you down a Dijkstra **flow field**; each
  wave is bigger, faster, tougher. Survive between waves to receive **supply
  drops** at real POIs (hospitals, supermarkets): medkits, ammo (damage
  buff), recruits.
- **Buildings are solid** — real footprints become Avian static colliders
  (greedy rectangle meshing of the raster), soldiers path around them with
  A* + string pulling, and hordes flow through the street canyons.
- **Fog of war & line of sight** — unexplored is black, explored is dimmed,
  and buildings block sight (per-pixel raycasts): a horde behind a building
  is invisible until it rounds the corner, on the map and the minimap alike.
  Soldiers only fire at what they can see.

### Debug/CI hooks

`--sim-ticks N` runs the deterministic headless simulation (16 ms virtual
ticks, no window) and prints wave/kill/stuck-unit stats.
`MAPGEN_AUTOSTART=1` skips the menu; `MAPGEN_AUTOSHOT=<secs>` takes an
automatic screenshot; `MAPGEN_NOFOG=1` disables the fog overlay.

## The map generator / editor (`--edit`)

Everything from earlier milestones is intact: Q-BAM-style political maps
from OSM admin boundaries, scenario files (owners/colours/patterns), the
owner-painting editor with drill-down, labels with an embedded pixel font,
post-fx, palettes (`palettes/qbam.toml`, `palettes/postapoc.toml`),
`--headless` PNG export, `--list-regions`, tiled Overpass fetching, Natural
Earth land fallback, and the wasm/trunk web demo. See `docs/PR-next-milestone.md`
and `docs/PR-milestone-2.md`.

## Layout

- `src/game/nav.rs` – walkability grid, A* + string pulling, flow field, greedy rect meshing
- `src/game/fog.rs` – fog states + LOS raycasts
- `src/game/world.rs` – map → colliders / nav / fog / POIs
- `src/game/units.rs` – classes, stats, generated pixel sprites
- `src/game/logic.rs` – head-independent simulation (movement, AI, combat, waves, drops)
- `src/game/control.rs` – selection, orders, groups, camera
- `src/game/view.rs` – fog & minimap textures, gizmos, HUD, menus, screenshots
- `src/game/mod.rs` – states, loading, headless sim + stress harness
- generator/editor: `src/{config,osm,land,raster,postfx,palette,scenario,labels,font,generate,viewer}.rs`
- `tests/pipeline.rs` (generator), `tests/game.rs` (nav/LOS/colliders/waves/sim/stress)

Data: © OpenStreetMap contributors (ODbL), fetched via Overpass API.
Scaffold based on [bevy_game_template](https://github.com/NiklasEi/bevy_game_template).
