//! Ammo / meds / scrap: the squad pool, and scavenging building interiors.

use super::logic::SimRng;
use super::tuning::*;
use super::units::{Dormant, Enemy, Soldier};
use super::world::GameWorld;
use bevy::prelude::*;

#[derive(Resource)]
pub struct Stockpile {
    pub ammo: f32,
    pub meds: f32,
    pub scrap: f32,
}

impl Default for Stockpile {
    fn default() -> Self {
        Stockpile {
            ammo: START_AMMO,
            meds: START_MEDS,
            scrap: START_SCRAP,
        }
    }
}

/// One lootable building interior.
#[derive(Debug, Clone)]
pub struct LootSite {
    /// interior component id (indexes `GameWorld::indoor_id`), or u32::MAX
    /// for free-standing sites (strategic maps without buildings)
    pub interior: u32,
    pub centre: Vec2,
    pub total: f32,
    pub remaining: f32,
}

#[derive(Resource, Default)]
pub struct LootSites(pub Vec<LootSite>);

pub struct EconomyPlugin;

impl Plugin for EconomyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Stockpile>()
            .init_resource::<LootSites>()
            .add_systems(Update, scavenge.run_if(resource_exists::<GameWorld>));
    }
}

/// Build loot sites from carved interiors (or bare POIs on building-less
/// maps). POI interiors (pharmacy/supermarket/hospital nearby) are richer.
pub fn build_sites(world: &GameWorld, interiors: &[super::buildings::Interior]) -> LootSites {
    let mut sites = Vec::new();
    if interiors.is_empty() {
        for (x, y, _) in &world.pois {
            sites.push(LootSite {
                interior: u32::MAX,
                centre: world.to_world(*x, *y),
                total: LOOT_BASE * LOOT_POI_MULT * 2.0,
                remaining: LOOT_BASE * LOOT_POI_MULT * 2.0,
            });
        }
    } else {
        for (id, interior) in interiors.iter().enumerate() {
            let mut value = LOOT_BASE + interior.pixels as f32 / 100.0 * LOOT_PER_100PX;
            let near_poi = world.pois.iter().any(|(px, py, _)| {
                (px - interior.centroid.0).abs() + (py - interior.centroid.1).abs() < 24.0
            });
            if near_poi {
                value *= LOOT_POI_MULT;
            }
            value = value.min(LOOT_VALUE_CAP);
            sites.push(LootSite {
                interior: id as u32,
                centre: world.to_world(interior.centroid.0, interior.centroid.1),
                total: value,
                remaining: value,
            });
        }
    }
    LootSites(sites)
}

/// A soldier standing in an unlooted interior (or at a bare site) drains it
/// into the squad pool. Sleepers indoors make this push-your-luck.
fn scavenge(
    time: Res<Time>,
    world: Res<GameWorld>,
    mut sites: ResMut<LootSites>,
    mut stock: ResMut<Stockpile>,
    mut rng: ResMut<SimRng>,
    soldiers: Query<&Transform, With<Soldier>>,
    _sleepers: Query<(), (With<Enemy>, With<Dormant>)>,
) {
    let dt = time.delta_secs();
    for tf in &soldiers {
        let pos = tf.translation.truncate();
        let (mx, my) = world.to_map(pos);
        let (xi, yi) = (mx as i32, my as i32);
        let interior = if xi >= 0 && yi >= 0 && xi < world.w as i32 && yi < world.h as i32 {
            world.indoor_id[(yi as u32 * world.w + xi as u32) as usize]
        } else {
            u32::MAX
        };
        for site in sites.0.iter_mut() {
            let here = if site.interior != u32::MAX {
                site.interior == interior
            } else {
                site.centre.distance(pos) < 7.0
            };
            if !here || site.remaining <= 0.0 {
                continue;
            }
            let take = (SCAVENGE_RATE * dt).min(site.remaining);
            site.remaining -= take;
            // 60/20/20 with jitter
            let r = rng.f32();
            if r < 0.6 {
                stock.ammo += take;
            } else if r < 0.8 {
                stock.meds += take * 0.5;
            } else {
                stock.scrap += take * 0.5;
            }
            break;
        }
    }
}

/// Fraction looted (for the HUD / current site ring).
pub fn site_progress(sites: &LootSites, interior: u32) -> Option<f32> {
    sites
        .0
        .iter()
        .find(|s| s.interior == interior)
        .map(|s| 1.0 - s.remaining / s.total.max(1.0))
}
