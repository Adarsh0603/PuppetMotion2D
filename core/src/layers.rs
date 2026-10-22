//! Layered characters: a drawing made of several same-size images (one per
//! part, as drawing apps export them), stacked bottom to top.
//!
//! Each layer is split along the bones it moves with, using its own
//! transparency, so its edges and limb thickness are exact and nothing is
//! hidden: the far leg is its own drawing behind the body. Pieces are drawn
//! in layer order (the artist's), and within a layer in the bones' order.
//!
//! Which bones a layer moves with is guessed (`Layers::guess_bones`) and can
//! be set by hand per layer (by bone name, so it survives other edits).

use crate::geom::RectI;
use crate::mask::Mask;
use crate::skeleton::{NO_PART, Skeleton, distance_transform, masks_from_labels, part_labels_among};
use crate::sprites::{Sprite, build_sprites, in_paint_order};
use image::RgbaImage;

/// A pixel belongs to a layer when it's at least this opaque.
const SOLID: u8 = 16;
/// A bone counts as drawn on a layer when this share of it lies on the layer.
const COVERED: f32 = 0.5;

/// One drawing layer.
#[derive(Clone)]
pub struct Layer {
    pub name: String,
    pub image: RgbaImage,
}

/// The layers of one character, bottom to top, with what's worked out from
/// each image once (its shape, edge distances and box).
pub struct Layers {
    pub layers: Vec<Layer>,
    pub masks: Vec<Mask>,
    dts: Vec<Vec<f32>>,
    boxes: Vec<Option<RectI>>,
}

impl Layers {
    /// Layers bottom to top. They must all be the same size.
    pub fn new(layers: Vec<Layer>) -> Result<Layers, String> {
        let Some(first) = layers.first() else { return Err("no layers".into()) };
        let size = first.image.dimensions();
        if let Some(odd) = layers.iter().find(|l| l.image.dimensions() != size) {
            let (w, h) = odd.image.dimensions();
            return Err(format!(
                "\"{}\" is {w}×{h} but \"{}\" is {}×{}: every layer must be the same size (export them from the same canvas)",
                odd.name, first.name, size.0, size.1
            ));
        }
        let masks: Vec<Mask> = layers.iter().map(|l| layer_mask(&l.image)).collect();
        let dts = masks.iter().map(distance_transform).collect();
        let boxes = masks.iter().map(Mask::bbox).collect();
        Ok(Layers { layers, masks, dts, boxes })
    }

    pub fn size(&self) -> (u32, u32) {
        self.layers[0].image.dimensions()
    }

    /// All layers flattened into one image (each drawn over the ones below).
    pub fn composite(&self) -> RgbaImage {
        let (w, h) = self.size();
        let mut out = RgbaImage::new(w, h);
        for l in &self.layers {
            image::imageops::overlay(&mut out, &l.image, 0, 0);
        }
        out
    }

    /// Every pixel any layer covers: the character's shape.
    pub fn mask(&self) -> Mask {
        let (w, h) = self.size();
        let mut out = Mask::new(w, h);
        for m in &self.masks {
            for (o, &v) in out.data.iter_mut().zip(&m.data) {
                *o |= v;
            }
        }
        out
    }

    /// The bones each layer moves with: `chosen` names set by hand (an empty
    /// list = guess), otherwise `guess_bones`.
    pub fn bones(&self, skel: &Skeleton, chosen: &[Vec<String>]) -> Vec<Vec<usize>> {
        let guessed = self.guess_bones(skel);
        (0..self.layers.len())
            .map(|l| {
                let named: Vec<usize> = chosen
                    .get(l)
                    .into_iter()
                    .flatten()
                    .filter_map(|n| skel.parts.iter().position(|p| &p.name == n))
                    .collect();
                if named.is_empty() { guessed[l].clone() } else { named }
            })
            .collect()
    }

