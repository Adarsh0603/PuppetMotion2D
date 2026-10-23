//! Classical cutout: finding the character by removing the background.

use crate::background::{BgModel, dist2, estimate_background, has_transparency, patch_median};
use crate::components::{fill_holes, flood_from_border, remove_small};
use crate::mask::Mask;
use image::RgbaImage;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Params {
    /// Max RGB distance from a background color for a pixel to count as background.
    pub threshold: f32,
    /// Side length of the square sampled at each corner.
    pub corner_patch: u32,
    /// Foreground specks smaller than this (pixels) are dropped.
    pub min_area: usize,
    /// Seal outline breaks up to about twice this many pixels wide, so the
    /// background can't flood inside a sketchy outline. 0 disables it.
    pub gap_close: u32,
}

impl Default for Params {
    fn default() -> Self {
        Self { threshold: 40.0, corner_patch: 8, min_area: 64, gap_close: 3 }
    }
}

/// Cutout settings picked for this image, so most drawings need no tuning.
/// - Tolerance grows with the background's grain (paper, JPEG noise).
/// - Gap closing is the smallest whose cutout is about as big as with the
///   widest closing tried: a leaking outline loses its whole inside, while
///   closing more than needed only fills narrow pockets (armpits).
pub fn auto_params(img: &RgbaImage) -> Params {
    let mut p = Params::default();
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 || has_transparency(img) {
        return p;
    }
    p.threshold = (40.0 + 2.0 * background_grain(img, p.corner_patch)).min(80.0);
    // Search on a small copy, since each try finds the whole cutout again.
    let scale = (w.min(h) / SEARCH_SIDE).max(1);
    let small = if scale > 1 {
        image::imageops::resize(img, w / scale, h / scale, image::imageops::FilterType::Triangle)
    } else {
        img.clone()
    };
    let min_area = (p.min_area / (scale * scale) as usize).max(1);
    let area = |gap: u32| foreground_mask(&small, &Params { gap_close: gap, min_area, ..p }).count() as f32;
    let sealed = area(MAX_GAP);
    let gap = (0..MAX_GAP).find(|&gap| area(gap) >= (1.0 - SEALED_SHARE) * sealed).unwrap_or(MAX_GAP);
    // Shrinking blurs hairline breaks shut, so a scaled-up gap gets one extra step.
    p.gap_close = if scale > 1 { (gap + 1) * scale } else { gap };
    p
}

/// `auto_params` searches on a copy about this many pixels on its short side.
const SEARCH_SIDE: u32 = 400;
/// The widest gap closing `auto_params` tries on that copy.
const MAX_GAP: u32 = 8;
/// How much smaller than the widest closing's cutout still counts as sealed.
const SEALED_SHARE: f32 = 0.05;

/// How far typical corner pixels stray from their corner's median colour
/// (the 90th percentile of the RGB distance).
fn background_grain(img: &RgbaImage, patch: u32) -> f32 {
    let (w, h) = img.dimensions();
    let patch = patch.clamp(1, w.min(h));
    let mut d: Vec<f32> = Vec::new();
    for (x0, y0) in [(0, 0), (w - patch, 0), (0, h - patch), (w - patch, h - patch)] {
        let m = patch_median(img, x0, y0, patch);
        for y in y0..y0 + patch {
            for x in x0..x0 + patch {
                let p = img.get_pixel(x, y).0;
                d.push((dist2(m, [p[0], p[1], p[2]]) as f32).sqrt());
            }
        }
    }
    d.sort_unstable_by(f32::total_cmp);
    // Most corners are pure background; the top tenth may be a stray mark.
    d[d.len() * 9 / 10]
}

