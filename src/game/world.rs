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
    pub flow: FlowField,
    pub fog: Fog,
    /// supply-drop points of interest (hospitals, supermarkets) in map px
    pub pois: Vec<(f32, f32, String)>,
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
}

/// Build the world from a generated map and spawn the static colliders.
pub fn build_world(commands: &mut Commands, g: &Generated) -> GameWorld {
    let (w, h) = (g.rendered.canvas.width, g.rendered.canvas.height);
    let blocked = g.rendered.blocked();
    let sight_blocked = g.rendered.building.clone();
    let nav = NavGrid::from_blocked(w, h, &blocked);
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
    let pois = g
        .features
        .iter()
        .filter_map(|f| match (&f.kind, &f.geom) {
            (crate::osm::Kind::Poi, crate::osm::Geometry::Point(p)) => {
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
        .collect();
    log::info!("game world: {}x{w}px, {} static colliders", h, rects.len());
    GameWorld {
        w,
        h,
        sight_blocked,
        blocked,
        flow: FlowField::default(),
        nav,
        fog: Fog::new(w, h),
        pois,
        collider_count: rects.len(),
    }
}

/// Marker for everything belonging to the loaded map (cleanup on restart).
#[derive(Component)]
pub struct StaticWorld;
