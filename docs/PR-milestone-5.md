# PR: milestone 5 — legible and tense: archetypes, cause-and-effect, director, sound

Implements all four features of `docs/milestone-5-prompt.md`, none cut.

## What changed

**3. Enemy archetypes** (`units::EnemyKind`, ratios in `tuning.rs`):
Shambler 75 % · **Shrieker** 8 % (12 hp, slow; death emits a `Noise` of
3× rifle radius) · **Runner** 12 % (0.6× hp, 2× speed, 2× chase range; sleeps
through daytime sight and soft noise — wakes only to gunfire-level noise,
shrieks, or at night) · **Brute** 5 % (6× hp, 0.6× speed, 1.8× damage,
1.8× collider, mass 6, 5× barricade damage; bayonets do 25 %). Distinct
sprites (awake + slumped per kind), colour-coded minimap blips, archetype
counts in the headless summary. SF seeds ~736/80/114/55.

**2. Visible cause and effect** (`view.rs`): expanding fading gizmo ring at
every `Noise` event's true radius; sleepers drawn slumped, woken upright
with a 0.6 s "!" glyph (`JustWoke`), alerted ones red-tinted; red pulsing
edge wedges where alerted enemies approach off-screen (strength by count);
minimap shows fading noise dots and last-known ghost blips (5 s) for
enemies that left vision; muzzle flashes; 3-pixel blood decals painted into
the map texture, capped at 4 000 (oldest fade toward ground).

**4. Intensity director** (`director.rs`): intensity = hunting enemies within
150 px ×6 + squad damage ×1.5 + noise heard, decaying 6/s. *Lull breaker*:
intensity <5 for 90 s → wake 3–6 sleepers 120–200 px out, alert them to a
noisy guess of the squad (40 s cooldown, never while extracting). *Relief
valve*: intensity >60 for 30 s with squad under half hp → wake radii ×0.5
and calm-down ×3 for 30 s (never at night/extracting). *Pre-night build-up*:
60 s before dark, 6 distant sleepers stir. Decisions logged; HUD tension
bar; state in the `--sim-ticks` summary.

**1. Sound** (`audio.rs`, Bevy's `bevy_audio` + `wav`, no new crates):
twelve effects synthesised into in-memory 22 kHz WAVs at startup (noise
bursts through one-pole low-passes, exponential envelopes, sine sweeps):
rifle, gunner, bayonet, shriek, hammer, splinter, rummage, chime, alarm,
victory, defeat, and a 4 s looping horde bed. `PlaySfx` requests are routed
from `TracerFx`/`Noise`/`GameOver` messages, attenuated by distance to the
camera (floor 12 %), panned via spatial playback, capped at 24 voices with
priority eviction (shriek/alarm > shots > ticks). Horde bed gain follows
awake enemies within 120 px of the camera. `-`/`+`/`M` volume/mute. With no
audio asset store (headless) everything is a no-op.

## Verification & performance (M2 Max, debug)

- **77 tests green** (36 unit + 29 game + 12 generator), clippy clean,
  native + wasm build, windowed smoke run without panics (audio device opens
  on macOS; sandbox capture still black — environment).
- New tests: archetype ratios/stats; shrieker scream > rifle; brute vs
  barricade multiplier; runners ignore daytime hammering but wake to a
  rifle shot (sparse map, chain excluded); director acts within the lull
  window on SF and never during the extraction hold; decal cap sane; audio
  buffers bounded/non-silent, WAV header valid, voice cap sane.
- Perf, SF 1024 px, 1 000 headless ticks, default population (985 enemies,
  308 awake, 55 brutes): **4.9 s wall (~3.4 ms/tick)** vs milestone 4's
  12.4 s for the same command with 110 awake — archetypes, rings and decals
  did not add measurable cost (the run is dominated by physics for awake
  enemies; this run simply resolved its fights faster).

## Design decisions

- Shriek-chain waking applies to runners too (by spec: "shrieks/loud noise");
  the test isolates the daytime-hammer rule on a sparse map.
- Director intensity counts *hunting* enemies only, so a sleeper that woke
  and wandered off doesn't suppress the lull breaker.
- Sound identification is message-driven: shot class from the nearest
  soldier to the tracer origin; non-gunfire sounds keyed by their tuned
  noise radius (hammer 45, splinter 30, scream 210, alarm 400).
- A noise reader and a noise writer can't share a system (Bevy B0002) —
  the director listens in a separate system.
