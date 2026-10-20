//! The Export step: settings in the side panel, a preview of the framed area
//! on the canvas, and rendering with a progress bar: on a background thread
//! into files on desktop, a frame per screen refresh into a download in the
//! browser.

use crate::panel::card;
use crate::state::AppState;
use anim_core::{Pose, RectI};
use anim_export::{Format, Job, Settings, frame_area, frame_times, output_size};
use eframe::egui;

#[derive(Default)]
pub struct ExportUi {
    pub settings: Settings,
    running: Option<Running>,
}

/// Desktop: an export in progress on its own thread, writing files.
#[cfg(not(target_arch = "wasm32"))]
struct Running {
    done: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    total: usize,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: std::thread::JoinHandle<std::io::Result<anim_export::Outcome>>,
    out: std::path::PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl Running {
    fn done(&self) -> usize {
        self.done.load(std::sync::atomic::Ordering::Relaxed)
    }
    fn cancel(&mut self) {
        self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    fn target(&self) -> String {
        self.out.display().to_string()
    }
}

/// Browser: an export in progress, one frame per screen refresh (there are
/// no threads), downloaded as one file at the end.
#[cfg(target_arch = "wasm32")]
struct Running {
    job: Job,
    next: usize,
    total: usize,
    sink: Sink,
    file_name: String,
    cancelled: bool,
}

#[cfg(target_arch = "wasm32")]
enum Sink {
    Gif(anim_export::GifWriter<Vec<u8>>),
    Frames(Box<anim_export::FramesZip>),
}

#[cfg(target_arch = "wasm32")]
impl Running {
    fn done(&self) -> usize {
        self.next
    }
    fn cancel(&mut self) {
        self.cancelled = true;
    }
    fn target(&self) -> String {
        self.file_name.clone()
    }
}

/// One pose per frame: the motion sampled at the frame rate, or just the
/// current pose when nothing is keyed yet.
fn frame_poses(state: &AppState, fps: u32) -> Vec<Pose> {
    let Some(doc) = &state.doc else { return Vec::new() };
    if doc.motion.is_empty() {
        return vec![doc.pose.clone()];
    }
    frame_times(doc.motion.duration, fps).into_iter().map(|t| doc.motion.sample(t, &doc.skeleton)).collect()
}

/// The drawing-space area the exported frames will show (for the canvas outline).
pub fn frame_rect(state: &AppState) -> Option<RectI> {
    let doc = state.doc.as_ref()?;
    let s = &state.export.settings;
    let poses = frame_poses(state, s.fps);
    Some(frame_area(&doc.sprites, &doc.meshes, &doc.skeleton, &poses, [doc.image.width(), doc.image.height()], s.crop))
}

/// The Export step's side panel.
pub fn panel(ui: &mut egui::Ui, state: &mut AppState) {
    if state.doc.is_none() {
        return;
    }
    state.refresh_sprites(ui.ctx());
    if let Some(r) = &mut state.export.running {
        card(ui, "Exporting", |ui| {
            let done = r.done();
            ui.label(egui::RichText::new(r.target()).weak());
            ui.add(egui::ProgressBar::new(done as f32 / r.total.max(1) as f32).text(format!("frame {done} of {}", r.total)));
            if ui.button("Cancel").clicked() {
                r.cancel();
            }
        });
        return;
    }

    let settings = state.export.settings;
    let poses = frame_poses(state, settings.fps);
    let doc = state.doc.as_ref().unwrap();
    let area = frame_area(&doc.sprites, &doc.meshes, &doc.skeleton, &poses, [doc.image.width(), doc.image.height()], settings.crop);
    let [w, h] = output_size(area, settings.scale);
    let no_keys = doc.motion.is_empty();
    card(ui, "Preview", |ui| {
        let play = if state.playing { "⏸ Stop preview" } else { "▶ Preview" };
        if ui.button(play).on_hover_text("Space: play the animation as it will be exported").clicked() {
            state.toggle_play();
        }
        ui.label(format!("{} frames, {w} × {h} px", poses.len()));
        if no_keys {
            ui.label(egui::RichText::new("No keys yet, so this exports the current pose as a single frame.").weak());
        }
    });

    card(ui, "Settings", |ui| {
        let s = &mut state.export.settings;
        egui::Grid::new("export settings").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Frame rate");
            ui.add(egui::DragValue::new(&mut s.fps).range(1..=60).suffix(" fps"))
                .on_hover_text("Starts at the timeline's frame rate");
            ui.end_row();
            ui.label("Size");
            let mut percent = s.scale * 100.0;
            ui.add(egui::DragValue::new(&mut percent).range(10.0..=400.0).speed(1.0).suffix(" %"));
            s.scale = percent / 100.0;
            ui.end_row();
            ui.label("Background");
            ui.horizontal(|ui| {
                let mut transparent = s.background.is_none();
                ui.checkbox(&mut transparent, "Transparent");
                match (transparent, &mut s.background) {
                    (true, bg) => *bg = None,
                    (false, None) => s.background = Some([255, 255, 255]),
                    (false, Some(rgb)) => {
                        ui.color_edit_button_srgb(rgb);
                    }
                }
            });
            ui.end_row();
        });
        ui.checkbox(&mut s.crop, "Crop to the moving character")
            .on_hover_text("The dashed box on the canvas. Off: every frame is the size of the whole drawing.");
    });