    /// A guess at the bones each layer moves with.
    ///
    /// Each bone goes to one layer: a layer whose name fits it ("far leg" for
    /// the far thigh, shin and foot; "back" counts as far, "front" as near),
    /// or else the smallest layer it lies on, since that's the most specific
    /// one (an arm lies on both the arm layer and the body layer). A layer
    /// that gets no bone (hair, a cape) moves with the bone it lies on most,
    /// or the one nearest to it.
    pub fn guess_bones(&self, skel: &Skeleton) -> Vec<Vec<usize>> {
        let n = self.layers.len();
        let mut out = vec![Vec::new(); n];
        if skel.parts.is_empty() {
            return out;
        }
        let area: Vec<usize> = self.masks.iter().map(Mask::count).collect();
        let cover: Vec<Vec<f32>> = (0..n).map(|l| (0..skel.parts.len()).map(|b| coverage(&self.masks[l], skel, b)).collect()).collect();
        let words: Vec<Vec<String>> = self.layers.iter().map(|l| layer_words(&l.name)).collect();
        for b in 0..skel.parts.len() {
            let by_name = (0..n).filter(|&l| name_fits(&words[l], &skel.parts[b].name)).max_by(|&x, &y| cover[x][b].total_cmp(&cover[y][b]));
            // Same size (two legs drawn alike): the layer with fewer bones so far.
            let by_shape = (0..n).filter(|&l| cover[l][b] >= COVERED).min_by_key(|&l| (area[l], out[l].len()));
            if let Some(l) = by_name.or(by_shape) {
                out[l].push(b);
            }
        }
        for l in 0..n {
            if out[l].is_empty() {
                out[l].push(fallback_bone(skel, &cover[l], self.boxes[l]));
            }
        }
        out
    }

    /// Which part each pixel shows, for tinting: the top layer's split wins.
    /// `bones` is per layer, as from `bones`.
    pub fn labels(&self, skel: &Skeleton, bones: &[Vec<usize>], out: &mut Vec<u16>) {
        let (w, h) = self.size();
        out.clear();
        out.resize((w * h) as usize, NO_PART);
        let mut layer_labels = Vec::new();
        for l in 0..self.layers.len() {
            self.split(l, skel, bones, &mut layer_labels);
            for (o, &v) in out.iter_mut().zip(&layer_labels) {
                if v != NO_PART {
                    *o = v;
                }
            }
        }
    }

    /// The pieces to draw, back to front: every layer bottom to top, each
    /// split along its bones (in their drawing order), and the layer each
    /// piece came from. `overlap` is the joint disc size, as in `build_sprites`.
    pub fn pieces(&self, skel: &Skeleton, bones: &[Vec<usize>], overlap: f32) -> (Vec<Sprite>, Vec<usize>) {
        let (mut out, mut from) = (Vec::new(), Vec::new());
        let mut labels = Vec::new();
        for l in 0..self.layers.len() {
            self.split(l, skel, bones, &mut labels);
            let (w, h) = self.size();
            let masks = masks_from_labels(&labels, w, h, skel.parts.len());
            let sprites = build_sprites(&self.layers[l].image, &self.masks[l], &self.dts[l], skel, &masks, overlap, false);
            let pieces = in_paint_order(sprites, skel);
            from.extend(std::iter::repeat_n(l, pieces.len()));
            out.extend(pieces);
        }
        (out, from)
    }

    /// Layer `l` split along its bones, as a part-per-pixel map.
    fn split(&self, l: usize, skel: &Skeleton, bones: &[Vec<usize>], labels: &mut Vec<u16>) {
        let mut among = vec![false; skel.parts.len()];
        for &b in bones.get(l).into_iter().flatten() {
            if let Some(a) = among.get_mut(b) {
                *a = true;
            }
        }
        part_labels_among(&self.masks[l], &self.dts[l], self.boxes[l], skel, &among, labels);
    }
}

/// The pixels of a layer image that are drawn on (not see-through).
pub fn layer_mask(img: &RgbaImage) -> Mask {
    let mut m = Mask::new(img.width(), img.height());
    for (i, px) in img.pixels().enumerate() {
        if px.0[3] >= SOLID {
            m.data[i] = 255;
        }
    }
    m
}

