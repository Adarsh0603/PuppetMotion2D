//! Background estimation: either the image's own alpha, or flat colors
//! sampled from the corners.

use image::RgbaImage;

#[derive(Clone, Debug, PartialEq)]
pub enum BgModel {
    /// The image has real transparency; use alpha as the foreground signal.
    Alpha,
    /// One to four background colors sampled from the corners.
    Colors(Vec<[u8; 3]>),
}

/// Fraction of pixels that must be non-opaque before alpha is trusted.
const ALPHA_FRACTION: f64 = 0.01;

pub fn has_transparency(img: &RgbaImage) -> bool {
    let total = img.width() as u64 * img.height() as u64;
    let clear = img.pixels().filter(|p| p.0[3] < 250).count() as u64;
    total > 0 && clear as f64 > total as f64 * ALPHA_FRACTION
}

#[inline]
pub fn dist2(a: [u8; 3], b: [u8; 3]) -> u32 {
    let d = |i: usize| (a[i] as i32 - b[i] as i32).pow(2) as u32;
    d(0) + d(1) + d(2)
}

/// Per-channel median of a `patch`×`patch` square at (x0, y0), clamped to the image.
pub fn patch_median(img: &RgbaImage, x0: u32, y0: u32, patch: u32) -> [u8; 3] {
    let x1 = (x0 + patch).min(img.width());
    let y1 = (y0 + patch).min(img.height());
    let mut ch: [Vec<u8>; 3] = Default::default();
    for y in y0..y1 {
        for x in x0..x1 {
            let p = img.get_pixel(x, y).0;
            for c in 0..3 {
                ch[c].push(p[c]);
            }
        }
    }
    ch.map(|mut v| {
        v.sort_unstable();
        v[v.len() / 2]
    })
}

pub fn estimate_background(img: &RgbaImage, patch: u32, threshold: f32) -> BgModel {
    if has_transparency(img) {
        return BgModel::Alpha;
    }
    let (w, h) = (img.width(), img.height());
    let patch = patch.clamp(1, w.min(h).max(1));
    let corners = [(0, 0), (w - patch, 0), (0, h - patch), (w - patch, h - patch)];
    let t2 = (threshold * threshold) as u32;
    let mut colors: Vec<[u8; 3]> = Vec::new();
    for (x, y) in corners {
        let c = patch_median(img, x, y, patch);
        if !colors.iter().any(|&k| dist2(k, c) <= t2) {
            colors.push(c);
        }
    }
    BgModel::Colors(colors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn flat_white_gives_one_color() {
        let img = RgbaImage::from_pixel(40, 30, Rgba([250, 250, 248, 255]));
        assert_eq!(estimate_background(&img, 8, 40.0), BgModel::Colors(vec![[250, 250, 248]]));
    }

    #[test]
    fn transparent_image_uses_alpha() {
        let mut img = RgbaImage::from_pixel(10, 10, Rgba([0, 0, 0, 0]));
        img.put_pixel(5, 5, Rgba([255, 0, 0, 255]));
        assert_eq!(estimate_background(&img, 8, 40.0), BgModel::Alpha);
    }
}
