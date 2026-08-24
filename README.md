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
| shift + click/drag | add to selection |
| ctrl + click | select every soldier of that class on screen |
| `F2` | select the whole squad |
| right click | move (formation) · on a visible enemy: attack-move |
| `A` + left click | attack-move |
| `S` / `H` / `P` + click | stop / hold position / patrol |
| `B` + click a door/window | board it up (click a boarded one to tear down) |
| Ctrl+`1`–`9` / `1`–`9` | assign / recall control group (double-tap: centre camera) |
| middle-mouse drag | pan the map |
| arrows / `W` `D` / screen edge / minimap click | pan camera |
| wheel | zoom to the cursor |
| `Esc` | cancel pending command / **pause menu with all controls** |
| `F12` | screenshot to `out/screenshot-NNN.png` |
| Enter | start (menu) · `R` retry (game over) |

### The game

- **The city sleeps.** A fixed population of infected (~1000 in downtown SF)
  lies dormant in buildings and streets. Gunfire wakes everything in
  earshot; the woken shriek and wake more; sightings become shared alerts
  the horde converges on down a Dijkstra **flow field**. Stay quiet, or
  start a snowball you can't stop. Awake enemies that lose the trail calm
  down by day — at night nothing calms and sleepers stir on their own.
- **Reach the evac.** Every run picks real, named objectives from the map:
  search a mid-point cache ("Sutter Fine Foods"), then reach the extraction
  and hold it 60 s through the alarm it raises. Extract = victory screen
  with the run's story; lose the squad = the light goes out.
- **The guns are hungry.** Shots drain a squad ammo pool (empty = bayonets),
  the medic burns meds, barricades and recruits cost scrap. Loot building
  interiors (progress by standing inside; pharmacies and markets are rich,
  sleepers lurk indoors) to stay supplied.
- **Named soldiers.** Reyes "Ghost", 23 kills, ranks up: harder-hitting and
  quieter. Death is permanent for the run; the fallen are listed at the end.
- **Barricades.** `B` + click boards up a carved door or window (scrap,
  hammering noise, enemies smash through given time) — turn any real
  building into a last stand.
- **Buildings are solid — and enterable.** Real footprints become walls with
  interior floors; procedural **doors** (≥1 per building where the street
  allows) and **windows** are carved deterministically. Walls are Avian
  static colliders; doors let units through; windows pass *sight and bullets*
  but not bodies — hole up inside and shoot out, but watch the doorways.
- **Hordes aren't psychic.** Enemies wander until they see a soldier
  (line-of-sight, windows included) or hear gunfire; sightings become shared
  alerts the horde converges on via the flow field, then decay. Waves spawn
  with only a rough "scent" of the squad.
- **Fog of war & line of sight** — the full map is always rendered; fog only
  darkens it (deepest where never scouted, lighter once explored) and hides
  what moves in it. Buildings block sight (per-pixel raycasts): a horde behind a building
  is invisible until it rounds the corner, on the map and the minimap alike.
  Soldiers only fire at what they can see.

UI text everywhere (menus, HUD, editor) uses an embedded **Iosevka** subset
(SIL OFL, `assets/fonts/`) — no tofu on any platform, wasm included.

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
