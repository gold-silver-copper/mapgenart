# Prompt: milestone 5 — make it legible and tense: sound, cause-and-effect, enemy archetypes, a director

Build the next milestone of "Last Light" as a single PR on this repository
(Rust + Bevy 0.19 + avian2d). Read `README.md`, `docs/PR-milestone-4.md`
and `src/game/` first; extend `population.rs` (noise/wake), `logic.rs`,
`units.rs`, `view.rs`, `tuning.rs` rather than rewriting. Four features, in
this priority order — cut from the bottom only if you must, and say so.
Keep `cargo build`, `cargo test`, `cargo clippy --all-targets` clean, keep
`--sim-ticks` headless runs and the wasm build working, keep the editor
(`--edit`) untouched.

## 1. Sound

Enable Bevy's built-in audio (the `bevy_audio` feature; no `bevy_kira_audio`)
and generate every effect procedurally at startup — no binary assets:
synthesise short PCM buffers (noise bursts, decaying sines, filtered noise)
into `AudioSource`s for: rifle shot, gunner burst, bayonet stab, shriek
(rising sine + noise), hammering tick, barricade splinter/crash, loot
"rummage" tick, objective secured chime, extraction alarm/flare, victory and
defeat stings, and a low "horde" loop (layered breathing/groans) whose gain
follows the number of awake enemies within ~120 px. Play them spatially:
volume by distance to the camera centre (off-screen sounds quieter, never
silent for shots you fired), stereo pan by horizontal offset, hard cap on
simultaneous voices with priority (shriek > shot > tick). Master/SFX volume
in the pause menu (`-`/`+`), `M` mutes. Headless builds must not require an
audio device (audio systems gated on the plugin/resource existing).

## 2. Visible cause and effect

- Every `Noise` draws an expanding, fading ring (gizmos) at its true radius
  so the player sees exactly what they woke.
- Enemies that wake show a brief "!" (Text2d or gizmo glyph) and a short
  stand-up animation (two-frame sprite swap: slumped ↔ upright). Dormant
  sprites are drawn slumped; awake upright; alerted ones slightly red-tinted.
- Off-screen threat indicators: when awake enemies approach from outside the
  view, pulse a red wedge on that screen edge (strength by count).
- The minimap marks recent noise events (fading dots) and awake enemies
  you have seen in the last 5 s even if they've left vision (last-known
  ghost blips).
- Shots leave brief muzzle flashes; hits spawn a 3-pixel blood spatter that
  stays on the map texture (decals accumulate; cap ~4 000 then oldest fade).

## 3. Enemy archetypes

Replace the single infected with three, spawned by the population seeder
with tuned ratios (`tuning.rs`):
- **Shambler** (≈75 %) — today's enemy.
- **Shrieker** (≈8 %) — fragile (12 hp), slower; when it *dies* it screams:
  a `Noise` of 3× rifle radius. Target priority becomes a real question.
  Distinct sprite (thin, pale) and shriek variant sound.
- **Runner** (≈12 %) — fast (2× shambler), low hp, only wakes at night or
  from shrieks/loud noise; runs straight (uses direct chase from further
  out). Distinct sprite (lean, dark).
- **Brute** (≈5 %) — slow, 6× hp, hits barricades for 5× damage and can
  push through unit crowds (higher mass). The barricade-breaker. Distinct
  large sprite (3× area); soldiers' bayonets barely scratch it.
Enemy AI keeps its perception/alert logic; add per-archetype `Kind` on the
`Enemy` component with speed/hp/damage/wake rules, and make the fog/minimap
blips colour-code kind once seen.

## 4. Intensity director

A Left-4-Dead-style pacing resource that keeps tension oscillating:
- Track *intensity* (awake enemies within 150 px of the squad, recent
  damage taken, recent noise) with decay.
- **Lull breaker**: if intensity has stayed near zero for ~90 s of play, wake
  a small "scout pack" (3–6 sleepers) 120–200 px away and give them an alert
  at the squad's position with noise; do not do this during the extraction
  hold or within 40 s of a previous director action.
- **Relief valve**: if intensity is very high for >30 s and the squad is
  below half total hp, temporarily halve wake radii and speed up calm-down
  for 30 s (never at night, never during extraction).
- **Build-up**: 60 s before each night, wake a few sleepers far from the
  squad so the night arrives with distant movement on the minimap.
- Log director decisions (`log::info!`) and expose the intensity value on
  the HUD (a small bar) and in the `--sim-ticks` summary.

## Quality bar

- Tests (extend `tests/game.rs`): synthesised buffers are non-silent and
  bounded (−1..1, no NaN); voice cap respected; shrieker death emits a
  larger `Noise` than a rifle shot; runners don't wake from daytime sight;
  brute barricade damage vs shambler; director lull breaker fires in a quiet
  headless run and never during extraction; decal cap honoured. Existing
  69 tests stay green.
- Headless: `--sim-ticks` summary adds archetype counts and director state.
- Performance: audio + rings + decals must not regress the 1 000-enemy SF
  tick cost measurably (state before/after ms/tick).
- README: update the game section (enemy types, sound, director);
  `docs/PR-milestone-5.md` with design decisions, tuning values, perf, cuts.

## Constraints

No new crates (Bevy's own audio only; synthesis by hand). Keep all knobs in
`tuning.rs`. Windowed smoke run per feature via `MAPGEN_AUTOSTART=1`
(sound may fail to open a device in CI/sandbox — must degrade to silent
without panicking). Deterministic headless behaviour must not depend on the
audio system.

## Suggested order

3 (archetypes) → 2 (cause-and-effect) → 4 (director) → 1 (sound) — the
game must remain runnable after every step.
