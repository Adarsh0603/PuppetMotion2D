//! Animation export: render every frame on the CPU and write them out as a
//! PNG sequence or an animated GIF. Must never depend on UI crates.
//!
//! Frames are drawn the way the canvas draws Pose mode: each part's sprite
//! moved by its part's transform, parents first so children sit on top,
//! sampled with bilinear filtering.

use anim_core::{PartMesh, Pose, RectI, Skeleton, Sprite, world_transforms};
use image::RgbaImage;
use std::io::{self, BufWriter, Write};
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
    /// Each part's mesh over its sprite (as the canvas draws it), bent by the
    /// bones. A part without one (or with an empty one) is drawn as its
    /// sprite turned rigidly.
    pub meshes: Vec<PartMesh>,
    pub skeleton: Skeleton,
    /// One pose per frame.
    pub poses: Vec<Pose>,
    /// Size of the original drawing.
    pub image_size: [u32; 2],
    pub settings: Settings,
}

/// The drawing-space area the frames show: the whole drawing, or with `crop`
/// everything the parts cover in any of `poses`, plus a margin.
pub fn frame_area(
    sprites: &[Sprite],
    meshes: &[PartMesh],
    skeleton: &Skeleton,
    poses: &[Pose],
    image_size: [u32; 2],
    crop: bool,
) -> RectI {
    let [w, h] = image_size;
    let full = RectI::new(0, 0, w as i32, h as i32);
    if !crop {
        return full;
    }
    let mut lo = [f32::INFINITY; 2];
    let mut hi = [f32::NEG_INFINITY; 2];
    let mut grow = |p: [f32; 2]| {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    };
    for pose in poses {
        let t = world_transforms(skeleton, pose);
        for s in sprites.iter().filter(|s| s.image.width() > 0 && s.part < t.len()) {
            match meshes.get(s.part).filter(|m| !m.triangles.is_empty()) {
                Some(m) => m.deform(&t).into_iter().for_each(&mut grow),
                None => sprite_corners(s).into_iter().for_each(|c| grow(t[s.part].apply(c))),
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
        frame_area(&self.sprites, &self.meshes, &self.skeleton, &self.poses, self.image_size, self.settings.crop)
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

        // Drawing-space point → output pixel coordinates.
        let to_out = |q: [f32; 2]| [(q[0] - area.x as f32) / sx, (q[1] - area.y as f32) / sy];
        for p in self.skeleton.paint_order() {
            let Some(sprite) = self.sprites.get(p).filter(|s| s.image.width() > 0) else { continue };
            if let Some(mesh) = self.meshes.get(p).filter(|m| !m.triangles.is_empty()) {
                let moved: Vec<[f32; 2]> = mesh.deform(&t).into_iter().map(to_out).collect();
                for tri in &mesh.triangles {
                    let corner = |k: usize| (moved[tri[k] as usize], mesh.uvs[tri[k] as usize]);
                    draw_triangle(&mut buf, w, h, [corner(0), corner(1), corner(2)], &sprite.image);
                }
                continue;
            }
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
        let mut gif = GifWriter::new(self, BufWriter::new(std::fs::File::create(path)?))?;
        for i in 0..self.poses.len() {
            gif.add_frame(self.render_frame(i))?;
            if !progress(i + 1) {
                return Ok(Outcome::Cancelled);
            }
        }
        gif.finish()?;
        Ok(Outcome::Done(self.poses.len()))
    }
}

/// Writes a job's frames into a looping GIF one at a time, so the caller
/// decides the pace (a background thread on desktop, a frame per screen
/// refresh in the browser).
pub struct GifWriter<W: Write> {
    encoder: gif::Encoder<W>,
    size: (u16, u16),
    transparent: bool,
    fps: f32,
    /// Frames written so far.
    count: usize,
}

impl<W: Write> GifWriter<W> {
    pub fn new(job: &Job, out: W) -> io::Result<Self> {
        let [w, h] = job.output_size();
        let too_big = || io::Error::other(format!("{w}×{h} is too big for a GIF (65535 px per side at most)"));
        let size = (u16::try_from(w).map_err(|_| too_big())?, u16::try_from(h).map_err(|_| too_big())?);
        let mut encoder = gif::Encoder::new(out, size.0, size.1, &[]).map_err(io::Error::other)?;
        encoder.set_repeat(gif::Repeat::Infinite).map_err(io::Error::other)?;
        let transparent = job.settings.background.is_none();
        Ok(GifWriter { encoder, size, transparent, fps: job.settings.fps.max(1) as f32, count: 0 })
    }

    /// Add the next frame (as rendered by `Job::render_frame`).
    pub fn add_frame(&mut self, image: RgbaImage) -> io::Result<()> {
        let mut pixels = image.into_raw();
        if self.transparent {
            // GIF pixels are either fully see-through or fully solid.
            for a in pixels.iter_mut().skip(3).step_by(4) {
                *a = if *a < 128 { 0 } else { 255 };
            }
        }
        // GIF delays are whole hundredths of a second; round the running
        // total so the overall speed stays right (e.g. 24 fps → 4, 4, 4, 5, …).
        let centis = |i: usize| (i as f32 * 100.0 / self.fps).round() as u16;
        let mut frame = gif::Frame::from_rgba_speed(self.size.0, self.size.1, &mut pixels, GIF_SPEED);
        frame.delay = centis(self.count + 1) - centis(self.count);
        // Clear each frame before the next, or moved parts leave trails on a transparent background.
        frame.dispose = gif::DisposalMethod::Background;
        self.encoder.write_frame(&frame).map_err(io::Error::other)?;
        self.count += 1;
        Ok(())
    }

    /// Finish the file and hand back what it was written to.
    pub fn finish(self) -> io::Result<W> {
        self.encoder.into_inner().map_err(io::Error::other)
    }
}

/// Collects frames as `frame_0001.png`, `frame_0002.png`, … in a zip, in
/// memory (for the browser, where frames are downloaded as one file).
pub struct FramesZip {
    zip: zip::ZipWriter<io::Cursor<Vec<u8>>>,
    count: usize,
}

impl Default for FramesZip {
    fn default() -> Self {
        FramesZip { zip: zip::ZipWriter::new(io::Cursor::new(Vec::new())), count: 0 }
    }
}

impl FramesZip {
    pub fn add_frame(&mut self, image: &RgbaImage) -> io::Result<()> {
        let mut png = io::Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).map_err(io::Error::other)?;
        self.count += 1;
        // PNGs are already compressed; store them as they are.
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        self.zip.start_file(format!("frame_{:04}.png", self.count), stored).map_err(io::Error::other)?;
        self.zip.write_all(png.get_ref())?;
        Ok(())
    }

    /// The finished zip file's bytes.
    pub fn finish(self) -> io::Result<Vec<u8>> {
        Ok(self.zip.finish().map_err(io::Error::other)?.into_inner())
    }
}

/// Paint one textured triangle over `buf` (premultiplied RGBA, `w`×`h`).
/// Corners are (output pixel position, UV on `image`). Each pixel whose
/// center is inside takes the image's colour at the matching UV, blended on
/// top. Pixels exactly on an edge belong to one side only (the top-left
/// rule), so neighbouring triangles never paint a pixel twice.
fn draw_triangle(buf: &mut [[f32; 4]], w: u32, h: u32, corners: [([f32; 2], [f32; 2]); 3], image: &RgbaImage) {
    let [(a, ua), (mut b, mut ub), (mut c, mut uc)] = corners;
    let edge = |p: [f32; 2], q: [f32; 2], r: [f32; 2]| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
    let mut area = edge(a, b, c);
    if area.abs() < 1e-9 {
        return;
    }
    if area < 0.0 {
        // Make the corners go the same way round every time.
        std::mem::swap(&mut b, &mut c);
        std::mem::swap(&mut ub, &mut uc);
        area = -area;
    }
    // Top-left rule: a pixel on an edge counts only for top or left edges.
    let top_left = |p: [f32; 2], q: [f32; 2]| (p[1] == q[1] && q[0] < p[0]) || q[1] > p[1];
    let owns = |e: f32, p: [f32; 2], q: [f32; 2]| e > 0.0 || (e == 0.0 && top_left(p, q));
    let x0 = a[0].min(b[0]).min(c[0]).floor().max(0.0) as u32;
    let x1 = (a[0].max(b[0]).max(c[0]).ceil().max(0.0) as u32).min(w);
    let y0 = a[1].min(b[1]).min(c[1]).floor().max(0.0) as u32;
    let y1 = (a[1].max(b[1]).max(c[1]).ceil().max(0.0) as u32).min(h);
    let (iw, ih) = (image.width() as f32, image.height() as f32);
    for y in y0..y1 {
        for x in x0..x1 {
            let p = [x as f32 + 0.5, y as f32 + 0.5];
            let (ea, eb, ec) = (edge(b, c, p), edge(c, a, p), edge(a, b, p));
            if !(owns(ea, b, c) && owns(eb, c, a) && owns(ec, a, b)) {
                continue;
            }
            let (la, lb, lc) = (ea / area, eb / area, ec / area);
            let u = la * ua[0] + lb * ub[0] + lc * uc[0];
            let v = la * ua[1] + lb * ub[1] + lc * uc[1];
            let src = sample(image, u * iw - 0.5, v * ih - 0.5);
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
        let meshes = anim_core::build_meshes(&sprites, 2.0);
        Job { sprites, meshes, skeleton, poses, image_size: [10, 10], settings }
    }

    #[test]
    fn gif_and_frames_can_be_made_in_memory() {
        let poses: Vec<Pose> = (0..3).map(|i| {
            let mut p = Pose::rest(1);
            p.set_angle(0, i as f32 * 0.4);
            p
        }).collect();
        let j = job(poses, Settings { format: Format::Gif, ..uncropped(None) });
        let mut gif = GifWriter::new(&j, Vec::new()).unwrap();
        for i in 0..3 {
            gif.add_frame(j.render_frame(i)).unwrap();
        }
        let bytes = gif.finish().unwrap();
        let frames = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).unwrap().into_frames().collect_frames().unwrap();
        assert_eq!(frames.len(), 3);

        let mut zip = FramesZip::default();
        for i in 0..3 {
            zip.add_frame(&j.render_frame(i)).unwrap();
        }
        let archive = zip::ZipArchive::new(std::io::Cursor::new(zip.finish().unwrap())).unwrap();
        let names: Vec<String> = archive.file_names().filter_map(Result::ok).map(|n| n.into_owned()).collect();
        assert_eq!(names.len(), 3);
        assert!(names.iter().any(|n| n == "frame_0001.png") && names.iter().any(|n| n == "frame_0003.png"));
    }

    #[test]
    fn mesh_and_rigid_renders_match_for_a_rigid_part() {
        // A bigger textured part, turned 30°: drawn as a mesh and as a rigid sprite.
        let mut skeleton = Skeleton::empty();
        let (a, b) = (skeleton.add_joint([10.0, 20.0]), skeleton.add_joint([40.0, 20.0]));
        skeleton.add_part(a, b);
        let image = RgbaImage::from_fn(30, 12, |x, y| Rgba([(x * 8) as u8, (y * 20) as u8, 100, 255]));
        let sprites = vec![Sprite { part: 0, image, origin: [10, 14], pivot: [10.0, 20.0] }];
        let mut pose = Pose::rest(1);
        pose.set_angle(0, 0.5);
        let settings = uncropped(None);
        let meshed = Job { meshes: anim_core::build_meshes(&sprites, 5.0), ..rigid_job(&sprites, &skeleton, &pose, settings) };
        let rigid = rigid_job(&sprites, &skeleton, &pose, settings);
        let (m, r) = (meshed.render_frame(0), rigid.render_frame(0));
        let differ = m.pixels().zip(r.pixels()).filter(|(p, q)| p.0.iter().zip(q.0).any(|(x, y)| x.abs_diff(y) > 2)).count();
        // Only a few pixels right on the part's outline may round differently.
        assert!(differ <= m.width() as usize, "{differ} pixels differ");
    }

    fn rigid_job(sprites: &[Sprite], skeleton: &Skeleton, pose: &Pose, settings: Settings) -> Job {
        Job {
            sprites: sprites.to_vec(),
            meshes: Vec::new(),
            skeleton: skeleton.clone(),
            poses: vec![pose.clone()],
            image_size: [60, 50],
            settings,
        }
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
