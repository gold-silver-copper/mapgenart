//! Pixel-art post-processing passes applied after rendering.

use crate::palette::{Palette, Rgba};
use crate::raster::{Canvas, layer};
use std::collections::HashMap;

/// Settings for the post-processing chain (all off except shoreline by default).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PostFx {
    pub smooth: u32,
    pub min_feature: u32,
    pub shoreline: bool,
    pub quantize: u32,
}

impl Default for PostFx {
    fn default() -> Self {
        PostFx {
            smooth: 0,
            min_feature: 0,
            shoreline: true,
            quantize: 0,
        }
    }
}

pub fn apply(canvas: &mut Canvas, fx: &PostFx, pal: &Palette) {
    for _ in 0..fx.smooth {
        smooth(canvas);
    }
    if fx.min_feature > 1 {
        min_feature(canvas, fx.min_feature as usize);
    }
    if fx.shoreline {
        shoreline(canvas, pal);
    }
    if fx.quantize > 0 {
        quantize(canvas, fx.quantize as usize, pal);
    }
}

fn is_water(tag: u8) -> bool {
    tag == layer::OCEAN
}

/// One pass of a 3×3 majority-vote filter. Line-layer pixels are neither
/// changed nor counted, so roads/borders stay crisp and do not bleed.
pub fn smooth(canvas: &mut Canvas) {
    let (w, h) = (canvas.width as i32, canvas.height as i32);
    let src_px = canvas.pixels.clone();
    let src_tags = canvas.tags.clone();
    let mut counts: Vec<(Rgba, u8, u8)> = Vec::with_capacity(9);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if src_tags[i] == layer::LINE {
                continue;
            }
            counts.clear();
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= w || ny >= h {
                        continue;
                    }
                    let j = (ny * w + nx) as usize;
                    if src_tags[j] == layer::LINE {
                        continue;
                    }
                    let c = src_px[j];
                    if let Some(e) = counts.iter_mut().find(|e| e.0 == c) {
                        e.1 += 1;
                    } else {
                        counts.push((c, 1, src_tags[j]));
                    }
                }
            }
            if let Some(best) = counts.iter().max_by_key(|e| e.1)
                && best.1 >= 5
            {
                canvas.pixels[i] = best.0;
                canvas.tags[i] = best.2;
            }
        }
    }
}

/// Remove 4-connected same-colour blobs smaller than `min_px`, filling them
/// with the most common neighbouring colour. Line pixels are left alone.
pub fn min_feature(canvas: &mut Canvas, min_px: usize) {
    let (w, h) = (canvas.width as i32, canvas.height as i32);
    let n = (w * h) as usize;
    let mut label = vec![u32::MAX; n];
    let mut blobs: Vec<Vec<usize>> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for start in 0..n {
        if label[start] != u32::MAX || canvas.tags[start] == layer::LINE {
            continue;
        }
        let id = blobs.len() as u32;
        let colour = canvas.pixels[start];
        let mut members = Vec::new();
        label[start] = id;
        stack.push(start);
        while let Some(i) = stack.pop() {
            members.push(i);
            let (x, y) = ((i as i32) % w, (i as i32) / w);
            for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                if nx < 0 || ny < 0 || nx >= w || ny >= h {
                    continue;
                }
                let j = (ny * w + nx) as usize;
                if label[j] == u32::MAX
                    && canvas.tags[j] != layer::LINE
                    && canvas.pixels[j] == colour
                {
                    label[j] = id;
                    stack.push(j);
                }
            }
        }
        blobs.push(members);
    }
    // process smallest first so merged blobs do not cascade unexpectedly
    let mut order: Vec<usize> = (0..blobs.len()).collect();
    order.sort_by_key(|b| blobs[*b].len());
    for b in order {
        if blobs[b].len() >= min_px {
            break;
        }
        let colour = canvas.pixels[blobs[b][0]];
        let mut votes: HashMap<Rgba, (usize, u8)> = HashMap::new();
        for &i in &blobs[b] {
            let (x, y) = ((i as i32) % w, (i as i32) / w);
            for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                if nx < 0 || ny < 0 || nx >= w || ny >= h {
                    continue;
                }
                let j = (ny * w + nx) as usize;
                if canvas.pixels[j] != colour && canvas.tags[j] != layer::LINE {
                    let e = votes.entry(canvas.pixels[j]).or_insert((0, canvas.tags[j]));
                    e.0 += 1;
                }
            }
        }
        if let Some((c, (_, tag))) = votes.into_iter().max_by_key(|(_, (n, _))| *n) {
            for &i in &blobs[b] {
                canvas.pixels[i] = c;
                canvas.tags[i] = tag;
            }
        }
    }
}

