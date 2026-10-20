//! "Fill behind": guessing what's hidden behind a part.
//!
//! A single flat drawing has no pixels behind an arm lying across the body,
//! so when the arm moves away there's a hole. Here each part is extended
//! under the parts drawn in front of it: over their pixels that are near the
//! part and inside its own box (so an arm along the body's side is covered,
//! a hand held far out is not), with colour spread in from the part's own
//! pixels around it. It's a soft blend, not real detail, but it reads as "more
//! hoodie" instead of a hole.

use crate::geom::RectI;
use crate::mask::Mask;
use image::RgbaImage;

/// Pixels to add behind one part: a box in the drawing, which pixels of it to
/// add, and their colours.
#[derive(Clone, Debug, PartialEq)]
pub struct FillPatch {
    pub rect: RectI,
    /// Row by row over `rect`: whether to add that pixel.
    pub add: Vec<bool>,
    /// Row by row over `rect`: the colour to give it (opaque).
    pub colors: Vec<[u8; 4]>,
}

impl FillPatch {
    pub fn get(&self, x: i32, y: i32) -> Option<[u8; 4]> {
        if !self.rect.contains(x, y) {
            return None;
        }
        let i = ((y - self.rect.y) * self.rect.w + (x - self.rect.x)) as usize;
        self.add[i].then(|| self.colors[i])
    }

    /// The tight box around the added pixels.
    pub fn bbox(&self) -> Option<RectI> {
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for (i, &on) in self.add.iter().enumerate() {
            if on {
                let (x, y) = (self.rect.x + i as i32 % self.rect.w, self.rect.y + i as i32 / self.rect.w);
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            }
        }
        (x0 < x1).then(|| RectI::from_corners(x0, y0, x1, y1))
    }
}

/// For each part (in `masks` order), what to add behind it, or `None`.
/// `order` is the painting order (back to front): a part is only filled under
/// parts drawn in front of it, never over parts behind it.
pub fn fill_behind(img: &RgbaImage, masks: &[Mask], order: &[usize]) -> Vec<Option<FillPatch>> {
    let Some(first) = masks.first() else { return Vec::new() };
    let (w, h) = (first.w as usize, first.h as usize);
    // One pass: each pixel's part, and each part's box.
    let mut labels = vec![u16::MAX; w * h];
    let mut boxes: Vec<Option<[usize; 4]>> = vec![None; masks.len()];
    for (p, m) in masks.iter().enumerate() {
        for (i, &v) in m.data.iter().enumerate() {
            if v != 0 {
                labels[i] = p as u16;
                let (x, y) = (i % w, i / w);
                let b = boxes[p].get_or_insert([x, y, x + 1, y + 1]);
                *b = [b[0].min(x), b[1].min(y), b[2].max(x + 1), b[3].max(y + 1)];
            }
        }
    }
    // Each part is independent, so they're filled side by side on the CPU cores.
    let rank: Vec<usize> = {
        let mut r = vec![usize::MAX; masks.len()];
        for (k, &p) in order.iter().enumerate() {
            r[p] = k;
        }
        r
    };
    let mut out = vec![None; masks.len()];
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..masks.len())
            .filter_map(|p| {
                let b = boxes[p]?;
                let (labels, rank) = (&labels, &rank);
                Some((p, s.spawn(move || part_fill(img, labels, w, h, rank, p, b))))
            })
            .collect();
        for (p, handle) in handles {
            out[p] = handle.join().ok().flatten();
        }
    });
    out
}

