//! Draws the icon Windows shows for .pm2d project files: a page with a folded
//! corner, the app icon on it, and a "PM2D" label. Run from the repository root:
//!
//!     cargo run --release -p puppetmotion --example make_file_icon
//!
//! It writes `app/assets/pm2d.ico` (16 to 256 px, each size drawn on its own
//! so small ones stay crisp; the label is left off below 32 px).
use image::{Rgba, RgbaImage, imageops};

const SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

fn len(x: f32, y: f32) -> f32 {
    (x * x + y * y).sqrt()
}

/// Rounded box centred at (cx, cy) with half-size (hw, hh) and corner radius r.
fn rbox(p: [f32; 2], c: [f32; 2], hw: f32, hh: f32, r: f32) -> f32 {
    let q = [(p[0] - c[0]).abs() - hw + r, (p[1] - c[1]).abs() - hh + r];
    len(q[0].max(0.0), q[1].max(0.0)) + q[0].max(q[1]).min(0.0) - r
}

fn segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (pa, ba) = ([p[0] - a[0], p[1] - a[1]], [b[0] - a[0], b[1] - a[1]]);
    let h = ((pa[0] * ba[0] + pa[1] * ba[1]) / (ba[0] * ba[0] + ba[1] * ba[1])).clamp(0.0, 1.0);
    len(pa[0] - ba[0] * h, pa[1] - ba[1] * h)
}

/// Strokes of the letters P, M, 2, D on a grid 7 units tall.
fn letters() -> Vec<([f32; 2], [f32; 2])> {
    let glyphs: [(f32, &[[f32; 4]]); 4] = [
        (4.0, &[[0., 0., 0., 7.], [0., 0., 3., 0.], [3., 0., 4., 1.], [4., 1., 4., 2.5], [4., 2.5, 3., 3.5], [3., 3.5, 0., 3.5]]),
        (5.0, &[[0., 7., 0., 0.], [0., 0., 2.5, 4.], [2.5, 4., 5., 0.], [5., 0., 5., 7.]]),
        (4.0, &[[0., 1., 1., 0.], [1., 0., 3., 0.], [3., 0., 4., 1.], [4., 1., 4., 2.5], [4., 2.5, 0., 7.], [0., 7., 4., 7.]]),
        (4.0, &[[0., 0., 0., 7.], [0., 0., 2., 0.], [2., 0., 4., 2.], [4., 2., 4., 5.], [4., 5., 2., 7.], [2., 7., 0., 7.]]),
    ];
    let mut out = Vec::new();
    let mut x = 0.0;
    for (width, strokes) in glyphs {
        for s in strokes {
            out.push(([x + s[0], s[1]], [x + s[2], s[3]]));
        }
        x += width + 1.6;
    }
    out
}
const LETTERS_WIDTH: f32 = 4.0 + 1.6 + 5.0 + 1.6 + 4.0 + 1.6 + 4.0;

fn over(px: &mut Rgba<u8>, col: [u8; 3], cov: f32) {
    if cov <= 0.0 {
        return;
    }
    let a0 = px.0[3] as f32 / 255.0;
    let a = cov + a0 * (1.0 - cov);
    for (c, &new) in px.0.iter_mut().zip(&col) {
        *c = ((new as f32 * cov + *c as f32 * a0 * (1.0 - cov)) / a).round() as u8;
    }
    px.0[3] = (a * 255.0).round() as u8;
}

