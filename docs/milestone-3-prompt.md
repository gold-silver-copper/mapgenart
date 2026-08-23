# Prompt: milestone 3 — "mapgenart: Last Light SF", an OSM-map RTS

Turn `mapgenart` (Rust + Bevy 0.19, this repository) into a playable real-time
tactics game as a single PR: StarCraft-style squad control — but **no base
building** — on top-down pixel-art maps generated from real OSM data, set in
post-apocalyptic San Francisco. The player commands a small band of soldiers
against growing hordes of enemies. Use **Avian physics (`avian2d`)** for
collision: soldiers and enemies must never pass through buildings or water.

Read `README.md`, `docs/PR-milestone-2.md` and `src/` first. Keep the existing
generator/editor as a library and a `--edit` mode; the game becomes the
default binary experience. Restructure into a cargo workspace or feature-gated
modules (`game/` vs `editor/`) — your call, justify it in the PR. Keep
`cargo build`, `cargo test`, `cargo clippy --all-targets` clean; native +
wasm must both build (wasm plays on the bundled fixture map).

## 1. Map → game world

- Default map: downtown San Francisco (pick a bbox around ~37.76,-122.52 →
  37.81,-122.38; make `--bbox`/`--width` still work so any real place is a map).
- Render the map with the existing pipeline at a fixed metres-per-pixel that
  makes a soldier sprite ≈ street width (target ~1.5–3 m/px, world scale
  documented). Post-apocalyptic palette preset (`palettes/postapoc.toml`):
  ash-grey land, rusted buildings, murky water, dead-grass parks.
- From the rasterized layers build a **collision world**: static Avian
  colliders for every building footprint (use the polygon geometry, convex
  decomposition or per-pixel merged rects — measure and pick) and for
  water/ocean. Roads/parks/sand are walkable. Cache the derived collision +
  walkability grid per map.
- A navigation grid (walkable = not building/water) drives A* pathfinding
  with path smoothing; physics resolves the residual pushing. Units must
  route around buildings, never tunnel through them (test this).

## 2. Units & control (the StarCraft feel, minus the base)

- Player squad: start with ~8 soldiers (rifleman, gunner, medic at minimum —
  distinct stats/sprites). Top-down pixel sprites (embed tiny hand-drawn
  8×8/16×16 sheets or generate programmatically; nearest-neighbour, 4/8-way
  facing, walk + fire + death frames).
- Controls: left-click select, drag box-select, shift add-to-selection,
  control groups (Ctrl+1–9 assign, 1–9 recall), right-click move /
  attack-move (A + click), stop (S), hold (H), patrol (P). Move orders use
  formation offsets so the squad doesn't stack on one pixel.
- Combat: hitscan or slow projectile rifles with range, cooldown, damage,
  friendly-fire off; enemies melee/short-range. Health bars, damage
  flashes, corpses. Medic heals nearby.
- Camera: edge-pan + WASD/arrows + minimap click; zoom on wheel; the whole
  map visible on a **minimap** with unit blips and fog.

## 3. Enemies & game loop

- Hordes spawn at map edges / from spawner buildings in waves that scale
  over time (count, speed, hp); they path toward the players' units or
  points of interest. Support hundreds of active enemies — use spatial
  partitioning and keep per-frame work bounded (measure; target 60 fps
  with 300 enemies on a 2019 laptop).
- Loop: survive escalating waves; between waves pick up supply drops
  (ammo/medkits/new recruit) that appear at real POIs (OSM `amenity=hospital`,
  `shop=supermarket`, etc. — parse a few POI types into pickups). Defeat =
  squad wiped; score = waves survived + kills. Menu → play → game-over →
  restart flow with the map choice.

## 4. Fog of war & line of sight

- Two-layer fog: unexplored (black) and explored-but-unseen (dimmed,
  buildings remembered). Vision circles per soldier; **buildings block
  sight** — compute LOS against building edges (shadowcasting on the
  walkability/building grid or raycasts via Avian) so streets behind a
  building stay hidden. Enemies are only rendered/audible inside current
  vision; the minimap respects fog.
- Fog updates incrementally (dirty regions), rendered as an overlay texture
  in the same pixel grid as the map.

## 5. Physics (Avian)

- Add `avian2d` (latest release compatible with Bevy 0.19); fixed timestep.
  Units = dynamic (or kinematic) circle colliders with damping, buildings/
  water = static colliders, collision layers: units collide with world and
  each other (soft separation), projectiles with world + enemies.
- No unit may end up inside a static collider (spawn-point validation +
  depenetration). Add a stress test spawning 200 units pushing through a
  street canyon — no tunneling, no NaNs.

## 6. Quality bar

- Tests: navgrid from the fixture map (a path around a known building, not
  through it), LOS blocked by a building between two points, fog
  explored/visible state transitions, wave scaling math, collider generation
  from fixture polygons (counts + a point-in-building assert), plus existing
  suite still green. Headless `--sim-ticks N` mode that runs the game loop
  without a window for CI smoke tests.
- Performance note in the PR (entity counts, frame times, what you measured).
- README rewrite: play instructions, controls table, how to play any city on
  earth (`cargo run -- --bbox …`), editor still reachable via `--edit`.
- `docs/PR-milestone-3.md` with design decisions, screenshots
  (`out/screenshot-*.png` via a screenshot key F12), and cut scope.

## Constraints

New dependencies allowed: `avian2d`, a pathfinding crate (or hand-rolled A*),
nothing else heavy without justification in the PR. Don't regress the
generator/editor (its tests must pass untouched or with stated updates). If
Overpass is slow for the SF bbox, check in a cached response under
`assets/maps/` (mind the size — thin it like the test fixture). Where the
full scope is at risk, cut in this order: patrol/hold-position, medic,
spawner buildings (keep edge spawns), supply-drop POIs — and say so.

## Suggested order

1 (map→collision+navgrid) → 5 (physics) → 2 (units/controls) → 4 (fog/LOS)
→ 3 (waves/loop) → 6 (tests/docs) — keep it compiling and playable at every
step.
