# Prompt: milestone 4 — make it a game: noise, extraction, scarcity, names, barricades, night

Turn "Last Light" from a wave-survival demo into an actual game, as a single
PR on this repository (Rust + Bevy 0.19 + avian2d). Read `README.md`,
`docs/PR-milestone-3.md` and `src/game/` first; extend the existing systems
(`logic.rs` alerts/flow field, `buildings.rs` doors/windows, `world.rs`,
`units.rs`, `view.rs`) rather than rewriting. All six features below are in
scope, in this priority order — if anything must be cut, cut from the bottom
and say so. Keep `cargo build`, `cargo test`, `cargo clippy --all-targets`
clean, keep `--sim-ticks` headless runs working, keep native + wasm building,
and keep the editor (`--edit`) untouched.

## 1. Noise-driven world (replaces timed waves)

Delete the wave director. At map load, seed a fixed sleeping population
scaled to map area (`--population N` override): dormant enemies placed in
building interiors (heavier) and streets (lighter), in the main walkable
region, visible only per fog rules. Dormant enemies don't move and cost
almost nothing per frame (no physics until woken — spawn their
RigidBody/Collider lazily on wake, or use a dormant marker that skips all
systems). Waking: gunfire wakes sleepers within its noise radius (gunner
louder than rifle, medic silent); a woken enemy shrieks once, waking others
in a smaller radius (chain with falloff so one shot doesn't wake the map);
sight of a soldier wakes; barricade hammering (see 5) is mid-loud. Woken
enemies use the existing alert/flow/wander AI; calm ones that lose all
alerts go back to dormant after ~30 s. HUD shows a noise meter (recent noise
made) so the player can feel how loud they are. Difficulty knob: total
population + wake radii, not spawn rate.

## 2. Extraction objective

A run has a goal: reach and hold the extraction point. Pick it at map load:
the named POI or `place` node farthest from the squad spawn (fall back to
the farthest reachable nav cell); show its real name in the briefing
("Evacuation at Golden Gate Park — hold 60 s"). Add 1–2 mid objectives
generated from real map features between spawn and extraction (e.g. "search
the pharmacy on <street/POI name> for the medical cache") — reaching one
grants a substantial supply reward. The extraction requires holding the
marked circle for 60 s (any soldier inside keeps the timer running; it
pauses, not resets, when contested), which triggers a heavy converging
horde. Surviving the hold = **victory screen** with run stats (time, kills,
shots fired, soldiers lost, loudest moment). Objective markers: on-map flag,
minimap icon, and an off-screen edge arrow toward the current objective.
Menu text and game-over/victory screens updated accordingly.

## 3. Ammo & scavenging economy

Shots consume ammo from a squad pool (gunner burns ~3× per second vs
rifle); at 0 ammo soldiers fight with bayonets (melee range, weak — a run
low on ammo is desperate, not dead). Meds: medic heal consumes meds; scrap:
spent by barricades (5) and recruiting at supply drops. Buildings become
lootable: each interior gets a loot value scaled by its size and OSM tag
(supermarket/pharmacy/hospital POIs rich, generic buildings poor, already-
looted marked); a soldier standing in an unlooted interior scavenges it
over a few seconds (progress ring), rolling ammo/meds/scrap. Sleepers
indoors make looting a push-your-luck act. Starting stock is enough for
roughly two loud fights. HUD: ammo/meds/scrap counters with low-ammo
warning. Existing supply drops become rarer and richer. Tune so a quiet,
scavenging squad stays supplied and a loud one starves.

## 4. Named soldiers & permadeath weight

Each soldier gets a generated name (small embedded name list, deterministic
from SimRng) shown over their head at close zoom, in the selection status
line, and on health bars' tooltips. Track per-soldier kills and shots.
Kills grant ranks (5/15/40 kills): each rank gives +8% damage and −10%
noise radius (steadier, quieter). Death is permanent for the run: show a
brief "☠ <name> — <kills> kills" feed line, and list the fallen on the
game-over/victory screen. Recruits arrive unranked. No meta-progression
between runs.

## 5. Barricades

One build action, no base-building: with a soldier selected, `B` then click
a door or window within reach orders them to board it up (3 s channel,
costs scrap, makes hammering noise per 1). A barricaded opening blocks
enemy movement (and movement for everyone — plan your own exits); windows
stay see-through/shootable when boarded? No — boarding a window blocks
sight and shots both ways; boarding a door blocks movement only. Enemies
that path into a barricade attack it (barricades have HP, splinter
visibly/audibly when breaking). Implement as: carve state change (blocked/
sight masks + collider add/remove + repaint pixels + navgrid/flow
invalidation for the affected cells). `Delete`-equivalent: a soldier can
tear down a friendly barricade (1 s, refunds half scrap). Keep the count
per run in stats.

## 6. Day/night pressure cycle

A run-long clock (~4 real minutes day, ~3 night, shown as a small sun/moon
dial): by day sleepers have small wake radii and woken enemies calm down
quickly; at night wake radii double, calming stops, and dormant enemies
occasionally self-wake and wander. Nights get a blue-dark map tint (overlay
tween, fog still layered above) and slightly reduced soldier vision radius.
The extraction hold is intentionally scarier at night — do not special-case
it. First night begins after the player has had time to loot (~4 min).

## Quality bar

- Headless: `--sim-ticks` still runs the full loop (population seeding,
  waking, objectives, economy) deterministically; extend the summary line
  with population/awake counts, ammo, objective state.
- Tests (extend `tests/game.rs`): population seeding lands only in the main
  region with the indoor/outdoor split; wake-chain falloff (one shot wakes
  a bounded count); dormant enemies cost no physics (entity count without
  colliders); objective selection picks a reachable far point and mid
  objectives lie between; scavenge loot respects POI richness; ammo pool
  drains and bayonet fallback engages; barricade blocks nav + flow and
  enemies break it; rank thresholds; day/night wake-radius switch. Golden
  tests and all existing 61 tests stay green (regenerate deliberately only
  with a stated reason).
- Performance: population for downtown SF ≈ 600–1200 sleepers must not
  regress frame cost while dormant (measure and state numbers in the PR).
- README: rewrite the game section around the new loop (be quiet, loot,
  move, extract). `docs/PR-milestone-4.md` with design decisions, tuning
  values chosen, measured perf, and cut scope.

## Constraints

No new dependencies. Don't break `--edit`, the wasm demo (bundled fixture
map gets a small population and a nearby extraction — fine if easy), or
`--bbox` play on arbitrary cities. Tuning values (population density, noise
radii, loot tables, day length) live in one `mod tuning` with comments, not
scattered magic numbers. Playtest via `--sim-ticks` scenarios plus at least
one windowed `MAPGEN_AUTOSTART` smoke run per feature; report results
honestly in the PR doc.

## Suggested order

1 (noise world) → 3 (economy) → 2 (extraction) → 4 (names) → 5 (barricades)
→ 6 (night) — the game must remain runnable after every step.
