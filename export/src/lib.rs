//! Animation export: render every frame on the CPU and write them out as a
//! PNG sequence or an animated GIF. Must never depend on UI crates.
//!
//! Frames are drawn the way the canvas draws Pose mode: each part's sprite
//! moved by its part's transform, parents first so children sit on top,
//! sampled with bilinear filtering.

use anim_core::{Pose, RectI, Skeleton, Sprite, world_transforms};
use image::RgbaImage;
use std::io::{self, BufWriter};
use std::path::Path;

/// Space left around the moving figure when cropping (drawing pixels).
pub const CROP_MARGIN: f32 = 8.0;

/// GIF quantizer speed: 1 is best quality and slowest, 30 fastest.
const GIF_SPEED: i32 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// One PNG per frame in a folder.
    Png,
    /// One looping animated GIF.
    Gif,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub format: Format,
    pub fps: u32,
    /// Output size relative to the drawing (1.0 = same pixel size).
    pub scale: f32,
    /// Color behind the figure; None = transparent.
    pub background: Option<[u8; 3]>,
    /// Frame just the area the figure moves through (plus a margin) instead
    /// of the whole drawing.
    pub crop: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { format: Format::Gif, fps: 24, scale: 1.0, background: Some([255, 255, 255]), crop: true }
    }
}

/// Times (seconds) of the frames of a motion `duration` long. The moment at
/// `duration` itself is left out: when the animation loops, that is the start.
pub fn frame_times(duration: f32, fps: u32) -> Vec<f32> {
    let fps = fps.max(1);
    let n = ((duration * fps as f32).round() as usize).max(1);
    (0..n).map(|i| i as f32 / fps as f32).collect()
}

/// How an export ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Every frame written; holds the frame count.
    Done(usize),
    Cancelled,
}

/// Everything needed to render, owned so it can run on another thread.
pub struct Job {
    pub sprites: Vec<Sprite>,
    pub skeleton: Skeleton,
    /// One pose per frame.
    pub poses: Vec<Pose>,
    /// Size of the original drawing.
    pub image_size: [u32; 2],
    pub settings: Settings,
}