/// Whole-image foreground mask.
pub fn foreground_mask(img: &RgbaImage, p: &Params) -> Mask {
    let (w, h) = img.dimensions();
    let mut fg = Mask::new(w, h);
    if w == 0 || h == 0 {
        return fg;
    }
    match estimate_background(img, p.corner_patch, p.threshold) {
        BgModel::Alpha => {
            for (o, px) in fg.data.iter_mut().zip(img.pixels()) {
                *o = if px.0[3] > 128 { 255 } else { 0 };
            }
            // Antialiasing and stray transparent pixels leave pinholes.
            fg = fill_holes(&fg, p.min_area);
        }
        BgModel::Colors(colors) => {
            let t2 = (p.threshold * p.threshold) as u32;
            let mut bg_like = Mask::new(w, h);
            for (o, px) in bg_like.data.iter_mut().zip(img.pixels()) {
                let c = [px.0[0], px.0[1], px.0[2]];
                if colors.iter().any(|&k| dist2(k, c) <= t2) {
                    *o = 255;
                }
            }
            // Only background-colored pixels connected to the border are background,
            // so enclosed light areas (eyes, white shirts) stay foreground.
            let bg = if p.gap_close == 0 {
                flood_from_border(&bg_like)
            } else {
                // Thicken the ink to seal small breaks, flood, then grow the
                // background back over the pixels the thickening covered. The
                // regrowth stops short of the far side of a sealed break.
                let ink = bg_like.inverted().dilate(p.gap_close);
                let seed = flood_from_border(&bg_like.and_not(&ink));
                seed.dilate(p.gap_close).and(&bg_like)
            };
            for (o, &b) in fg.data.iter_mut().zip(&bg.data) {
                *o = 255 - b;
            }
        }
    }
    remove_small(&fg, p.min_area)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::RectI;
    use image::Rgba;

    const WHITE: Rgba<u8> = Rgba([255, 255, 255, 255]);
    const INK: Rgba<u8> = Rgba([30, 20, 60, 255]);

    fn draw_disc(img: &mut RgbaImage, cx: i32, cy: i32, r: i32, c: Rgba<u8>) {
        for y in 0..img.height() as i32 {
            for x in 0..img.width() as i32 {
                if (x - cx).pow(2) + (y - cy).pow(2) <= r * r {
                    img.put_pixel(x as u32, y as u32, c);
                }
            }
        }
    }

    #[test]
    fn disc_on_white() {
        let mut img = RgbaImage::from_pixel(100, 100, WHITE);
        draw_disc(&mut img, 50, 50, 20, INK);
        let fg = foreground_mask(&img, &Params::default());
        assert!(fg.get(50, 50));
        assert!(!fg.get(5, 5));
        assert_eq!(fg.bbox(), Some(RectI::new(30, 30, 41, 41)));
    }

    #[test]
    fn enclosed_white_stays_foreground() {
        let mut img = RgbaImage::from_pixel(100, 100, WHITE);
        draw_disc(&mut img, 50, 50, 30, INK);
        draw_disc(&mut img, 50, 50, 20, WHITE); // white "shirt" inside an outline
        let fg = foreground_mask(&img, &Params::default());
        assert!(fg.get(50, 50), "interior white must not leak to background");
    }

    #[test]
    fn small_outline_gap_is_sealed() {
        // 6 px thick ring outline with a 5 px wide break at the top.
        let mut img = RgbaImage::from_pixel(100, 100, WHITE);
        for (x, y, px) in img.enumerate_pixels_mut() {
            let d = ((x as f32 - 50.0).powi(2) + (y as f32 - 50.0).powi(2)).sqrt();
            let in_gap = y < 50 && (x as i32 - 50).abs() <= 2;
            if (27.0..=33.0).contains(&d) && !in_gap {
                *px = INK;
            }
        }
        let open = Params { gap_close: 0, ..Params::default() };
        assert!(!foreground_mask(&img, &open).get(50, 50), "without closing, background leaks in");
        let fg = foreground_mask(&img, &Params::default());
        assert!(fg.get(50, 50), "interior is foreground once the gap is sealed");
        assert!(!fg.get(50, 10), "background outside the gap stays background");
        assert!(!fg.get(5, 5));
    }

    #[test]
    fn auto_params_seal_a_gap_and_leave_clean_outlines_alone() {
        let ring = |gap: bool| {
            let mut img = RgbaImage::from_pixel(100, 100, WHITE);
            for (x, y, px) in img.enumerate_pixels_mut() {
                let d = ((x as f32 - 50.0).powi(2) + (y as f32 - 50.0).powi(2)).sqrt();
                let in_gap = gap && y < 50 && (x as i32 - 50).abs() <= 2;
                if (27.0..=33.0).contains(&d) && !in_gap {
                    *px = INK;
                }
            }
            img
        };
        let p = auto_params(&ring(true));
        assert!(p.gap_close > 0);
        assert!(foreground_mask(&ring(true), &p).get(50, 50), "the inside stays the character");
        assert_eq!(auto_params(&ring(false)).gap_close, 0, "nothing to seal");
    }

    #[test]
    fn auto_params_raise_tolerance_on_grainy_paper() {
        let mut img = RgbaImage::from_pixel(100, 100, Rgba([235, 232, 225, 255]));
        for (i, px) in img.pixels_mut().enumerate() {
            let n = (i * 7919 % 31) as u8;
            px.0[0] -= n;
            px.0[1] -= n;
            px.0[2] -= n;
        }
        draw_disc(&mut img, 50, 50, 20, INK);
        let p = auto_params(&img);
        assert!(p.threshold > 40.0);
        let fg = foreground_mask(&img, &p);
        assert!(fg.get(50, 50) && !fg.get(5, 5));
        assert_eq!(auto_params(&RgbaImage::from_pixel(50, 50, WHITE)).threshold, 40.0, "flat paper keeps the default");
    }

    #[test]
    fn transparent_png_uses_alpha() {
        let mut img = RgbaImage::from_pixel(80, 80, Rgba([255, 255, 255, 0]));
        draw_disc(&mut img, 40, 40, 15, Rgba([255, 255, 255, 255])); // white on transparent
        let fg = foreground_mask(&img, &Params::default());
        assert!(fg.get(40, 40));
        assert!(!fg.get(2, 2));
    }

    #[test]
    fn noisy_background_corner() {
        let mut img = RgbaImage::from_pixel(100, 100, Rgba([240, 238, 230, 255]));
        // Paper grain plus a speck of dirt inside one corner patch.
        for (i, px) in img.pixels_mut().enumerate() {
            let n = (i * 7919 % 13) as u8;
            px.0[0] -= n;
            px.0[1] -= n;
        }
        img.put_pixel(1, 1, Rgba([0, 0, 0, 255]));
        draw_disc(&mut img, 50, 50, 20, INK);
        let fg = foreground_mask(&img, &Params::default());
        assert!(fg.get(50, 50));
        assert!(!fg.get(1, 1), "speck is removed by min_area");
        assert!(fg.count() < 1400);
    }
}
