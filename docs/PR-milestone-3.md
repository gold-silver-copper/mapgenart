# PR: milestone 3 — "Last Light": an RTS on real OSM maps

Implements `docs/milestone-3-prompt.md`: the repository is now a playable
real-time tactics game (default) with the map generator/editor behind
`--edit`. Bevy 0.19 + `avian2d` 0.7.

## What changed

**1. Map → world** — the pipeline gained a per-pixel `building` mask;
`blocked = buildings ∪ water`. `game/nav.rs` builds a half-resolution
walkability grid, `greedy_rects` meshes blocked pixels into axis-aligned
rectangles → Avian static colliders (`RigidBody::Static`, collision layers
World/Unit/Enemy, plus map-edge walls). Default map: downtown San Francisco
(`assets/maps/sf.json`, 3.5 MB thinned Overpass snapshot, 1024×720 px at
2.3 m/px, **20 401 colliders**) with `palettes/postapoc.toml` (ash, rust,
murky water). Any `--bbox` on earth still works.

**2. Units & control** — 8-soldier squad (rifleman/gunner/medic — distinct
stats and generated 13×13 pixel sprites), box/click/shift select, control
groups (Ctrl+1–9), right-click move with formation offsets, `A` attack-move,
`S` stop, `H` hold, `P` patrol; A* with string pulling, re-plan rate-limited
to 2 Hz per unit; camera edge/keys/minimap-click pan + wheel zoom; hitscan
combat with tracers, health bars, corpses; medic heals nearby allies.

**3. Enemies & loop** — waves spawn at walkable edge cells ≥90 px from the
squad (`6 + 5·wave` enemies, hp ×1.12/wave, speed capped); hordes follow a
Dijkstra **flow field** recomputed at 2 Hz (per-enemy cost is one array
lookup), switching to direct chase in close range and melee at contact.
Between waves a supply drop (medkit/ammo/recruit) lands at a real OSM POI
(hospital/pharmacy/supermarket, newly parsed as `Kind::Poi`). Menu → play →
game-over (score = waves + kills) → retry.

**4. Fog & LOS** — `game/fog.rs`: unexplored/explored/visible per map pixel;
perimeter raycasts per soldier with **buildings blocking sight** (the wall
itself stays visible, nothing behind it); enemies and their minimap blips
render only inside current vision; fog is an alpha overlay texture, minimap
is a fog-respecting downsample with unit blips, rebuilt at 5 Hz.

**5. Physics (Avian)** — dynamic circle bodies with damping and locked
rotation, zero gravity, soft unit-unit separation; spawn points snapped to
walkable cells; stress harness (`run_stress`) piles **208 units** into one
street and pushes them across the map.

**6. Quality** — `--sim-ticks N`: the same logic systems run under
`MinimalPlugins` with 16 ms manual virtual time (no window, deterministic
RNG) and print a summary; visual systems are cleanly separated in
`game/view.rs`. CI keeps native + wasm builds.

## Verification & performance (M2 Max, debug build)

| check | result |
|---|---|
| `cargo test` | **51 pass** (32 unit + 7 game + 12 generator), clippy clean |
| game tests | path-around-building (SF map), LOS blocked by real building, collider coverage exact vs blocked mask, wave scaling, headless smoke (0 units in blocked cells), stress: 208 units / 500 ticks → **0 tunneled, 0 NaN** |
| headless sim, SF 1024 px, 20 401 colliders | ~2.9 ms/tick (1000 ticks in 2.9 s) |
| stress 208 units, 500 ticks | 2.0 s ≈ 4 ms/tick |
| windowed SF run (`MAPGEN_AUTOSTART=1`) | loads, wave spawns, no panics, screenshot written |
| `cargo build --target wasm32-unknown-unknown` | ✅ (plays the bundled fixture map) |
| editor regression | `--edit` runs; all milestone-1/2 tests untouched and green |

## Design decisions

- **Flow field for hordes, A* for soldiers** — hundreds of enemies pay one
  grid lookup per frame; only player orders run A*. Per-unit A* re-plans are
  rate-limited (0.5 s) — without this a jammed 200-unit crowd ran A* every
  frame (74 ms/tick → 4 ms/tick).
- **Hitscan rifles** instead of projectile bodies (prompt allowed either);
  tracers/heal beams are one-frame gizmo lines.
- Nav cells are 2 px and conservatively blocked (any blocked pixel blocks the
  cell), so corner-cutting is impossible; the game needs ≥ ~1000 px maps for
  SF-density streets (the default).
- Feature-gated module split (not a workspace): `game/logic.rs` has no
  rendering imports, so the identical systems run headless; `game/view.rs`
  holds every visual.
- Screenshots: `F12` + `MAPGEN_AUTOSHOT`. In this sandboxed session, window
  surface capture produces black frames even for a stock Bevy app
  (environment limitation — verified with a minimal repro; `screencapture`
  is also permission-blocked), so PR screenshots must be taken on a desktop
  session. Map renders are verified via the headless PNG pipeline
  (`out/sf-test.png`).

## Cut scope (allowed by the prompt, stated)

- Spawner buildings: waves spawn at map edges only.
- Walk/fire/death animation frames: units are single-frame rotated sprites
  with muzzle tracers, damage flashes (hurt timer), and corpse sprites.
- 300-enemy wave benchmark: measured 208 simultaneous dynamic units at
  ~4 ms/tick in debug; release builds are far faster. Wave sizes reach 300+
  enemies by wave ~59 by formula; not separately profiled.