/// The drawing-space area the frames show: the whole drawing, or with `crop`
/// everything the sprites cover in any of `poses`, plus a margin.
pub fn frame_area(sprites: &[Sprite], skeleton: &Skeleton, poses: &[Pose], image_size: [u32; 2], crop: bool) -> RectI {
    let [w, h] = image_size;
    let full = RectI::new(0, 0, w as i32, h as i32);
    if !crop {
        return full;
    }
    let mut lo = [f32::INFINITY; 2];
    let mut hi = [f32::NEG_INFINITY; 2];
    for pose in poses {
        let t = world_transforms(skeleton, pose);
        for s in sprites.iter().filter(|s| s.image.width() > 0 && s.part < t.len()) {
            for c in sprite_corners(s) {
                let p = t[s.part].apply(c);
                for k in 0..2 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
        }
    }
    if lo[0] > hi[0] {
        return full;
    }
    RectI::from_corners(
        (lo[0] - CROP_MARGIN).floor() as i32,
        (lo[1] - CROP_MARGIN).floor() as i32,
        (hi[0] + CROP_MARGIN).ceil() as i32,
        (hi[1] + CROP_MARGIN).ceil() as i32,
    )
}

/// Output frame size in pixels for `area` drawn at `scale`.
pub fn output_size(area: RectI, scale: f32) -> [u32; 2] {
    [((area.w as f32 * scale).round() as u32).max(1), ((area.h as f32 * scale).round() as u32).max(1)]
}

impl Job {
    /// The drawing-space area every frame shows.
    pub fn area(&self) -> RectI {
        frame_area(&self.sprites, &self.skeleton, &self.poses, self.image_size, self.settings.crop)
    }

    /// Output frame size in pixels.
    pub fn output_size(&self) -> [u32; 2] {
        output_size(self.area(), self.settings.scale)
    }

    /// Draw frame `i`.
    pub fn render_frame(&self, i: usize) -> RgbaImage {
        let area = self.area();
        let [w, h] = output_size(area, self.settings.scale);
        // Drawing pixels per output pixel.
        let (sx, sy) = (area.w as f32 / w as f32, area.h as f32 / h as f32);
        let bg = self.settings.background.map_or([0.0; 4], |[r, g, b]| [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]);
        // Premultiplied RGBA, 0..1.
        let mut buf = vec![bg; (w * h) as usize];
        let rest = Pose::rest(self.skeleton.parts.len());
        let t = world_transforms(&self.skeleton, self.poses.get(i).unwrap_or(&rest));

        for p in self.skeleton.draw_order() {
            let Some(sprite) = self.sprites.get(p).filter(|s| s.image.width() > 0) else { continue };
            // Output pixels the moved sprite can touch.
            let (mut lo, mut hi) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
            for c in sprite_corners(sprite) {
                let q = t[p].apply(c);
                let o = [(q[0] - area.x as f32) / sx, (q[1] - area.y as f32) / sy];
                for k in 0..2 {
                    lo[k] = lo[k].min(o[k]);
                    hi[k] = hi[k].max(o[k]);
                }
            }
            let (x0, x1) = (lo[0].floor().max(0.0) as u32, (hi[0].ceil().max(0.0) as u32).min(w));
            let (y0, y1) = (lo[1].floor().max(0.0) as u32, (hi[1].ceil().max(0.0) as u32).min(h));
            let inv = t[p].inverse();
            let [ox, oy] = [sprite.origin[0] as f32, sprite.origin[1] as f32];
            for y in y0..y1 {
                for x in x0..x1 {
                    let q = [area.x as f32 + (x as f32 + 0.5) * sx, area.y as f32 + (y as f32 + 0.5) * sy];
                    let r = inv.apply(q);
                    let src = sample(&sprite.image, r[0] - ox - 0.5, r[1] - oy - 0.5);
                    if src[3] <= 0.0 {
                        continue;
                    }
                    let dst = &mut buf[(y * w + x) as usize];
                    for k in 0..4 {
                        dst[k] = src[k] + dst[k] * (1.0 - src[3]);
                    }
                }
            }
        }

        let mut out = RgbaImage::new(w, h);
        for (px, c) in out.pixels_mut().zip(&buf) {
            let a = c[3];
            if a > 0.0 {
                let to8 = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
                px.0 = [to8(c[0] / a), to8(c[1] / a), to8(c[2] / a), to8(a)];
            }
        }
        out
    }

    /// Render every frame and write them to `out`: a folder for a PNG
    /// sequence (created if needed), a file for a GIF. `progress` is called
    /// with the number of frames done; returning false cancels (a cancelled
    /// GIF is deleted, PNGs already written are kept).
    pub fn run(&self, out: &Path, mut progress: impl FnMut(usize) -> bool) -> io::Result<Outcome> {
        let n = self.poses.len();
        match self.settings.format {
            Format::Png => {
                std::fs::create_dir_all(out)?;
                for i in 0..n {
                    self.render_frame(i).save(out.join(format!("frame_{:04}.png", i + 1))).map_err(io::Error::other)?;
                    if !progress(i + 1) {
                        return Ok(Outcome::Cancelled);
                    }
                }
            }
            Format::Gif => {
                let result = self.write_gif(out, &mut progress);
                if !matches!(result, Ok(Outcome::Done(_))) {
                    let _ = std::fs::remove_file(out);
                }
                return result;
            }
        }
        Ok(Outcome::Done(n))
    }

    fn write_gif(&self, path: &Path, progress: &mut impl FnMut(usize) -> bool) -> io::Result<Outcome> {
        let [w, h] = self.output_size();
        let too_big = || io::Error::other(format!("{w}×{h} is too big for a GIF (65535 px per side at most)"));
        let (w16, h16) = (u16::try_from(w).map_err(|_| too_big())?, u16::try_from(h).map_err(|_| too_big())?);
        let file = BufWriter::new(std::fs::File::create(path)?);
        let mut encoder = gif::Encoder::new(file, w16, h16, &[]).map_err(io::Error::other)?;
        encoder.set_repeat(gif::Repeat::Infinite).map_err(io::Error::other)?;
        let transparent = self.settings.background.is_none();
        let fps = self.settings.fps.max(1) as f32;
        // GIF delays are whole hundredths of a second; round the running
        // total so the overall speed stays right (e.g. 24 fps → 4, 4, 4, 5, …).
        let centis = |i: usize| (i as f32 * 100.0 / fps).round() as u16;
        for i in 0..self.poses.len() {
            let mut pixels = self.render_frame(i).into_raw();
            if transparent {
                // GIF pixels are either fully see-through or fully solid.
                for a in pixels.iter_mut().skip(3).step_by(4) {
                    *a = if *a < 128 { 0 } else { 255 };
                }
            }
            let mut frame = gif::Frame::from_rgba_speed(w16, h16, &mut pixels, GIF_SPEED);
            frame.delay = centis(i + 1) - centis(i);
            // Clear each frame before the next, or moved parts leave trails on a transparent background.
            frame.dispose = gif::DisposalMethod::Background;
            encoder.write_frame(&frame).map_err(io::Error::other)?;
            if !progress(i + 1) {
                return Ok(Outcome::Cancelled);
            }
        }
        Ok(Outcome::Done(self.poses.len()))
    }
}

fn sprite_corners(s: &Sprite) -> [[f32; 2]; 4] {
    let [x, y] = [s.origin[0] as f32, s.origin[1] as f32];
    let [w, h] = [s.image.width() as f32, s.image.height() as f32];
    [[x, y], [x + w, y], [x + w, y + h], [x, y + h]]
}

/// Bilinear sample of `img` at (`fx`, `fy`) in texel-center coordinates
/// (0, 0 = center of the top-left pixel), premultiplied RGBA 0..1.
/// Outside the image is transparent.
fn sample(img: &RgbaImage, fx: f32, fy: f32) -> [f32; 4] {
    let (w, h) = (img.width() as i64, img.height() as i64);
    if fx < -1.0 || fy < -1.0 || fx >= w as f32 || fy >= h as f32 {
        return [0.0; 4];
    }
    let (x0, y0) = (fx.floor() as i64, fy.floor() as i64);
    let (u, v) = (fx - x0 as f32, fy - y0 as f32);
    let texel = |x: i64, y: i64| -> [f32; 4] {
        if x < 0 || y < 0 || x >= w || y >= h {
            return [0.0; 4];
        }
        let [r, g, b, a] = img.get_pixel(x as u32, y as u32).0;
        let a = a as f32 / 255.0;
        [r as f32 / 255.0 * a, g as f32 / 255.0 * a, b as f32 / 255.0 * a, a]
    };
    let (c00, c10, c01, c11) = (texel(x0, y0), texel(x0 + 1, y0), texel(x0, y0 + 1), texel(x0 + 1, y0 + 1));
    let mut out = [0.0; 4];
    for k in 0..4 {
        let top = c00[k] + (c10[k] - c00[k]) * u;
        let bottom = c01[k] + (c11[k] - c01[k]) * u;
        out[k] = top + (bottom - top) * v;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{AnimationDecoder, Rgba};
    use std::f32::consts::PI;

    /// A 10×10 drawing with one bone along a 4×2 red bar at (2, 4), pivot at its left end.
    fn job(poses: Vec<Pose>, settings: Settings) -> Job {
        let mut skeleton = Skeleton::empty();
        let a = skeleton.add_joint([2.0, 5.0]);
        let b = skeleton.add_joint([6.0, 5.0]);
        skeleton.add_part(a, b);
        let image = RgbaImage::from_pixel(4, 2, Rgba([255, 0, 0, 255]));
        let sprites = vec![Sprite { part: 0, image, origin: [2, 4], pivot: [2.0, 5.0] }];
        Job { sprites, skeleton, poses, image_size: [10, 10], settings }
    }

    fn uncropped(background: Option<[u8; 3]>) -> Settings {
        Settings { format: Format::Png, fps: 10, scale: 1.0, background, crop: false }
    }

    #[test]
    fn frame_times_leave_out_the_loop_point() {
        let t = frame_times(1.0, 24);
        assert_eq!(t.len(), 24);
        assert_eq!(t[0], 0.0);
        assert!(t[23] < 1.0);
        assert_eq!(frame_times(0.0, 24).len(), 1, "always at least one frame");
    }

    #[test]
    fn rest_pose_reproduces_the_drawing() {
        let img = job(vec![Pose::rest(1)], uncropped(None)).render_frame(0);
        assert_eq!(img.dimensions(), (10, 10));
        assert_eq!(img.get_pixel(3, 4).0, [255, 0, 0, 255]);
        assert_eq!(img.get_pixel(5, 5).0, [255, 0, 0, 255]);
        assert_eq!(img.get_pixel(0, 0).0[3], 0, "transparent outside the part");
        assert_eq!(img.get_pixel(7, 5).0[3], 0);
    }

    #[test]
    fn rotation_moves_the_part_and_background_fills_the_rest() {
        let mut pose = Pose::rest(1);
        pose.set_angle(0, PI); // swing the bar to the left of its pivot
        let img = job(vec![pose], uncropped(Some([255, 255, 255]))).render_frame(0);
        assert_eq!(img.get_pixel(0, 5).0, [255, 0, 0, 255]);
        assert_eq!(img.get_pixel(4, 5).0, [255, 255, 255, 255], "old place is background now");
    }

    #[test]
    fn crop_frames_the_motion_with_a_margin() {
        let mut moved = Pose::rest(1);
        moved.offset = [20.0, 0.0];
        let settings = Settings { crop: true, ..uncropped(None) };
        let j = job(vec![Pose::rest(1), moved], settings);
        let m = CROP_MARGIN as i32;
        assert_eq!(j.area(), RectI::from_corners(2 - m, 4 - m, 26 + m, 6 + m));
        let scaled = Job { settings: Settings { scale: 0.5, ..settings }, ..j };
        assert_eq!(scaled.output_size(), [(24 + 2 * m as u32) / 2, (2 + 2 * m as u32) / 2]);
    }

    #[test]
    fn gif_has_one_frame_per_pose_and_cancel_removes_it() {
        let dir = std::env::temp_dir().join(format!("anim-export-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("walk.gif");
        let poses: Vec<Pose> = (0..5).map(|i| {
            let mut p = Pose::rest(1);
            p.set_angle(0, i as f32 * 0.3);
            p
        }).collect();
        let j = job(poses, Settings { format: Format::Gif, ..uncropped(None) });

        assert_eq!(j.run(&path, |_| true).unwrap(), Outcome::Done(5));
        let file = std::io::BufReader::new(std::fs::File::open(&path).unwrap());
        let frames = image::codecs::gif::GifDecoder::new(file).unwrap().into_frames().collect_frames().unwrap();
        assert_eq!(frames.len(), 5);
        assert_eq!(frames[0].delay().numer_denom_ms(), (100, 1), "10 fps = 100 ms per frame");

        assert_eq!(j.run(&path, |done| done < 2).unwrap(), Outcome::Cancelled);
        assert!(!path.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
