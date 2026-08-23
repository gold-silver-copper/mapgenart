//! Land polygons from GeoJSON (Natural Earth `ne_10m_land`, or the
//! osmdata.openstreetmap.de land polygons converted to GeoJSON). Used as the
//! land/ocean base for bboxes without usable coastline data.

use anyhow::{Context, Result};
use serde_json::Value;
use std::path::Path;

/// Polygons as lists of rings (outer first, holes after) in lon/lat.
pub type LandPolygons = Vec<Vec<Vec<[f64; 2]>>>;

pub fn load(path: &Path) -> Result<LandPolygons> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let v: Value = serde_json::from_str(&text).context("parsing GeoJSON")?;
    let mut out = Vec::new();
    collect(&v, &mut out);
    log::info!("loaded {} land polygons from {}", out.len(), path.display());
    Ok(out)
}

fn ring(v: &Value) -> Vec<[f64; 2]> {
    v.as_array()
        .map(|pts| {
            pts.iter()
                .filter_map(|p| {
                    let a = p.as_array()?;
                    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
                })
                .collect()
        })
        .unwrap_or_default()
}

fn collect(v: &Value, out: &mut LandPolygons) {
    match v.get("type").and_then(Value::as_str) {
        Some("FeatureCollection") => {
            for f in v
                .get("features")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                collect(f, out);
            }
        }
        Some("Feature") => {
            if let Some(g) = v.get("geometry") {
                collect(g, out);
            }
        }
        Some("GeometryCollection") => {
            for g in v
                .get("geometries")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                collect(g, out);
            }
        }
        Some("Polygon") => {
            if let Some(rings) = v.get("coordinates").and_then(Value::as_array) {
                let poly: Vec<_> = rings.iter().map(ring).filter(|r| r.len() >= 4).collect();
                if !poly.is_empty() {
                    out.push(poly);
                }
            }
        }
        Some("MultiPolygon") => {
            if let Some(polys) = v.get("coordinates").and_then(Value::as_array) {
                for rings in polys {
                    if let Some(rings) = rings.as_array() {
                        let poly: Vec<_> =
                            rings.iter().map(ring).filter(|r| r.len() >= 4).collect();
                        if !poly.is_empty() {
                            out.push(poly);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_feature_collection() {
        let v: Value = serde_json::from_str(
            r#"{"type":"FeatureCollection","features":[
                {"type":"Feature","geometry":{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]]]}},
                {"type":"Feature","geometry":{"type":"MultiPolygon","coordinates":[[[[2,2],[3,2],[3,3],[2,2]]]]}}
            ]}"#,
        )
        .unwrap();
        let mut out = Vec::new();
        collect(&v, &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0][0].len(), 4);
    }
}