/// Darken the 1px ring of land pixels that touch water.
pub fn shoreline(canvas: &mut Canvas, pal: &Palette) {
    let (w, h) = (canvas.width as i32, canvas.height as i32);
    let tags = canvas.tags.clone();
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if is_water(tags[i]) || tags[i] == layer::LINE {
                continue;
            }
            let touches_water = [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]
                .into_iter()
                .any(|(nx, ny)| {
                    nx >= 0 && ny >= 0 && nx < w && ny < h && is_water(tags[(ny * w + nx) as usize])
                });
            if touches_water {
                canvas.pixels[i] = pal.shoreline;
                canvas.tags[i] = layer::SHORE;
            }
        }
    }
}

fn dist2(a: Rgba, b: Rgba) -> u32 {
    (0..3)
        .map(|k| (a[k] as i32 - b[k] as i32).pow(2) as u32)
        .sum()
}

/// Snap every pixel to the nearest of the `n` most frequent colours that are
/// also palette colours (falls back to the most frequent image colours when
/// fewer than `n` palette colours are present).
pub fn quantize(canvas: &mut Canvas, n: usize, pal: &Palette) {
    let mut freq: HashMap<Rgba, usize> = HashMap::new();
    for p in &canvas.pixels {
        *freq.entry(*p).or_insert(0) += 1;
    }
    let palette_set: Vec<Rgba> = pal.colours();
    let mut candidates: Vec<(Rgba, usize)> = freq.iter().map(|(c, n)| (*c, *n)).collect();
    // palette colours first (by frequency), then everything else by frequency
    candidates.sort_by(|a, b| {
        let pa = palette_set.contains(&a.0);
        let pb = palette_set.contains(&b.0);
        pb.cmp(&pa).then(b.1.cmp(&a.1))
    });
    let keep: Vec<Rgba> = candidates.iter().take(n.max(1)).map(|c| c.0).collect();
    if keep.is_empty() {
        return;
    }
    let mut cache: HashMap<Rgba, Rgba> = HashMap::new();
    for p in canvas.pixels.iter_mut() {
        let q = *cache
            .entry(*p)
            .or_insert_with(|| *keep.iter().min_by_key(|k| dist2(*p, **k)).unwrap());
        *p = q;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Rgba = [10, 10, 10, 255];
    const B: Rgba = [200, 200, 200, 255];

    fn canvas_with_speck() -> Canvas {
        let mut c = Canvas::new(5, 5, A);
        c.layer = layer::COVER;
        c.set(2, 2, B);
        c
    }

    #[test]
    fn smooth_removes_single_speck_but_not_lines() {
        let mut c = canvas_with_speck();
        smooth(&mut c);
        assert_eq!(c.get(2, 2).unwrap(), A);
        let mut c = canvas_with_speck();
        c.layer = layer::LINE;
        c.set(2, 2, B);
        smooth(&mut c);
        assert_eq!(c.get(2, 2).unwrap(), B);
    }

    #[test]
    fn min_feature_fills_small_blob() {
        let mut c = Canvas::new(6, 6, A);
        c.layer = layer::COVER;
        c.set(1, 1, B);
        c.set(2, 1, B);
        min_feature(&mut c, 3);
        assert_eq!(c.get(1, 1).unwrap(), A);
        assert_eq!(c.get(2, 1).unwrap(), A);
    }

    #[test]
    fn shoreline_marks_land_next_to_water() {
        let pal = Palette::default();
        let mut c = Canvas::new(4, 1, pal.land);
        c.layer = layer::OCEAN;
        c.set(0, 0, pal.ocean);
        shoreline(&mut c, &pal);
        assert_eq!(c.get(1, 0).unwrap(), pal.shoreline);
        assert_eq!(c.get(2, 0).unwrap(), pal.land);
        assert_eq!(c.get(0, 0).unwrap(), pal.ocean);
    }

    #[test]
    fn quantize_snaps_to_two_colours() {
        let pal = Palette::default();
        let mut c = Canvas::new(4, 1, pal.land);
        c.set(0, 0, pal.ocean);
        c.set(1, 0, [pal.ocean[0] + 3, pal.ocean[1], pal.ocean[2], 255]); // near-ocean
        quantize(&mut c, 2, &pal);
        assert_eq!(c.get(1, 0).unwrap(), pal.ocean);
        assert_eq!(c.get(3, 0).unwrap(), pal.land);
    }
}
