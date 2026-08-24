//! Every gameplay tuning knob in one place.

/// --- population (noise-driven world) ---------------------------------------
/// sleepers per 10 000 walkable map pixels (≈ downtown SF → ~900)
pub const POP_PER_10K_WALKABLE: f32 = 16.0;
/// fraction of sleepers seeded inside building interiors (rest on streets)
pub const POP_INDOOR_FRACTION: f32 = 0.62;
/// gunshot noise radius in px (rifle); the gunner is louder, ranks are quieter
pub const NOISE_RIFLE: f32 = 70.0;
pub const NOISE_GUNNER: f32 = 95.0;
pub const NOISE_MELEE: f32 = 10.0;
pub const NOISE_HAMMER: f32 = 45.0;
/// a woken enemy shrieks once, waking others in this radius (chain falloff)
pub const SHRIEK_RADIUS: f32 = 26.0;
/// dormant enemy wakes when a soldier is visible within this range
pub const DORMANT_SIGHT: f32 = 26.0;
/// awake, alert-less, unseen enemies go back to sleep after this long
pub const CALM_AFTER_S: f32 = 30.0;
/// noise meter decay per second (HUD)
pub const NOISE_METER_DECAY: f32 = 18.0;

/// --- economy ----------------------------------------------------------------
pub const START_AMMO: f32 = 260.0;
pub const START_MEDS: f32 = 30.0;
pub const START_SCRAP: f32 = 20.0;
/// ammo per shot
pub const AMMO_RIFLE: f32 = 1.0;
pub const AMMO_GUNNER: f32 = 0.45; // per bullet; ~3× rifle drain per second
/// meds consumed per hp healed
pub const MEDS_PER_HP: f32 = 0.05;
/// bayonet fallback when the ammo pool is dry
pub const BAYONET_RANGE: f32 = 5.0;
pub const BAYONET_DAMAGE: f32 = 5.0;
/// scavenging: seconds to loot a site fully, per 100 loot value
pub const SCAVENGE_RATE: f32 = 14.0; // loot value per second while standing in a site
/// loot value assigned per interior, scaled by size; POI interiors are richer
pub const LOOT_BASE: f32 = 18.0;
pub const LOOT_PER_100PX: f32 = 6.0;
pub const LOOT_POI_MULT: f32 = 4.0;
pub const LOOT_VALUE_CAP: f32 = 260.0;

/// --- objectives -------------------------------------------------------------
pub const EXTRACT_HOLD_S: f32 = 60.0;
pub const EXTRACT_RADIUS: f32 = 14.0;
pub const MID_OBJECTIVE_RADIUS: f32 = 8.0;
/// waking surge when the extraction hold starts
pub const EXTRACT_ALARM_RADIUS: f32 = 400.0;
pub const MID_REWARD_AMMO: f32 = 90.0;
pub const MID_REWARD_MEDS: f32 = 18.0;
pub const MID_REWARD_SCRAP: f32 = 14.0;

/// --- ranks ------------------------------------------------------------------
pub const RANK_KILLS: [u32; 3] = [5, 15, 40];
pub const RANK_DAMAGE_BONUS: f32 = 0.08;
pub const RANK_NOISE_CUT: f32 = 0.10;

/// --- barricades -------------------------------------------------------------
pub const BARRICADE_SCRAP: f32 = 4.0;
pub const BARRICADE_BUILD_S: f32 = 3.0;
pub const BARRICADE_HP: f32 = 120.0;
pub const BARRICADE_TEAR_S: f32 = 1.0;
pub const BARRICADE_REFUND: f32 = 2.0;
/// enemy damage to barricades per hit
pub const BARRICADE_ENEMY_DMG: f32 = 8.0;

/// --- day/night --------------------------------------------------------------
pub const DAY_S: f32 = 240.0;
pub const NIGHT_S: f32 = 180.0;
/// wake radii multiplier at night; calming stops at night
pub const NIGHT_WAKE_MULT: f32 = 2.0;
/// chance per dormant enemy per second to self-wake at night
pub const NIGHT_SELF_WAKE_P: f32 = 0.0035;
pub const NIGHT_VISION_MULT: f32 = 0.85;

/// --- enemy archetypes ---------------------------------------------------------
/// seeding ratios (must sum ≤ 1; remainder are shamblers)
pub const RATIO_SHRIEKER: f32 = 0.08;
pub const RATIO_RUNNER: f32 = 0.12;
pub const RATIO_BRUTE: f32 = 0.05;
pub const SHRIEKER_HP: f32 = 12.0;
pub const SHRIEKER_SPEED_MULT: f32 = 0.8;
/// a dying shrieker screams this × rifle radius
pub const SHRIEKER_SCREAM_MULT: f32 = 3.0;
pub const RUNNER_HP_MULT: f32 = 0.6;
pub const RUNNER_SPEED_MULT: f32 = 2.0;
pub const RUNNER_CHASE_MULT: f32 = 2.0;
pub const BRUTE_HP_MULT: f32 = 6.0;
pub const BRUTE_SPEED_MULT: f32 = 0.6;
pub const BRUTE_DAMAGE_MULT: f32 = 1.8;
pub const BRUTE_BARRICADE_MULT: f32 = 5.0;
pub const BRUTE_MASS: f32 = 6.0;
/// bayonets do this fraction of damage to brutes
pub const BRUTE_BAYONET_FACTOR: f32 = 0.25;

/// --- director -------------------------------------------------------------------
pub const DIRECTOR_LULL_S: f32 = 90.0;
pub const DIRECTOR_COOLDOWN_S: f32 = 40.0;
pub const DIRECTOR_SCOUT_MIN: u32 = 3;
pub const DIRECTOR_SCOUT_MAX: u32 = 6;
pub const DIRECTOR_SCOUT_NEAR: f32 = 120.0;
pub const DIRECTOR_SCOUT_FAR: f32 = 200.0;
pub const DIRECTOR_HIGH_INTENSITY: f32 = 60.0;
pub const DIRECTOR_HIGH_FOR_S: f32 = 30.0;
pub const DIRECTOR_RELIEF_S: f32 = 30.0;
pub const DIRECTOR_PRENIGHT_S: f32 = 60.0;
pub const INTENSITY_DECAY: f32 = 6.0;

/// --- visuals --------------------------------------------------------------------
pub const DECAL_CAP: usize = 4000;
pub const GHOST_BLIP_S: f32 = 5.0;

/// --- audio ------------------------------------------------------------------------
pub const MAX_VOICES: usize = 24;
pub const AUDIO_HEAR_RADIUS: f32 = 260.0;
