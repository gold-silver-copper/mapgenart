//! Procedural sound: every effect is synthesised into a PCM buffer at
//! startup (no binary assets) and played spatially relative to the camera.
//! Degrades to silence when no audio device exists (headless/CI).

use super::logic::{Score, TracerFx};
use super::population::Noise;
use super::tuning::*;
use super::units::{Dormant, Enemy};
use bevy::audio::{AudioPlayer, AudioSource, PlaybackSettings, Volume};
use bevy::prelude::*;
use std::sync::Arc;

pub const SAMPLE_RATE: u32 = 22_050;

/// Cheap deterministic noise for synthesis.
pub struct Lcg(u64);
impl Lcg {
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
    }
}

/// Build a mono f32 buffer of `secs` seconds with `f(t_seconds, i) -> sample`.
pub fn synth(secs: f32, mut f: impl FnMut(f32, &mut Lcg) -> f32) -> Vec<f32> {
    let n = (secs * SAMPLE_RATE as f32) as usize;
    let mut rng = Lcg(0x9E3779B97F4A7C15);
    (0..n)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            f(t, &mut rng).clamp(-1.0, 1.0)
        })
        .collect()
}

fn env(t: f32, attack: f32, decay: f32) -> f32 {
    if t < attack {
        t / attack
    } else {
        (-(t - attack) / decay).exp()
    }
}

/// One-pole low-pass state for filtered noise.
struct Lp(f32);
impl Lp {
    fn run(&mut self, x: f32, k: f32) -> f32 {
        self.0 += (x - self.0) * k;
        self.0
    }
}

pub fn rifle() -> Vec<f32> {
    let mut lp = Lp(0.0);
    synth(0.35, |t, r| {
        let crack = r.next() * env(t, 0.002, 0.02) * 1.0;
        let body = lp.run(r.next(), 0.25) * env(t, 0.004, 0.09) * 0.9;
        let thump = (t * 90.0 * std::f32::consts::TAU).sin() * env(t, 0.003, 0.05) * 0.7;
        crack + body + thump
    })
}

pub fn gunner() -> Vec<f32> {
    let mut lp = Lp(0.0);
    synth(0.18, |t, r| {
        let crack = r.next() * env(t, 0.001, 0.012) * 0.8;
        let body = lp.run(r.next(), 0.35) * env(t, 0.002, 0.05) * 0.7;
        crack + body
    })
}

pub fn bayonet() -> Vec<f32> {
    let mut lp = Lp(0.0);
    synth(0.15, |t, r| {
        lp.run(r.next(), 0.6) * env(t, 0.005, 0.04) * 0.6
    })
}

pub fn shriek() -> Vec<f32> {
    synth(0.9, |t, r| {
        let f = 600.0 + 900.0 * (t * 3.0).min(1.0) + 40.0 * (t * 37.0).sin();
        let tone = (t * f * std::f32::consts::TAU).sin() * 0.6;
        let rasp = r.next() * 0.25;
        (tone + rasp) * env(t, 0.05, 0.5)
    })
}

pub fn hammer() -> Vec<f32> {
    let mut lp = Lp(0.0);
    synth(0.12, |t, r| {
        let knock = (t * 180.0 * std::f32::consts::TAU).sin() * env(t, 0.002, 0.03);
        knock * 0.8 + lp.run(r.next(), 0.5) * env(t, 0.001, 0.02) * 0.4
    })
}

pub fn splinter() -> Vec<f32> {
    let mut lp = Lp(0.0);
    synth(0.6, |t, r| {
        let crack = r.next() * env(t, 0.003, 0.08);
        let rumble = lp.run(r.next(), 0.08) * env(t, 0.01, 0.35) * 1.2;
        (crack * 0.7 + rumble).clamp(-1.0, 1.0)
    })
}

pub fn rummage() -> Vec<f32> {
    let mut lp = Lp(0.0);
    synth(0.08, |t, r| {
        lp.run(r.next(), 0.4) * env(t, 0.005, 0.03) * 0.35
    })
}

pub fn chime() -> Vec<f32> {
    synth(0.8, |t, _| {
        let a = (t * 660.0 * std::f32::consts::TAU).sin();
        let b = (t * 880.0 * std::f32::consts::TAU).sin() * (t > 0.15) as u8 as f32;
        (a + b) * 0.3 * env(t, 0.01, 0.4)
    })
}