fn draw(size: u32, app: &RgbaImage) -> RgbaImage {
    let s = size as f32 / 256.0;
    let mut img = RgbaImage::new(size, size);
    // The page: 176 × 232 in a 256 box, its top-right corner folded down by `fold`.
    let (left, right, top, bottom, fold) = (40.0 * s, 216.0 * s, 12.0 * s, 244.0 * s, 52.0 * s);
    let line = (6.0 * s).max(1.0);
    let corner = (12.0 * s).max(1.0);
    let page = |p: [f32; 2]| {
        let body = rbox(p, [(left + right) / 2.0, (top + bottom) / 2.0], (right - left) / 2.0, (bottom - top) / 2.0, corner);
        // Cut the corner off along the fold's diagonal.
        let cut = ((p[0] - (right - fold)) - (p[1] - top)) / std::f32::consts::SQRT_2;
        body.max(cut)
    };
    let flap = |p: [f32; 2]| {
        // The folded-over triangle: right of x = right - fold, above y = top + fold, below the diagonal.
        let (x0, y1) = (right - fold, top + fold);
        let diag = ((p[0] - x0) - (p[1] - top)) / std::f32::consts::SQRT_2;
        (x0 - p[0]).max(p[1] - y1).max(diag)
    };
    let small = size < 32;
    let label_h = 44.0 * s;
    let label = |p: [f32; 2]| rbox(p, [(left + right) / 2.0, bottom - 34.0 * s], (right - left) / 2.0 - 18.0 * s, label_h / 2.0, 8.0 * s);
    let glyph_h = 24.0 * s;
    let unit = glyph_h / 7.0;
    let gx = (left + right) / 2.0 - LETTERS_WIDTH * unit / 2.0;
    let gy = bottom - 34.0 * s - glyph_h / 2.0;
    let strokes = letters();
    for (x, y, px) in img.enumerate_pixels_mut() {
        let p = [x as f32 + 0.5, y as f32 + 0.5];
        let d = page(p);
        over(px, [0x9a, 0xa3, 0xb5], (0.5 - (d - line)).clamp(0.0, 1.0));
        over(px, [0xfb, 0xfc, 0xfe], (0.5 - d).clamp(0.0, 1.0));
        let f = flap(p);
        over(px, [0x9a, 0xa3, 0xb5], (0.5 - (f - line)).clamp(0.0, 1.0));
        over(px, [0xdd, 0xe3, 0xee], (0.5 - f).clamp(0.0, 1.0));
        if !small {
            over(px, [0x2f, 0x7d, 0xf6], (0.5 - label(p)).clamp(0.0, 1.0));
            let q = [(p[0] - gx) / unit, (p[1] - gy) / unit];
            let t = strokes.iter().map(|&(a, b)| segment(q, a, b)).fold(f32::MAX, f32::min) * unit - 2.6 * s;
            over(px, [0xff, 0xff, 0xff], (0.5 - t).clamp(0.0, 1.0));
        }
    }
    // The app icon in the middle of the page (higher up when there's a label).
    let icon = if small { 0.62 } else { 0.56 } * (right - left);
    let icon_px = icon.round().max(1.0) as u32;
    let scaled = imageops::resize(app, icon_px, icon_px, imageops::FilterType::Lanczos3);
    let cx = (left + right) / 2.0 - icon / 2.0;
    let cy = if small { (top + bottom) / 2.0 - icon / 2.0 + 6.0 * s } else { top + 52.0 * s };
    imageops::overlay(&mut img, &scaled, cx.round() as i64, cy.round() as i64);
    img
}

/// An .ico holding each image as PNG (supported since Windows Vista).
fn ico(images: &[RgbaImage]) -> Vec<u8> {
    let pngs: Vec<Vec<u8>> = images
        .iter()
        .map(|im| {
            let mut buf = std::io::Cursor::new(Vec::new());
            im.write_to(&mut buf, image::ImageFormat::Png).unwrap();
            buf.into_inner()
        })
        .collect();
    let mut out = vec![0, 0, 1, 0];
    out.extend((images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (im, png) in images.iter().zip(&pngs) {
        let side = |v: u32| if v >= 256 { 0 } else { v as u8 };
        out.extend([side(im.width()), side(im.height()), 0, 0]);
        out.extend(1u16.to_le_bytes());
        out.extend(32u16.to_le_bytes());
        out.extend((png.len() as u32).to_le_bytes());
        out.extend(offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for png in &pngs {
        out.extend(png);
    }
    out
}

fn main() {
    let app = image::open("app/assets/icon.png").unwrap().to_rgba8();
    let images: Vec<RgbaImage> = SIZES.iter().map(|&s| draw(s, &app)).collect();
    std::fs::write("app/assets/pm2d.ico", ico(&images)).unwrap();
    // A preview of every size side by side, for checking by eye.
    if let Some(path) = std::env::args().nth(1) {
        let w: u32 = SIZES.iter().sum::<u32>() + 10 * SIZES.len() as u32;
        let mut sheet = RgbaImage::from_pixel(w, 256, Rgba([235, 238, 243, 255]));
        let mut x = 0;
        for im in &images {
            imageops::overlay(&mut sheet, im, x, 256 - im.height() as i64);
            x += im.width() as i64 + 10;
        }
        sheet.save(path).unwrap();
    }
}
