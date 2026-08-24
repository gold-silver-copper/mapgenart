//! Game world built from a generated map: coordinate helpers, walkability,
//! static physics colliders and the fog resource.

use super::fog::Fog;
use super::nav::{FlowField, NavGrid, greedy_rects};
use crate::generate::Generated;
use avian2d::prelude::*;
use bevy::prelude::*;

/// Collision layers.
#[derive(PhysicsLayer, Default)]
pub enum Layer {
    #[default]
    World,
    Unit,
    Enemy,
}

/// Everything the simulation needs about the loaded map.
#[derive(Resource)]
pub struct GameWorld {
    pub w: u32,
    pub h: u32,
    /// building pixels (block movement + sight)
    pub sight_blocked: Vec<bool>,
    /// building or water pixels (block movement)
    pub blocked: Vec<bool>,
    pub nav: NavGrid,
    /// nav cells of the largest connected walkable region
    pub main_region: Vec<bool>,
    pub flow: FlowField,
    pub fog: Fog,
    /// supply/loot points of interest (hospitals, supermarkets) in map px
    pub pois: Vec<(f32, f32, String)>,
    /// named places (cities/towns) in map px
    pub places: Vec<(f32, f32, String)>,
    /// per-pixel interior component id (u32::MAX outdoors)
    pub indoor_id: Vec<u32>,
    /// carved doors/windows (barricade targets)
    pub openings: Vec<super::buildings::Opening>,
    pub collider_count: usize,
}

impl GameWorld {
    /// map pixel (y down) → bevy world (y up, origin at map centre)
    pub fn to_world(&self, x: f32, y: f32) -> Vec2 {
        Vec2::new(x - self.w as f32 / 2.0, self.h as f32 / 2.0 - y)
    }

    /// bevy world → map pixel
    pub fn to_map(&self, v: Vec2) -> (f32, f32) {
        (v.x + self.w as f32 / 2.0, self.h as f32 / 2.0 - v.y)
    }

    pub fn walkable_world(&self, v: Vec2) -> bool {
        let (x, y) = self.to_map(v);
        let c = self.nav.cell_of(x, y);
        !self.nav.is_blocked(c.0, c.1)
    }

    /// Walkable AND connected to the open world (safe to spawn there).
    pub fn spawnable_cell(&self, c: (i32, i32)) -> bool {
        self.nav
            .idx(c.0, c.1)
            .map(|i| !self.nav.blocked[i] && self.main_region[i])
            .unwrap_or(false)
    }

    /// Nearest spawnable cell (walkable + main region).
    pub fn nearest_spawnable(&self, c: (i32, i32)) -> Option<(i32, i32)> {
        if self.spawnable_cell(c) {
            return Some(c);
        }
        for r in 1..=96i32 {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs().max(dy.abs()) == r && self.spawnable_cell((c.0 + dx, c.1 + dy)) {
                        return Some((c.0 + dx, c.1 + dy));
                    }
                }
            }
        }
        None
    }

    /// Pixel-precise obstacle probe (for steering / wall sliding).
    pub fn blocked_at_world(&self, v: Vec2) -> bool {
        let (x, y) = self.to_map(v);
        let (xi, yi) = (x as i32, y as i32);
        if xi < 0 || yi < 0 || xi >= self.w as i32 || yi >= self.h as i32 {
            return true;
        }
        self.blocked[(yi as u32 * self.w + xi as u32) as usize]
    }

    /// Steer `dir` around walls: if the probe ahead hits an obstacle, slide
    /// along it (increasingly rotated directions on both sides). Never
    /// returns zero for a nonzero input: in tight doorways it degrades to a
    /// short-probe or raw push at reduced speed — the physics engine stops
    /// actual penetration, and a slow grind beats freezing in place.
    pub fn slide(&self, pos: Vec2, dir: Vec2, look_ahead: f32) -> Vec2 {
        if dir == Vec2::ZERO || !self.blocked_at_world(pos + dir * look_ahead) {
            return dir;
        }
        for probe in [look_ahead, look_ahead * 0.5] {
            for deg in [35.0f32, 60.0, 85.0] {
                for sign in [1.0, -1.0] {
                    let a = deg.to_radians() * sign;
                    let (sin, cos) = a.sin_cos();
                    let d = Vec2::new(dir.x * cos - dir.y * sin, dir.x * sin + dir.y * cos);
                    if !self.blocked_at_world(pos + d * probe) {
                        return d * (probe / look_ahead).max(0.5);
                    }
                }
            }
        }
        dir * 0.4
    }
}