    card(ui, "Save", |ui| {
        let buttons = [
            (Format::Gif, "Export GIF…", "One looping animated GIF, for sharing and previews"),
            (Format::Png, "Export frames…", "One PNG per frame in a folder, for game engines"),
        ];
        for (format, label, hint) in buttons {
            let text = egui::RichText::new(label).size(15.0).strong();
            let button = egui::Button::new(text).min_size(egui::vec2(ui.available_width(), 30.0));
            if ui.add(button).on_hover_text(hint).clicked() {
                state.export.settings.format = format;
                begin(state);
            }
        }
        if settings.background.is_none() {
            ui.label(egui::RichText::new("GIF has no soft edges, so semi-transparent pixels become solid or clear.").weak());
        }
    });
}

/// Open the file manager showing what was just exported: the GIF selected
/// in its folder, or the frames folder itself.
#[cfg(not(target_arch = "wasm32"))]
fn reveal(path: &std::path::Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Explorer needs `/select,"path"` as one raw argument; it exits with 1 even on success.
        let arg = if path.is_dir() { format!("\"{}\"", path.display()) } else { format!("/select,\"{}\"", path.display()) };
        let _ = std::process::Command::new("explorer").raw_arg(arg).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let dir = if path.is_dir() { path } else { path.parent().unwrap_or(path) };
        let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
    }
}