/// The fill behind part `p` (box `b` = x0, y0, x1, y1). `labels` is each
/// pixel's part (`u16::MAX` = none); `rank` each part's place in the painting
/// order, so "in front of p" means a higher rank.
fn part_fill(img: &RgbaImage, labels: &[u16], iw: usize, ih: usize, rank: &[usize], p: usize, b: [usize; 4]) -> Option<FillPatch> {
    let bb = RectI::from_corners(b[0] as i32, b[1] as i32, b[2] as i32, b[3] as i32);
    let in_front = |l: u16| (l as usize) < rank.len() && rank[l as usize] != usize::MAX && rank[l as usize] > rank[p];
    if !(0..rank.len()).any(|q| q != p && rank[q] != usize::MAX && rank[q] > rank[p]) {
        return None;
    }
    // Reach: a quarter of the part's size, enough to cover a limb lying along
    // a body's edge; things further out (a hand held away) are left alone.
    let r = (bb.w.max(bb.h) / 4).max(2);
    // The fill stays inside the part's own box (see `near` below), so that's
    // all the room needed, plus the reach for spreading its outline.
    let rect = RectI::new(bb.x - r, bb.y - r, bb.w + 2 * r, bb.h + 2 * r).clamp_to(iw as u32, ih as u32);
    let (w, h) = (rect.w as u32, rect.h as u32);
    // This part, and what's in front of it, cropped to the working box.
    let mut own_c = Mask::new(w, h);
    let mut front = Mask::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let l = labels[(y as usize + rect.y as usize) * iw + x as usize + rect.x as usize];
            if l as usize == p {
                own_c.set(x, y, true);
            } else if in_front(l) {
                front.set(x, y, true);
            }
        }
    }
    // Near the part (within reach) and inside its own box: the area its
    // outline plausibly continues under what's in front.
    let mut near = own_c.dilate(r as u32);
    for y in 0..h {
        for x in 0..w {
            if !bb.contains(x as i32 + rect.x, y as i32 + rect.y) {
                near.set(x, y, false);
            }
        }
    }
    // Only where a part in front of this one sits.
    let add: Vec<bool> = (0..(w * h) as usize).map(|i| near.data[i] != 0 && front.data[i] != 0 && own_c.data[i] == 0).collect();
    if !add.iter().any(|&a| a) {
        return None;
    }
    // Colours: the part's own pixels, spread into the gaps.
    let known: Vec<bool> = own_c.data.iter().map(|&v| v != 0).collect();
    let colors: Vec<[f32; 3]> = (0..(w * h) as usize)
        .map(|i| {
            let (x, y) = (i as u32 % w + rect.x as u32, i as u32 / w + rect.y as u32);
            let [r, g, b, _] = img.get_pixel(x, y).0;
            [r as f32, g as f32, b as f32]
        })
        .collect();
    let filled = push_pull(&colors, &known, w as usize, h as usize);
    let colors = filled.iter().map(|c| [c[0].round() as u8, c[1].round() as u8, c[2].round() as u8, 255]).collect();
    Some(FillPatch { rect, add, colors })
}

