//! Decision-gate helper: writes `<dir>/out/<name>_fg.png` for every image in
//! `<dir>`, with the detected foreground tinted over the original.
//!
//! cargo run -p anim-core --example batch_fg -- testdata [gap_close]

use anim_core::{Params, foreground_mask};
use image::{Rgba, RgbaImage};
use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "testdata".into()));
    let out_dir = dir.join("out");
    std::fs::create_dir_all(&out_dir).expect("create out dir");
    let mut params = Params::default();
    if let Some(gap) = std::env::args().nth(2) {
        params.gap_close = gap.parse().expect("gap_close must be a whole number");
    }

    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("read input dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif")
        })
        .collect();
    entries.sort();

    for path in entries {
        let img = match image::open(&path) {
            Ok(i) => i.to_rgba8(),
            Err(e) => {
                eprintln!("skip {}: {e}", path.display());
                continue;
            }
        };
        let t = std::time::Instant::now();
        let fg = foreground_mask(&img, &params);
        let ms = t.elapsed().as_millis();

        // Original on a checkerboard with a magenta tint over the foreground and
        // background darkened, so leaks in either direction are obvious.
        let mut out = RgbaImage::new(img.width(), img.height());
        for (x, y, px) in out.enumerate_pixels_mut() {
            let checker = if (x / 16 + y / 16) % 2 == 0 { 200.0 } else { 160.0 };
            let s = img.get_pixel(x, y).0;
            let a = s[3] as f32 / 255.0;
            let mut rgb = [0f32; 3];
            for i in 0..3 {
                rgb[i] = s[i] as f32 * a + checker * (1.0 - a);
            }
            if fg.get(x, y) {
                let tint = [255.0, 0.0, 200.0];
                for i in 0..3 {
                    rgb[i] = rgb[i] * 0.6 + tint[i] * 0.4;
                }
            } else {
                for v in &mut rgb {
                    *v *= 0.35;
                }
            }
            *px = Rgba([rgb[0] as u8, rgb[1] as u8, rgb[2] as u8, 255]);
        }
        let stem = path.file_stem().unwrap().to_string_lossy();
        let dst = out_dir.join(format!("{stem}_fg.png"));
        out.save(&dst).expect("save overlay");
        let pct = 100.0 * fg.count() as f64 / (img.width() as f64 * img.height() as f64);
        println!("{:<40} {:>5} ms  fg {:>5.1}%", path.file_name().unwrap().to_string_lossy(), ms, pct);
    }
}