/// Pick up a finished export and keep the progress bar moving. Call once per frame.
#[cfg(not(target_arch = "wasm32"))]
pub fn poll(ctx: &egui::Context, state: &mut AppState) {
    finish(ctx, state);
    if state.export.running.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// Render the next frame of a browser export, and download the file when
/// it's done. Call once per frame.
#[cfg(target_arch = "wasm32")]
pub fn poll(ctx: &egui::Context, state: &mut AppState) {
    let Some(r) = &mut state.export.running else { return };
    if r.cancelled {
        state.export.running = None;
        state.status = "Export cancelled".into();
        return;
    }
    if r.next < r.total {
        let frame = r.job.render_frame(r.next);
        let added = match &mut r.sink {
            Sink::Gif(gif) => gif.add_frame(frame),
            Sink::Frames(zip) => zip.add_frame(&frame),
        };
        r.next += 1;
        if let Err(e) = added {
            state.export.running = None;
            state.status = format!("Export failed: {e}");
            return;
        }
        ctx.request_repaint();
        return;
    }
    let r = state.export.running.take().unwrap();
    let (bytes, mime) = match r.sink {
        Sink::Gif(gif) => (gif.finish(), "image/gif"),
        Sink::Frames(zip) => ((*zip).finish(), "application/zip"),
    };
    state.status = match bytes {
        Ok(bytes) => {
            crate::platform::download(&r.file_name, &bytes, mime);
            format!("Exported {} frames as {}", r.total, r.file_name)
        }
        Err(e) => format!("Export failed: {e}"),
    };
}

/// Start rendering in the browser: frames are made one per screen refresh
/// (see `poll`) and downloaded as a GIF or a zip of PNGs.
#[cfg(target_arch = "wasm32")]
fn begin(state: &mut AppState) {
    let settings = state.export.settings;
    let Some(doc) = &state.doc else { return };
    let job = Job {
        sprites: doc.sprites.clone(),
        meshes: doc.meshes.clone(),
        skeleton: doc.skeleton.clone(),
        poses: frame_poses(state, settings.fps),
        image_size: [doc.image.width(), doc.image.height()],
        settings,
    };
    let (sink, file_name) = match settings.format {
        Format::Gif => match anim_export::GifWriter::new(&job, Vec::new()) {
            Ok(gif) => (Sink::Gif(gif), format!("{}.gif", doc.name)),
            Err(e) => {
                state.status = format!("Export failed: {e}");
                return;
            }
        },
        Format::Png => (Sink::Frames(Box::default()), format!("{}_frames.zip", doc.name)),
    };
    let total = job.poses.len();
    state.pause();
    state.status = format!("Exporting {total} frames…");
    state.export.running = Some(Running { job, next: 0, total, sink, file_name, cancelled: false });
}

/// Ask where to save, then start rendering on a background thread.
#[cfg(not(target_arch = "wasm32"))]
fn begin(state: &mut AppState) {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let settings = state.export.settings;
    let Some(doc) = &state.doc else { return };
    let out = match settings.format {
        Format::Gif => {
            let Some(mut path) = rfd::FileDialog::new()
                .set_title("Export GIF")
                .add_filter("GIF", &["gif"])
                .set_file_name(format!("{}.gif", doc.name))
                .save_file()
            else {
                return;
            };
            if path.extension().is_none_or(|e| !e.eq_ignore_ascii_case("gif")) {
                path.set_extension("gif");
            }
            path
        }
        Format::Png => {
            let Some(folder) = rfd::FileDialog::new().set_title("Export frames into…").pick_folder() else { return };
            folder.join(format!("{}_frames", doc.name))
        }
    };
    let job = Job {
        sprites: doc.sprites.clone(),
        meshes: doc.meshes.clone(),
        skeleton: doc.skeleton.clone(),
        poses: frame_poses(state, settings.fps),
        image_size: [doc.image.width(), doc.image.height()],
        settings,
    };
    let total = job.poses.len();
    let done = Arc::new(AtomicUsize::new(0));
    let cancel = Arc::new(AtomicBool::new(false));
    let (d, c, path) = (done.clone(), cancel.clone(), out.clone());
    let handle = std::thread::spawn(move || {
        job.run(&path, |n| {
            d.store(n, Ordering::Relaxed);
            !c.load(Ordering::Relaxed)
        })
    });
    state.pause();
    state.status = format!("Exporting {total} frames…");
    state.export.running = Some(Running { done, total, cancel, handle, out });
}

/// Report a finished export in the status line and show it in the file manager.
#[cfg(not(target_arch = "wasm32"))]
fn finish(ctx: &egui::Context, state: &mut AppState) {
    use anim_export::Outcome;
    if !state.export.running.as_ref().is_some_and(|r| r.handle.is_finished()) {
        return;
    }
    let r = state.export.running.take().unwrap();
    let result = r.handle.join().unwrap_or_else(|_| Err(std::io::Error::other("the export crashed")));
    state.status = match result {
        Ok(Outcome::Done(n)) => {
            reveal(&r.out);
            format!("Exported {n} frames to {}", r.out.display())
        }
        Ok(Outcome::Cancelled) => "Export cancelled".into(),
        Err(e) => format!("Export failed: {e}"),
    };
    ctx.request_repaint();
}
