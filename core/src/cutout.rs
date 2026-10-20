//! Classical cutout: finding the character by removing the background.

use crate::background::{BgModel, dist2, estimate_background};
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
