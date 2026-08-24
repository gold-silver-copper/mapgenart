# PR: milestone 4 — the game: noise, extraction, scarcity, names, barricades, night

Implements all six features of `docs/milestone-4-prompt.md` in priority
order, no cuts. The run loop is now: *stay quiet → loot → move → extract.*

## What changed

**1. Noise-driven world** — waves deleted. A fixed sleeping population is
seeded at load (`population.rs`; ~16 per 10k walkable px → **1 038 sleepers**
in downtown SF; 62 % indoors), placed in the main walkable region; dormant
enemies carry no physics (collider/rigidbody attached only on wake, stripped
again on calm). Waking: `Noise` messages (rifle 70 px, gunner 95 px, hammer
45 px, bayonet 10 px; rank-quieted), one-hop shriek chain (26 px falloff),
sight (26 px + LOS — windows count), and night restlessness. Awake+idle
enemies re-sleep after 30 s (daytime only). HUD noise meter
(quiet/low/LOUD/DEAFENING).

**2. Extraction** — `objectives.rs` picks the farthest reachable *named*
feature as evac (real names: "Whole Foods Market") plus up to two search
objectives at ~40 %/70 % of the way, each verified A*-reachable. Search =
walk there, big supply reward. Extraction = enter the circle (fires a
400 px alarm), hold 60 s (pauses while contested, never resets) → victory
screen with time/kills/shots/barricades/loudest + the fallen. Pulsing map
ring, minimap-agnostic edge arrow when off-screen.

**3. Economy** — squad `Stockpile` (ammo 260 / meds 30 / scrap 20 to start).
Shots consume ammo (gunner ~3× drain), dry pool = bayonets (5 px, weak,
near-silent — desperate, not dead). Medic heals cost meds. Interiors are
loot sites valued by size, ×4 near pharmacy/supermarket/hospital POIs
(capped); standing inside drains the site into the pool at 14/s (60/20/20
ammo/meds/scrap). Recruit supply drops now cost 12 scrap to take.

**4. Names & permadeath** — `Dossier` per soldier (deterministic name table:
Reyes "Ghost"), kills/shots tracked, ranks at 5/15/40 kills (+8 % damage,
−10 % noise each), floating Text2d name+rank tags at close zoom, death feed
line, fallen list on the end screen.

**5. Barricades** — `B` + click a carved door/window: the nearest selected
soldier walks over and channels 3 s (hammering noise/s, 4 scrap). Doors
block movement; windows also block sight both ways. Implemented as pixel
mask + nav-cell + tight-mask surgery (`barricade::set_masks`) plus a static
collider, map-texture repaint (original pixels restored on removal), flow
field re-routes on its next refresh. Enemies with an alert smash adjacent
barricades (120 hp, splinter noise, crash on collapse). Tear-down: 1 s,
half refund.

**6. Day/night** — 4 min day / 3 min night clock: night doubles wake radii,
disables calming, self-wakes sleepers, dims soldier vision ×0.85 and fades
in a blue shade sprite between map and fog. HUD shows ☀/☾. First night
arrives after the 4-minute looting grace.

Also: `--population N` override; headless summary now reports dormant/awake
counts, stockpile and current objective; all tuning in `game/tuning.rs`.

## Verification & performance (M2 Max, debug)

- **69 tests green**, clippy clean, native + wasm build, editor untouched.
- New tests: dormant seeding in-bounds with zero colliders; one rifle shot
  on SF (600 pop) wakes a bounded minority; objectives reachable + mids
  nearer than evac; loot richness varies >2×; ammo drains and stays ≥0;
  rank thresholds + name determinism; barricade closes/reopens nav cells;
  night wake multiplier.
- Headless SF (1024 px, 20 401 colliders): 1 000 ticks with **1 004 enemies
  (894 dormant, ~110 woken and fighting)** ≈ 10.6 ms/tick vs 7.1 ms/tick
  with population 0 — the dormant 894 cost roughly nothing; the delta is the
  ~110 awake (physics + AI) plus the fight itself.
- Windowed SF smoke run: seeds 1 038 sleepers, picks "Whole Foods Market"
  evac + 2 search objectives, no panics.

## Design decisions

- Dormant = component-stripped entities (no lazy spawn table): wake/sleep is
  a component insert/remove, so fog visibility, corpses and stealth kills
  work on sleepers for free.
- The wake chain is one hop deep by design; density does the spreading in
  crowded blocks while a lone street sleeper stays a lone problem.
- Victory/defeat both go through the same `GameOver { victory }` message and
  end screen; stats live in `Score`.
- Barricade flow-field handling is emergent: sealed routes make the field
  route around, and a fully sealed squad makes hordes fall back to walking
  at their last alert — straight into the boards they then smash.

## Tuning chosen (see `game/tuning.rs`)
Population 16/10k px · rifle 70 px vs gunner 95 px noise · shriek 26 px ·
calm 30 s · start 260/30/20 · scavenge 14/s · loot ×4 at POIs · ranks
5/15/40 · barricade 4 scrap/120 hp/3 s · day 240 s, night 180 s (×2 wake).