pub fn alarm() -> Vec<f32> {
    synth(2.0, |t, _| {
        let f = if (t * 2.0).fract() < 0.5 {
            520.0
        } else {
            700.0
        };
        (t * f * std::f32::consts::TAU).sin() * 0.4 * env(t, 0.05, 2.5)
    })
}

pub fn victory() -> Vec<f32> {
    synth(1.6, |t, _| {
        let notes = [523.0, 659.0, 784.0, 1046.0];
        let i = ((t * 4.0) as usize).min(3);
        (t * notes[i] * std::f32::consts::TAU).sin() * 0.35 * env(t, 0.01, 1.2)
    })
}

pub fn defeat() -> Vec<f32> {
    synth(1.8, |t, _| {
        let f = 220.0 - 60.0 * t;
        (t * f * std::f32::consts::TAU).sin() * 0.4 * env(t, 0.02, 1.0)
    })
}

/// Looping horde bed: layered slow breathing + groans.
pub fn horde_loop() -> Vec<f32> {
    let mut lp = Lp(0.0);
    synth(4.0, |t, r| {
        let breath = lp.run(r.next(), 0.05) * (0.5 + 0.5 * (t * 0.7 * std::f32::consts::TAU).sin());
        let groan = (t * (70.0 + 10.0 * (t * 1.3).sin()) * std::f32::consts::TAU).sin()
            * 0.2
            * (0.5 + 0.5 * (t * 0.4 * std::f32::consts::TAU + 1.0).sin());
        (breath * 0.8 + groan) * 0.6
    })
}

fn to_source(samples: &[f32]) -> AudioSource {
    // 16-bit PCM WAV in memory
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    let data_len = (samples.len() * 2) as u32;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    bytes.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        bytes.extend_from_slice(&((s * 32767.0) as i16).to_le_bytes());
    }
    AudioSource {
        bytes: Arc::from(bytes.into_boxed_slice()),
    }
}

#[derive(Resource)]
pub struct Sfx {
    pub rifle: Handle<AudioSource>,
    pub gunner: Handle<AudioSource>,
    pub bayonet: Handle<AudioSource>,
    pub shriek: Handle<AudioSource>,
    pub hammer: Handle<AudioSource>,
    pub splinter: Handle<AudioSource>,
    pub rummage: Handle<AudioSource>,
    pub chime: Handle<AudioSource>,
    pub alarm: Handle<AudioSource>,
    pub victory: Handle<AudioSource>,
    pub defeat: Handle<AudioSource>,
    pub horde: Handle<AudioSource>,
}

/// Master volume (0..1) and mute; `-`/`+`/`M` in the pause menu.
#[derive(Resource)]
pub struct AudioSettings {
    pub volume: f32,
    pub muted: bool,
}

impl Default for AudioSettings {
    fn default() -> Self {
        AudioSettings {
            volume: 0.8,
            muted: false,
        }
    }
}

/// Voice budget: live one-shot players.
#[derive(Component)]
pub struct Voice {
    pub priority: u8,
}

#[derive(Component)]
pub struct HordeBed;

/// Where a sound should play from (world coords) — pan/attenuate vs camera.
#[derive(Message)]
pub struct PlaySfx {
    pub kind: SfxKind,
    pub pos: Option<Vec2>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SfxKind {
    Rifle,
    Gunner,
    Bayonet,
    Shriek,
    Hammer,
    Splinter,
    Rummage,
    Chime,
    Alarm,
    Victory,
    Defeat,
}

impl SfxKind {
    fn priority(self) -> u8 {
        match self {
            SfxKind::Shriek | SfxKind::Alarm | SfxKind::Victory | SfxKind::Defeat => 3,
            SfxKind::Rifle | SfxKind::Gunner | SfxKind::Splinter | SfxKind::Chime => 2,
            _ => 1,
        }
    }
}

pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AudioSettings>()
            .add_message::<PlaySfx>()
            .add_systems(Startup, build_sfx)
            .add_systems(
                Update,
                (route_events, play_queued, horde_bed, volume_keys).chain(),
            );
    }
}

