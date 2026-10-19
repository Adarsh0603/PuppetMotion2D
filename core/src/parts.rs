//! Exclusive per-part masks from overlapping boxes.
//!
//! Each box gets a stand-in "bone" (its center line) until the template
//! supplies real joints. A pixel claimed by several boxes goes to the part
//! with the lowest cost: distance from that part's bone minus how deep the
//! pixel sits inside its box. So chained boxes (upper arm / forearm) split at
//! the middle of their overlap, and crossing parts split along their bones.

use crate::cutout::keep_dominant;
use crate::geom::RectI;
use crate::mask::Mask;
use crate::skeleton::dist_to_segment;

/// Which line through the box acts as the part's bone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Axis {
    /// Along the box's longer side.
    #[default]
    Auto,
    Horizontal,
    Vertical,
    /// Top-left to bottom-right.
    DiagDown,
    /// Bottom-left to top-right.
    DiagUp,
}

impl Axis {
    pub const ALL: [Axis; 5] = [Axis::Auto, Axis::Horizontal, Axis::Vertical, Axis::DiagDown, Axis::DiagUp];

    pub fn next(self) -> Axis {
        let i = Self::ALL.iter().position(|&a| a == self).unwrap();
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    pub fn label(self) -> &'static str {
        match self {
            Axis::Auto => "auto",
            Axis::Horizontal => "—",
            Axis::Vertical => "|",
            Axis::DiagDown => "↘",
            Axis::DiagUp => "↗",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PartBox {
    pub rect: RectI,
    pub axis: Axis,
}

impl PartBox {
    /// Bone endpoints in image coordinates (pixel edges, so a pixel's center is x + 0.5).
    pub fn bone(&self) -> ([f32; 2], [f32; 2]) {
        let r = &self.rect;
        let (x0, y0, x1, y1) = (r.x as f32, r.y as f32, r.right() as f32, r.bottom() as f32);
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let axis = match self.axis {
            Axis::Auto if r.w >= r.h => Axis::Horizontal,
            Axis::Auto => Axis::Vertical,
            a => a,
        };
        match axis {
            Axis::Horizontal => ([x0, cy], [x1, cy]),
            Axis::Vertical => ([cx, y0], [cx, y1]),
            Axis::DiagDown => ([x0, y0], [x1, y1]),
            Axis::DiagUp | Axis::Auto => ([x0, y1], [x1, y0]),
        }
    }

    /// Lower is a stronger claim on pixel (x, y).
    fn cost(&self, x: u32, y: u32) -> f32 {
        let p = [x as f32 + 0.5, y as f32 + 0.5];
        let (a, b) = self.bone();
        let r = &self.rect;
        let depth = (p[0] - r.x as f32)
            .min(r.right() as f32 - p[0])
            .min(p[1] - r.y as f32)
            .min(r.bottom() as f32 - p[1]);
        dist_to_segment(p, a, b) - depth
    }
}

pub struct PartMasks {
    /// One mask per box, in box order.
    pub masks: Vec<Mask>,
    /// Pixels that more than one box claimed before resolution.
    pub overlap: Mask,
}

/// Per-part masks for `boxes`. With `resolve`, every foreground pixel belongs
/// to at most one part; without it, overlapping boxes share pixels.
pub fn part_masks(fg: &Mask, boxes: &[PartBox], keep_ratio: f32, resolve: bool) -> PartMasks {
    let mut masks: Vec<Mask> = boxes.iter().map(|b| keep_dominant(fg, &b.rect, keep_ratio)).collect();
    let mut overlap = Mask::new(fg.w, fg.h);
    let rects: Vec<RectI> = boxes.iter().map(|b| b.rect.clamp_to(fg.w, fg.h)).collect();

    let mut claimants = Vec::new();
    for (i, r) in rects.iter().enumerate() {
        for y in r.y as u32..r.bottom() as u32 {
            for x in r.x as u32..r.right() as u32 {
                // Each overlap pixel is handled once, from the first box containing it.
                if rects[..i].iter().any(|o| o.contains(x as i32, y as i32)) {
                    continue;
                }
                claimants.clear();
                claimants.extend((i..masks.len()).filter(|&k| masks[k].get(x, y)));
                if claimants.len() < 2 {
                    continue;
                }
                overlap.set(x, y, true);
                if resolve {
                    let best = *claimants
                        .iter()
                        .min_by(|&&a, &&b| boxes[a].cost(x, y).total_cmp(&boxes[b].cost(x, y)))
                        .unwrap();
                    for &k in &claimants {
                        if k != best {
                            masks[k].set(x, y, false);
                        }
                    }
                }
            }
        }
    }
    if resolve {
        // Losing pixels can strand small fragments; drop them again.
        for (m, r) in masks.iter_mut().zip(&rects) {
            *m = keep_dominant(m, r, keep_ratio);
        }
    }
    PartMasks { masks, overlap }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Horizontal bar at y 45..55 from x 10..90, like an arm.
    fn bar() -> Mask {
        let mut m = Mask::new(100, 100);
        for y in 45..55 {
            for x in 10..90 {
                m.set(x, y, true);
            }
        }
        m
    }

    fn pb(x: i32, y: i32, w: i32, h: i32) -> PartBox {
        PartBox { rect: RectI::new(x, y, w, h), axis: Axis::Auto }
    }

    #[test]
    fn chained_boxes_split_mid_overlap() {
        let fg = bar();
        let boxes = [pb(0, 30, 60, 40), pb(40, 30, 60, 40)]; // overlap x 40..60
        let pm = part_masks(&fg, &boxes, 0.2, true);
        assert!(pm.masks[0].get(45, 50) && !pm.masks[1].get(45, 50));
        assert!(pm.masks[1].get(55, 50) && !pm.masks[0].get(55, 50));
        assert_eq!(pm.overlap.count(), 20 * 10);
        assert_eq!(pm.masks[0].count() + pm.masks[1].count(), fg.count(), "no pixel lost or doubled");
    }

    #[test]
    fn unresolved_boxes_share_pixels() {
        let pm = part_masks(&bar(), &[pb(0, 30, 60, 40), pb(40, 30, 60, 40)], 0.2, false);
        assert!(pm.masks[0].get(50, 50) && pm.masks[1].get(50, 50));
    }

    #[test]
    fn crossing_parts_follow_their_bones() {
        // A vertical "torso" bar crossed by the horizontal "arm" bar.
        let mut fg = bar();
        for y in 10..90 {
            for x in 45..55 {
                fg.set(x, y, true);
            }
        }
        let torso = pb(35, 0, 30, 100); // vertical bone at x = 50
        let arm = pb(0, 40, 100, 20); // horizontal bone at y = 50
        let pm = part_masks(&fg, &[torso, arm], 0.2, true);
        assert!(pm.masks[0].get(50, 20), "torso keeps its own pixels above the arm");
        assert!(pm.masks[1].get(40, 50), "arm keeps pixels just left of the torso");
        assert!(!pm.masks[0].get(40, 50));
    }

    #[test]
    fn diagonal_bone() {
        let b = PartBox { rect: RectI::new(0, 0, 10, 20), axis: Axis::DiagUp };
        assert_eq!(b.bone(), ([0.0, 20.0], [10.0, 0.0]));
        assert_eq!(Axis::DiagUp.next(), Axis::Auto);
    }
}