/// Fill every unknown pixel with a smooth blend of the known ones around it:
/// shrink the image by halves until everything is covered, then grow back,
/// using the coarser level only where the finer one had nothing.
fn push_pull(colors: &[[f32; 3]], known: &[bool], w: usize, h: usize) -> Vec<[f32; 3]> {
    if known.iter().all(|&k| k) || !known.iter().any(|&k| k) || (w <= 1 && h <= 1) {
        return colors.to_vec();
    }
    let (w2, h2) = (w.div_ceil(2), h.div_ceil(2));
    let mut c2 = vec![[0.0; 3]; w2 * h2];
    let mut k2 = vec![false; w2 * h2];
    for y2 in 0..h2 {
        for x2 in 0..w2 {
            let (mut sum, mut n) = ([0.0f32; 3], 0.0f32);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (x, y) = (x2 * 2 + dx, y2 * 2 + dy);
                if x < w && y < h && known[y * w + x] {
                    let c = colors[y * w + x];
                    sum = [sum[0] + c[0], sum[1] + c[1], sum[2] + c[2]];
                    n += 1.0;
                }
            }
            if n > 0.0 {
                c2[y2 * w2 + x2] = [sum[0] / n, sum[1] / n, sum[2] / n];
                k2[y2 * w2 + x2] = true;
            }
        }
    }
    let coarse = push_pull(&c2, &k2, w2, h2);
    // Smoothly enlarge the coarse level for the pixels that were unknown.
    let sample = |fx: f32, fy: f32| -> [f32; 3] {
        let (fx, fy) = (fx.clamp(0.0, (w2 - 1) as f32), fy.clamp(0.0, (h2 - 1) as f32));
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(w2 - 1), (y0 + 1).min(h2 - 1));
        let (u, v) = (fx - x0 as f32, fy - y0 as f32);
        let at = |x: usize, y: usize| coarse[y * w2 + x];
        let mut out = [0.0; 3];
        for (k, o) in out.iter_mut().enumerate() {
            let top = at(x0, y0)[k] + (at(x1, y0)[k] - at(x0, y0)[k]) * u;
            let bottom = at(x0, y1)[k] + (at(x1, y1)[k] - at(x0, y1)[k]) * u;
            *o = top + (bottom - top) * v;
        }
        out
    };
    (0..w * h)
        .map(|i| if known[i] { colors[i] } else { sample((i % w) as f32 / 2.0 - 0.25, (i / w) as f32 / 2.0 - 0.25) })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    const BODY: [u8; 4] = [40, 40, 60, 255];
    const ARM: [u8; 4] = [200, 150, 120, 255];

    /// A 60×100 "body" with an arm lying along its left edge and a hand
    /// sticking far out to the left. Parts: 0 body, 1 arm (in front), 2 hand
    /// (in front, outside the body).
    fn scene() -> (RgbaImage, Vec<Mask>) {
        let (w, h) = (200u32, 160u32);
        let mut img = RgbaImage::new(w, h);
        let mut masks = vec![Mask::new(w, h); 3];
        for y in 30..130 {
            for x in 100..160 {
                masks[0].set(x, y, true);
            }
        }
        for y in 40..110 {
            for x in 96..112 {
                masks[0].set(x, y, false);
                masks[1].set(x, y, true);
            }
        }
        for y in 100..112 {
            for x in 20..96 {
                masks[2].set(x, y, true);
            }
        }
        for (i, px) in img.pixels_mut().enumerate() {
            let (x, y) = (i as u32 % w, i as u32 / w);
            *px = if masks[0].get(x, y) { Rgba(BODY) } else if masks[1].get(x, y) || masks[2].get(x, y) { Rgba(ARM) } else { Rgba([0; 4]) };
        }
        (img, masks)
    }

    #[test]
    fn fills_under_the_arm_in_body_colours_but_not_out_to_the_hand() {
        let (img, masks) = scene();
        let fills = fill_behind(&img, &masks, &[0, 1, 2]);
        let body = fills[0].as_ref().expect("the body gets a fill");
        // Under the arm, inside the body's outline: filled with body colour.
        let c = body.get(105, 70).expect("hidden body area is filled");
        assert!(c.iter().zip(BODY).all(|(a, b)| a.abs_diff(b) < 8), "{c:?}");
        // Out where only the hand is: left alone.
        assert_eq!(body.get(40, 105), None);
        // Never over the body's own pixels or empty background.
        assert_eq!(body.get(130, 70), None);
        assert_eq!(body.get(105, 10), None);
        // Nothing is in front of the hand, so nothing to fill behind it.
        assert!(fills[2].is_none());
    }

    #[test]
    fn never_fills_over_a_part_drawn_behind() {
        let (img, masks) = scene();
        // Now the arm is behind the body: the body must not cover it.
        let fills = fill_behind(&img, &masks, &[1, 0, 2]);
        assert!(fills[0].as_ref().and_then(|f| f.get(105, 70)).is_none());
    }

    #[test]
    fn push_pull_blends_smoothly_between_known_colours() {
        // Known: left column black, right column white; the middle fills in between.
        let (w, h) = (9, 3);
        let known: Vec<bool> = (0..w * h).map(|i| i % w == 0 || i % w == w - 1).collect();
        let colors: Vec<[f32; 3]> = (0..w * h).map(|i| if i % w == w - 1 { [255.0; 3] } else { [0.0; 3] }).collect();
        let out = push_pull(&colors, &known, w, h);
        // Across the middle row: from black to white, never stepping back.
        let row: Vec<f32> = (0..w).map(|x| out[w + x][0]).collect();
        assert!(row.windows(2).all(|p| p[1] >= p[0] - 1e-3), "{row:?}");
        assert!(row[1..w - 1].iter().all(|&v| v > 0.0 && v < 255.0), "the gap is a blend: {row:?}");
    }
}