/// Build the world from a generated map and spawn the static colliders.
/// `sight` overrides the line-of-sight mask (walls minus windows/doors).
pub fn build_world(commands: &mut Commands, g: &Generated, sight: Option<Vec<bool>>) -> GameWorld {
    let (w, h) = (g.rendered.canvas.width, g.rendered.canvas.height);
    let blocked = g.rendered.blocked();
    let sight_blocked = sight.unwrap_or_else(|| g.rendered.building.clone());
    let nav = NavGrid::from_blocked(w, h, &blocked);
    let main_region = nav.main_region();
    let rects = greedy_rects(w, h, &blocked);
    let half = Vec2::new(w as f32 / 2.0, h as f32 / 2.0);
    for (x, y, rw, rh) in &rects {
        let centre = Vec2::new(
            *x as f32 + *rw as f32 / 2.0 - half.x,
            half.y - (*y as f32 + *rh as f32 / 2.0),
        );
        commands.spawn((
            RigidBody::Static,
            Collider::rectangle(*rw as f32, *rh as f32),
            CollisionLayers::new(Layer::World, [Layer::Unit, Layer::Enemy]),
            Transform::from_translation(centre.extend(0.0)),
            StaticWorld,
        ));
    }
    // map edges so nothing wanders off the world
    for (cx, cy, cw, ch) in [
        (0.0, half.y + 2.0, w as f32 + 8.0, 4.0),
        (0.0, -half.y - 2.0, w as f32 + 8.0, 4.0),
        (-half.x - 2.0, 0.0, 4.0, h as f32 + 8.0),
        (half.x + 2.0, 0.0, 4.0, h as f32 + 8.0),
    ] {
        commands.spawn((
            RigidBody::Static,
            Collider::rectangle(cw, ch),
            CollisionLayers::new(Layer::World, [Layer::Unit, Layer::Enemy]),
            Transform::from_translation(Vec3::new(cx, cy, 0.0)),
            StaticWorld,
        ));
    }
    let point_features = |kinds: &[crate::osm::Kind]| -> Vec<(f32, f32, String)> {
        g.features
            .iter()
            .filter_map(|f| match &f.geom {
                crate::osm::Geometry::Point(p) if kinds.contains(&f.kind) => {
                    let px = g.rendered.proj.project(*p);
                    Some((
                        px[0] as f32,
                        px[1] as f32,
                        f.name.clone().unwrap_or_default(),
                    ))
                }
                _ => None,
            })
            .filter(|(x, y, _)| *x >= 0.0 && *y >= 0.0 && *x < w as f32 && *y < h as f32)
            .collect()
    };
    let pois = point_features(&[crate::osm::Kind::Poi]);
    let places = point_features(&[crate::osm::Kind::City, crate::osm::Kind::Town]);
    log::info!("game world: {}x{w}px, {} static colliders", h, rects.len());
    GameWorld {
        w,
        h,
        sight_blocked,
        blocked,
        flow: FlowField::default(),
        main_region,
        nav,
        fog: Fog::new(w, h),
        pois,
        places,
        indoor_id: vec![u32::MAX; (w * h) as usize],
        openings: Vec::new(),
        collider_count: rects.len(),
    }
}

/// Marker for everything belonging to the loaded map (cleanup on restart).
#[derive(Component)]
pub struct StaticWorld;
