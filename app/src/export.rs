//! The "Export animation" window: pick a format and settings, then the frames
//! are rendered on a background thread while a progress bar runs.

use crate::state::AppState;
use anim_core::Pose;
use anim_export::{Format, Job, Outcome, Settings, frame_area, frame_times, output_size};
use eframe::egui;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::JoinHandle;

#[derive(Default)]
pub struct ExportUi {
    pub open: bool,
    /// Whether the window was showing last frame, to notice it opening.
    was_open: bool,
    pub settings: Settings,
    running: Option<Running>,
}

/// An export in progress on its own thread.
struct Running {
    done: Arc<AtomicUsize>,
    total: usize,
    cancel: Arc<AtomicBool>,
    handle: JoinHandle<std::io::Result<Outcome>>,
    out: PathBuf,
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

/// Draw the window (when open) and pick up a finished export. Call once per frame.
pub fn show(ctx: &egui::Context, state: &mut AppState) {
    finish(ctx, state);
    let showing = state.export.open && state.doc.is_some();
    let just_opened = showing && !state.export.was_open;
    state.export.was_open = showing;
    if !showing {
        return;
    }
    state.refresh_sprites(ctx);
    let mut open = state.export.open;
    let mut start = false;
    // Centered on the canvas each time it opens; free to drag after that.
    let center = if state.canvas_rect.is_positive() { state.canvas_rect.center() } else { ctx.content_rect().center() };
    let mut window = egui::Window::new("Export animation").pivot(egui::Align2::CENTER_CENTER);
    window = if just_opened { window.current_pos(center) } else { window.default_pos(center) };
    window.open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
        if let Some(r) = &state.export.running {
            let done = r.done.load(Ordering::Relaxed);
            ui.label(format!("Exporting to {}", r.out.display()));
            ui.add(egui::ProgressBar::new(done as f32 / r.total.max(1) as f32).text(format!("frame {done} of {}", r.total)));
            if ui.button("Cancel").clicked() {
                r.cancel.store(true, Ordering::Relaxed);
            }
            return;
        }
        let s = &mut state.export.settings;
        ui.horizontal(|ui| {
            ui.label("Format");
            ui.radio_value(&mut s.format, Format::Gif, "GIF");
            ui.radio_value(&mut s.format, Format::Png, "PNG sequence");
        });
        ui.horizontal(|ui| {
            ui.label("Frame rate");
            ui.add(egui::DragValue::new(&mut s.fps).range(1..=60).suffix(" fps"));
        });
        ui.horizontal(|ui| {
            ui.label("Size");
            let mut percent = s.scale * 100.0;
            ui.add(egui::DragValue::new(&mut percent).range(10.0..=400.0).speed(1.0).suffix(" %"));
            s.scale = percent / 100.0;
        });
        ui.horizontal(|ui| {
            let mut transparent = s.background.is_none();
            ui.checkbox(&mut transparent, "Transparent background");
            match (transparent, &mut s.background) {
                (true, bg) => *bg = None,
                (false, None) => s.background = Some([255, 255, 255]),
                (false, Some(rgb)) => {
                    ui.color_edit_button_srgb(rgb);
                }
            }
        });
        ui.checkbox(&mut s.crop, "Crop to the moving figure")
            .on_hover_text("Off: every frame is the size of the whole drawing");

        let settings = *s;
        let poses = frame_poses(state, settings.fps);
        let doc = state.doc.as_ref().unwrap();
        let area = frame_area(&doc.sprites, &doc.skeleton, &poses, [doc.image.width(), doc.image.height()], settings.crop);
        let [w, h] = output_size(area, settings.scale);
        ui.separator();
        if doc.motion.is_empty() {
            ui.label("No keys yet: exports the current pose as a single frame.");
        }
        ui.label(format!("{} frames, {w}×{h} px", poses.len()));
        if settings.format == Format::Gif && settings.background.is_none() {
            ui.small("GIF has no soft edges: semi-transparent pixels become solid or clear.");
        }
        start = ui.button("Export…").clicked();
    });
    state.export.open = open || state.export.running.is_some();
    if start {
        begin(state);
    }
    if state.export.running.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// Ask where to save, then start rendering on a background thread.
fn begin(state: &mut AppState) {
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
    state.status = format!("Exporting {total} frames…");
    state.export.running = Some(Running { done, total, cancel, handle, out });
}

/// Report a finished export in the status line and close the window on success.
fn finish(ctx: &egui::Context, state: &mut AppState) {
    if !state.export.running.as_ref().is_some_and(|r| r.handle.is_finished()) {
        return;
    }
    let r = state.export.running.take().unwrap();
    let result = r.handle.join().unwrap_or_else(|_| Err(std::io::Error::other("the export crashed")));
    state.status = match result {
        Ok(Outcome::Done(n)) => {
            state.export.open = false;
            format!("Exported {n} frames to {}", r.out.display())
        }
        Ok(Outcome::Cancelled) => "Export cancelled".into(),
        Err(e) => format!("Export failed: {e}"),
    };
    ctx.request_repaint();
}