fn build_sfx(mut commands: Commands, sources: Option<ResMut<Assets<AudioSource>>>) {
    // no audio asset store (headless / no device): stay silent
    let Some(mut sources) = sources else { return };
    let mut add = |v: Vec<f32>| sources.add(to_source(&v));
    commands.insert_resource(Sfx {
        rifle: add(rifle()),
        gunner: add(gunner()),
        bayonet: add(bayonet()),
        shriek: add(shriek()),
        hammer: add(hammer()),
        splinter: add(splinter()),
        rummage: add(rummage()),
        chime: add(chime()),
        alarm: add(alarm()),
        victory: add(victory()),
        defeat: add(defeat()),
        horde: add(horde_loop()),
    });
}

/// Translate gameplay messages into sound requests.
fn route_events(
    mut out: MessageWriter<PlaySfx>,
    mut tracers: MessageReader<TracerFx>,
    mut noises: MessageReader<Noise>,
    mut over: MessageReader<super::logic::GameOver>,
    shooters: Query<(&Transform, &super::units::Soldier)>,
) {
    for t in tracers.read() {
        if t.heal {
            continue;
        }
        // which class fired? nearest soldier to the tracer origin
        let class = shooters
            .iter()
            .min_by(|a, b| {
                a.0.translation
                    .truncate()
                    .distance_squared(t.from)
                    .total_cmp(&b.0.translation.truncate().distance_squared(t.from))
            })
            .map(|(_, s)| s.class);
        let kind = match class {
            Some(super::units::Class::Gunner) => SfxKind::Gunner,
            _ => SfxKind::Rifle,
        };
        // a bayonet hit is a very short tracer
        let kind = if t.from.distance(t.to) <= BAYONET_RANGE + 0.5 {
            SfxKind::Bayonet
        } else {
            kind
        };
        out.write(PlaySfx {
            kind,
            pos: Some(t.from),
        });
    }
    for n in noises.read() {
        // non-gunfire noise sources are identified by their radius
        let kind = if (n.radius - NOISE_HAMMER).abs() < 0.5 {
            Some(SfxKind::Hammer)
        } else if (n.radius - 30.0).abs() < 0.5 {
            Some(SfxKind::Splinter)
        } else if n.radius >= NOISE_RIFLE * SHRIEKER_SCREAM_MULT - 0.5
            && n.radius < EXTRACT_ALARM_RADIUS - 0.5
        {
            Some(SfxKind::Shriek)
        } else if (n.radius - EXTRACT_ALARM_RADIUS).abs() < 0.5 {
            Some(SfxKind::Alarm)
        } else {
            None
        };
        if let Some(kind) = kind {
            out.write(PlaySfx {
                kind,
                pos: Some(n.pos),
            });
        }
    }
    for g in over.read() {
        out.write(PlaySfx {
            kind: if g.victory {
                SfxKind::Victory
            } else {
                SfxKind::Defeat
            },
            pos: None,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn play_queued(
    mut commands: Commands,
    sfx: Option<Res<Sfx>>,
    settings: Res<AudioSettings>,
    mut queue: MessageReader<PlaySfx>,
    cam: Query<&Transform, With<Camera2d>>,
    voices: Query<(Entity, &Voice)>,
) {
    let Some(sfx) = sfx else {
        queue.clear();
        return;
    };
    let cam_pos = cam
        .iter()
        .next()
        .map(|t| t.translation.truncate())
        .unwrap_or(Vec2::ZERO);
    let mut live: Vec<(Entity, u8)> = voices.iter().map(|(e, v)| (e, v.priority)).collect();
    for req in queue.read() {
        if settings.muted {
            continue;
        }
        let (gain, pan) = match req.pos {
            Some(p) => {
                let d = p.distance(cam_pos);
                let att = (1.0 - d / AUDIO_HEAR_RADIUS).clamp(0.12, 1.0);
                let pan = ((p.x - cam_pos.x) / 160.0).clamp(-1.0, 1.0);
                (att, pan)
            }
            None => (1.0, 0.0),
        };
        let pri = req.kind.priority();
        // voice cap: drop the lowest-priority live voice if we're full
        if live.len() >= MAX_VOICES
            && let Some((idx, _)) = live.iter().enumerate().min_by_key(|(_, (_, p))| *p)
        {
            if live[idx].1 <= pri {
                let (e, _) = live.swap_remove(idx);
                commands.entity(e).despawn();
            } else {
                continue;
            }
        }
        let handle = match req.kind {
            SfxKind::Rifle => sfx.rifle.clone(),
            SfxKind::Gunner => sfx.gunner.clone(),
            SfxKind::Bayonet => sfx.bayonet.clone(),
            SfxKind::Shriek => sfx.shriek.clone(),
            SfxKind::Hammer => sfx.hammer.clone(),
            SfxKind::Splinter => sfx.splinter.clone(),
            SfxKind::Rummage => sfx.rummage.clone(),
            SfxKind::Chime => sfx.chime.clone(),
            SfxKind::Alarm => sfx.alarm.clone(),
            SfxKind::Victory => sfx.victory.clone(),
            SfxKind::Defeat => sfx.defeat.clone(),
        };
        let vol = gain * settings.volume;
        // stereo pan via spatial playback: position the emitter left/right of a listener at the camera
        let e = commands
            .spawn((
                AudioPlayer::new(handle),
                PlaybackSettings::DESPAWN
                    .with_volume(Volume::Linear(vol))
                    .with_spatial(true),
                Transform::from_xyz(pan * 4.0, 0.0, 0.0),
                Voice { priority: pri },
            ))
            .id();
        live.push((e, pri));
    }
}

/// A looping horde bed whose gain follows nearby awake enemies.
fn horde_bed(
    mut commands: Commands,
    sfx: Option<Res<Sfx>>,
    settings: Res<AudioSettings>,
    cam: Query<&Transform, With<Camera2d>>,
    awake: Query<&Transform, (With<Enemy>, Without<Dormant>)>,
    mut bed: Query<&mut bevy::audio::AudioSink, With<HordeBed>>,
) {
    let Some(sfx) = sfx else { return };
    let cam_pos = cam
        .iter()
        .next()
        .map(|t| t.translation.truncate())
        .unwrap_or(Vec2::ZERO);
    let near = awake
        .iter()
        .filter(|t| t.translation.truncate().distance(cam_pos) < 120.0)
        .count();
    let target = if settings.muted {
        0.0
    } else {
        ((near as f32) / 25.0).min(1.0) * settings.volume * 0.7
    };
    match bed.single_mut() {
        Ok(mut sink) => {
            let v = sink.volume().to_linear();
            sink.set_volume(Volume::Linear(v + (target - v) * 0.05));
        }
        Err(_) => {
            commands.spawn((
                AudioPlayer::new(sfx.horde.clone()),
                PlaybackSettings::LOOP.with_volume(Volume::Linear(0.0)),
                HordeBed,
            ));
        }
    }
}

/// `-` / `+` volume and `M` mute (while the pause menu is open or any time).
fn volume_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut settings: ResMut<AudioSettings>,
    score: Option<Res<Score>>,
) {
    if keys.just_pressed(KeyCode::Minus) {
        settings.volume = (settings.volume - 0.1).max(0.0);
    }
    if keys.just_pressed(KeyCode::Equal) {
        settings.volume = (settings.volume + 0.1).min(1.0);
    }
    if keys.just_pressed(KeyCode::KeyM) && score.is_some() {
        settings.muted = !settings.muted;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffers_are_bounded_and_audible() {
        for (name, buf) in [
            ("rifle", rifle()),
            ("gunner", gunner()),
            ("shriek", shriek()),
            ("hammer", hammer()),
            ("splinter", splinter()),
            ("chime", chime()),
            ("alarm", alarm()),
            ("victory", victory()),
            ("defeat", defeat()),
            ("horde", horde_loop()),
        ] {
            assert!(!buf.is_empty(), "{name} empty");
            assert!(
                buf.iter()
                    .all(|s| s.is_finite() && (-1.0..=1.0).contains(s)),
                "{name} out of range"
            );
            let peak = buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            assert!(peak > 0.05, "{name} silent (peak {peak})");
        }
    }

    #[test]
    fn wav_header_is_valid() {
        let src = to_source(&rifle());
        assert_eq!(&src.bytes[0..4], b"RIFF");
        assert_eq!(&src.bytes[8..12], b"WAVE");
        assert_eq!(src.bytes.len() % 2, 0);
    }
}