/// Share of bone `b` (sampled along its middle, away from the joints) that lies on `mask`.
fn coverage(mask: &Mask, skel: &Skeleton, b: usize) -> f32 {
    let (a, c) = skel.bone(b);
    const SAMPLES: usize = 9;
    let on = (1..=SAMPLES)
        .filter(|&i| {
            let t = 0.1 + 0.8 * (i - 1) as f32 / (SAMPLES - 1) as f32;
            let (x, y) = (a[0] + (c[0] - a[0]) * t, a[1] + (c[1] - a[1]) * t);
            x >= 0.0 && y >= 0.0 && mask.get(x as u32, y as u32)
        })
        .count();
    on as f32 / SAMPLES as f32
}

/// The bone a layer with no bone of its own moves with: the one it covers
/// most, else the one nearest the middle of its box.
fn fallback_bone(skel: &Skeleton, cover: &[f32], bbox: Option<RectI>) -> usize {
    let best = (0..cover.len()).max_by(|&x, &y| cover[x].total_cmp(&cover[y])).unwrap_or(0);
    if cover.get(best).is_some_and(|&c| c > 0.0) {
        return best;
    }
    let Some(r) = bbox else { return best };
    let mid = [r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0];
    (0..skel.parts.len())
        .min_by(|&x, &y| {
            let d = |b: usize| {
                let (a, c) = skel.bone(b);
                crate::skeleton::dist_to_segment(mid, a, c)
            };
            d(x).total_cmp(&d(y))
        })
        .unwrap_or(0)
}

/// A layer's name as lowercase words, with common names for parts and sides
/// turned into the words bone names use ("back leg" → far, leg, thigh, shin, foot).
fn layer_words(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    for w in name.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
        let w = w.trim_end_matches('s'); // "legs", "arms"
        let extra: &[&str] = match w {
            "back" | "behind" | "rear" => &["far"],
            "front" => &["near"],
            "leg" => &["thigh", "shin", "foot"],
            "arm" => &["upper", "forearm", "hand"],
            "body" | "chest" | "torso" | "trunk" => &["torso"],
            "head" | "face" | "hair" => &["head"],
            "foot" | "shoe" | "boot" | "feet" => &["foot"],
            "hand" => &["forearm", "hand"],
            _ => &[],
        };
        out.push(w.to_string());
        out.extend(extra.iter().map(|e| e.to_string()));
    }
    out
}

/// Whether a layer named with `words` is clearly where bone `bone` belongs:
/// it has the bone's side ("near"/"far", "left"/"right") if the bone has
/// one, and at least one of the bone's other words.
fn name_fits(words: &[String], bone: &str) -> bool {
    let has = |w: &str| words.iter().any(|x| x == w);
    let mut other = false;
    for w in bone.to_lowercase().split_whitespace() {
        if matches!(w, "near" | "far" | "left" | "right") {
            if !has(w) {
                return false;
            }
        } else if has(w) {
            other = true;
        }
    }
    other
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skeleton::{Joint, SkelPart};
    use image::Rgba;

    /// A side figure as three layers on a 100×200 canvas: a body block
    /// (x 30..70, y 20..110), and two legs (x 40..60, y 100..190) drawn in the
    /// same place, the far one under the body, the near one on top.
    fn figure(names: [&str; 3]) -> (Layers, Skeleton) {
        let layer = |name: &str, x: std::ops::Range<u32>, y: std::ops::Range<u32>, c: [u8; 3]| {
            let mut image = RgbaImage::new(100, 200);
            for yy in y.clone() {
                for xx in x.clone() {
                    image.put_pixel(xx, yy, Rgba([c[0], c[1], c[2], 255]));
                }
            }
            Layer { name: name.into(), image }
        };
        let layers = Layers::new(vec![
            layer(names[0], 40..60, 100..190, [80, 80, 160]),
            layer(names[1], 30..70, 20..110, [200, 60, 60]),
            layer(names[2], 40..60, 100..190, [60, 160, 60]),
        ])
        .unwrap();
        let j = |name: &str, x: f32, y: f32| Joint { name: name.into(), pos: [x, y] };
        let skel = Skeleton {
            joints: vec![j("hip", 50.0, 105.0), j("neck", 50.0, 25.0), j("near ankle", 50.0, 185.0), j("far ankle", 52.0, 185.0)],
            parts: vec![
                SkelPart { name: "torso".into(), pivot: 0, tip: 1, parent: None, limit: None, layer: 1 },
                SkelPart { name: "near leg".into(), pivot: 0, tip: 2, parent: Some(0), limit: None, layer: 2 },
                SkelPart { name: "far leg".into(), pivot: 0, tip: 3, parent: Some(0), limit: None, layer: 0 },
            ],
        };
        (layers, skel)
    }

    #[test]
    fn layers_must_match_in_size() {
        let a = Layer { name: "a".into(), image: RgbaImage::new(10, 10) };
        let b = Layer { name: "b".into(), image: RgbaImage::new(10, 12) };
        let err = Layers::new(vec![a, b]).err().unwrap();
        assert!(err.contains("same size"), "{err}");
    }

    #[test]
    fn names_decide_between_overlapping_limbs() {
        let (layers, skel) = figure(["back leg", "body", "front leg"]);
        assert_eq!(layers.guess_bones(&skel), vec![vec![2], vec![0], vec![1]]);
        // Each leg layer is all its own leg, even though both bones lie on both.
        let bones = layers.bones(&skel, &[]);
        let (pieces, from) = layers.pieces(&skel, &bones, 0.0);
        let parts: Vec<usize> = pieces.iter().map(|s| s.part).collect();
        assert_eq!(parts, vec![2, 0, 1], "back to front: far leg, body, near leg");
        assert_eq!(from, vec![0, 1, 2], "one piece from each layer");
        let far = &pieces[0];
        assert_eq!((far.image.width(), far.image.height()), (20, 90), "the whole far leg layer");
    }

    #[test]
    fn without_names_the_smallest_covering_layer_wins_and_choices_override() {
        let (layers, skel) = figure(["layer 1", "layer 2", "layer 3"]);
        let guess = layers.guess_bones(&skel);
        // Both leg bones lie on both (same-size) leg layers: one each. The body layer gets the torso.
        assert_eq!(guess[1], vec![0]);
        assert_eq!((guess[0].len(), guess[2].len()), (1, 1));
        // Choosing by hand: the bottom layer moves with the far leg.
        let chosen = vec![vec!["far leg".to_string()], vec![], vec!["near leg".to_string()]];
        assert_eq!(layers.bones(&skel, &chosen), vec![vec![2], vec![0], vec![1]]);
    }

    #[test]
    fn composite_and_labels_show_the_top_layer() {
        let (layers, skel) = figure(["back leg", "body", "front leg"]);
        let img = layers.composite();
        assert_eq!(img.get_pixel(50, 150).0, [60, 160, 60, 255], "front leg on top");
        assert_eq!(img.get_pixel(35, 50).0, [200, 60, 60, 255]);
        assert_eq!(img.get_pixel(5, 5).0[3], 0);
        let mut labels = Vec::new();
        layers.labels(&skel, &layers.bones(&skel, &[]), &mut labels);
        assert_eq!(labels[150 * 100 + 50], 1, "near leg");
        assert_eq!(labels[50 * 100 + 35], 0, "torso");
        assert_eq!(labels[5 * 100 + 5], NO_PART);
        assert_eq!(layers.mask().count(), 40 * 90 + 20 * 80);
    }

    #[test]
    fn a_layer_with_no_bone_follows_the_one_it_lies_on() {
        let (mut layers, skel) = figure(["back leg", "body", "front leg"]);
        // A hat on top of the body's upper end: no bone is "its own".
        let mut hat = RgbaImage::new(100, 200);
        for y in 15..30 {
            for x in 40..60 {
                hat.put_pixel(x, y, Rgba([10, 10, 10, 255]));
            }
        }
        let mut all = layers.layers.clone();
        all.push(Layer { name: "hat".into(), image: hat });
        layers = Layers::new(all).unwrap();
        assert_eq!(layers.guess_bones(&skel)[3], vec![0], "the hat moves with the torso");
    }
}
