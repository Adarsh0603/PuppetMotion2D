use crate::canvas::{Drag, View};
use crate::export::ExportUi;
use crate::unsaved::{Pending, Unsaved};
use anim_core::{
    Clip, Ease, History, KeyRef, Mask, Motion, Params, Pose, ProjectData, RectI, Skeleton, Sprite, build_sprites,
    foreground_mask, load_project, project, save_project, skeleton_masks,
};
use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions};
use image::RgbaImage;
use std::path::{Path, PathBuf};

/// Tint colors for part masks, cycled per part.
pub const PALETTE: [[u8; 3]; 12] = [
    [255, 80, 80],
    [80, 170, 255],
    [90, 220, 110],
    [255, 190, 40],
    [200, 100, 255],
    [40, 220, 220],
    [255, 130, 190],
    [160, 200, 60],
    [120, 120, 255],
    [255, 150, 90],
    [60, 180, 150],
    [220, 220, 90],
];

/// Undo steps kept per drawing.
const UNDO_LIMIT: usize = 200;

pub fn part_color(i: usize) -> Color32 {
    let [r, g, b] = PALETTE[i % PALETTE.len()];
    Color32::from_rgb(r, g, b)
}

/// The four steps of making an animation, shown as a bar across the top.
/// Each step has its own side panel and its own canvas tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Open a drawing and check the character is cut out cleanly.
    Review,
    /// Place the bones (and later the mesh).
    Rig,
    /// Pose on the timeline.
    Animate,
    /// Preview and write the animation to files.
    Export,
}

impl Step {
    pub const ALL: [Step; 4] = [Step::Review, Step::Rig, Step::Animate, Step::Export];

    pub fn label(self) -> &'static str {
        match self {
            Step::Review => "Review",
            Step::Rig => "Rig",
            Step::Animate => "Animate",
            Step::Export => "Export",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Step::Review => "Check the character is cut out cleanly, ready to rig",
            Step::Rig => "Place the bones that move the character",
            Step::Animate => "Pose the character over time",
            Step::Export => "Save the animation as a GIF or frames",
        }
    }

    /// Steps that show the posed character (and its motion) instead of the drawing.
    pub fn shows_pose(self) -> bool {
        matches!(self, Step::Animate | Step::Export)
    }
}

/// What is selected in the Rig step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkelSel {
    None,
    Joint(usize),
    Part(usize),
}

/// Everything the user has placed: what undo/redo saves and restores.
/// Masks are not included; they are recomputed from this.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub skeleton: Skeleton,
    pub pose: Pose,
    pub motion: Motion,
}

/// Keys being dragged along the timeline.
pub struct KeyDrag {
    /// Time of the key under the pointer when the drag began.
    grab_t: f32,
    /// The selection and the motion as they were when the drag began.
    sel: Vec<KeyRef>,
    motion: Motion,
    /// For the undo step recorded when the drag ends.
    before: Option<Snapshot>,
}

/// A loaded drawing and everything derived from it.
pub struct Doc {
    pub image: RgbaImage,
    pub texture: TextureHandle,
    pub fg: Mask,
    pub skeleton: Skeleton,
    /// Per-part masks from the skeleton.
    pub skel_masks: Vec<Mask>,
    pub overlay: Option<TextureHandle>,
    pub overlay_dirty: bool,
    pub history: History<Snapshot>,
    /// File name without extension, used to name exported parts.
    pub name: String,
    pub pose: Pose,
    /// Keyed animation. In Animate and Export the shown pose is sampled from it at the playhead.
    pub motion: Motion,
    /// Cut-out parts and their textures (None for an empty part), rebuilt when
    /// `sprites_dirty` is set.
    pub sprites: Vec<Sprite>,
    pub sprite_tex: Vec<Option<TextureHandle>>,
    pub sprites_dirty: bool,
    /// The original image file, kept byte for byte so projects store it unchanged.
    pub image_bytes: Vec<u8>,
    pub image_ext: String,
    /// Where this was last saved as a project; None until the first save.
    pub project_path: Option<PathBuf>,
    /// Changes since the last save (or since opening).
    pub dirty: bool,
}

impl Doc {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            skeleton: self.skeleton.clone(),
            pose: self.pose.clone(),
            motion: self.motion.clone(),
        }
    }

    /// Put back a snapshot's skeleton, pose and motion (masks are recomputed by the caller).
    fn restore(&mut self, s: Snapshot) {
        self.skeleton = s.skeleton;
        self.pose = s.pose;
        self.motion = s.motion;
    }

    /// Back to the pose as drawn. Needed whenever bones are added or removed,
    /// since pose angles are stored by part number.
    pub fn reset_pose(&mut self) {
        self.pose = Pose::rest(self.skeleton.parts.len());
    }

    pub fn size(&self) -> egui::Vec2 {
        egui::vec2(self.image.width() as f32, self.image.height() as f32)
    }

    pub fn recompute_skeleton(&mut self) {
        self.skel_masks = skeleton_masks(&self.fg, &self.skeleton).masks;
        self.overlay_dirty = true;
        self.sprites_dirty = true;
    }

    /// Place the template skeleton over the figure's bounding box.
    pub fn fit_skeleton(&mut self) {
        let (w, h) = self.image.dimensions();
        let bbox = self.fg.bbox().unwrap_or(RectI::new(0, 0, w as i32, h as i32));
        self.skeleton = Skeleton::person_front(bbox);
        self.reset_pose();
        self.recompute_skeleton();
    }
}

pub struct AppState {
    pub doc: Option<Doc>,
    /// Which of the four steps is showing. Change it with `set_step`.
    pub step: Step,
    pub params: Params,
    pub view: View,
    pub drag: Drag,
    /// State before the current drag, recorded for undo when the drag ends.
    pub drag_before: Option<Snapshot>,
    /// State before a typed or dragged field edit (a name, a limit), recorded
    /// as one undo step when the field is let go.
    pub field_before: Option<Snapshot>,
    /// Pose mode: dragging a hand or foot moves the whole limb.
    pub ik: bool,
    /// Current time on the timeline (seconds).
    pub playhead: f32,
    pub playing: bool,
    pub looping: bool,
    /// Space was used to pan during this press, so releasing it shouldn't toggle playback.
    pub space_panned: bool,
    /// Pose drags save keys at the playhead right away (the record button).
    pub auto_key: bool,
    /// The shown pose was changed with auto-key off and hasn't been keyed yet.
    pub unkeyed_pose: bool,
    /// Playhead time the shown pose was last sampled at (to notice scrubbing).
    pub sampled_at: f32,
    /// Keys selected on the timeline (for copy, paste, delete).
    pub key_sel: Vec<KeyRef>,
    /// Keys being dragged along the timeline, if any.
    pub key_drag: Option<KeyDrag>,
    /// Last copied keys, used if the system clipboard has something else on it.
    pub clip: Option<Clip>,
    /// Selected joint or bone (Rig step).
    pub skel_sel: SkelSel,
    /// Rig step: tint each part's pixels in its colour.
    pub show_parts: bool,
    /// Joint disc radius as a multiple of the child part's thickness (0 = no discs).
    pub overlap: f32,
    /// Animate step: show the original drawing faintly behind the posed parts.
    pub show_ghost: bool,
    /// Animate step: draw bones and joint dots over the parts (B toggles).
    pub show_bones: bool,
    pub status: String,
    /// Set when a new image is loaded so the canvas fits it on the next frame.
    pub fit_pending: bool,
    /// Export settings and any export in progress.
    pub export: ExportUi,
    /// The "Save changes?" prompt, autosave, and crash recovery.
    pub unsaved: Unsaved,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            doc: None,
            step: Step::Review,
            params: Params::default(),
            view: View::default(),
            drag: Drag::None,
            drag_before: None,
            field_before: None,
            ik: true,
            playhead: 0.0,
            playing: false,
            looping: true,
            space_panned: false,
            auto_key: false,
            unkeyed_pose: false,
            sampled_at: 0.0,
            key_sel: Vec::new(),
            key_drag: None,
            clip: None,
            skel_sel: SkelSel::None,
            show_parts: true,
            overlap: 1.0,
            show_ghost: false,
            show_bones: true,
            status: "Open or drop a drawing to start.".into(),
            fit_pending: false,
            export: ExportUi::default(),
            unsaved: Unsaved::new(crate::unsaved::recovery_dir()),
        }
    }
}

impl AppState {
    /// Open an image or a `.pm2d` project, chosen by the file extension.
    pub fn open(&mut self, ctx: &egui::Context, path: &Path) {
        let ext = path.extension().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
        if ext == project::EXTENSION {
            self.open_project(ctx, path);
        } else {
            self.open_image(ctx, path);
        }
    }

    fn open_image(&mut self, ctx: &egui::Context, path: &Path) {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                self.status = format!("Could not open {}: {e}", path.display());
                return;
            }
        };
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let ext = path.extension().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
        let Some(mut doc) = self.load_doc(ctx, &name, bytes, ext) else { return };
        doc.fit_skeleton();
        self.set_doc(doc);
        // A new drawing starts at Import, so the cutout can be checked first.
        self.set_step(Step::Review);
    }

    fn open_project(&mut self, ctx: &egui::Context, path: &Path) {
        let loaded = match load_project(path) {
            Ok(l) => l,
            Err(e) => {
                self.status = format!("Could not open {}: {e}", path.display());
                return;
            }
        };
        let data = loaded.data;
        // Settings first: the figure is found using the project's own cutout settings.
        self.params = data.params;
        self.overlap = data.overlap;
        let Some(mut doc) = self.load_doc(ctx, &data.image_name, loaded.image_bytes, loaded.image_ext) else { return };
        doc.restore(Snapshot { skeleton: data.skeleton, pose: data.pose, motion: data.motion });
        doc.recompute_skeleton();
        doc.project_path = Some(path.to_path_buf());
        self.status = format!("Opened project {}", path.file_name().unwrap_or_default().to_string_lossy());
        // A project picks up where it was left: animating if it has keys, else rigging.
        let step = if doc.motion.is_empty() { Step::Rig } else { Step::Animate };
        self.set_doc(doc);
        self.set_step(step);
    }

    /// Switch to `step`, ending any drag and refreshing what the canvas shows.
    pub fn set_step(&mut self, step: Step) {
        if step == self.step {
            return;
        }
        if self.step.shows_pose() && !step.shows_pose() {
            self.pause();
        }
        if step == Step::Export
            && let Some(doc) = &self.doc
        {
            // Export at the timeline's frame rate unless changed here.
            self.export.settings.fps = doc.motion.fps;
        }
        self.step = step;
        self.drag = Drag::None;
        self.drag_before = None;
        if let Some(doc) = &mut self.doc {
            doc.overlay_dirty = true;
        }
    }

    /// Decode an image and build a fresh document for it (empty skeleton).
    fn load_doc(&mut self, ctx: &egui::Context, name: &str, bytes: Vec<u8>, ext: String) -> Option<Doc> {
        let image = match image::load_from_memory(&bytes) {
            Ok(img) => img.to_rgba8(),
            Err(e) => {
                self.status = format!("Could not read the image {name}: {e}");
                return None;
            }
        };
        let max_side = ctx.input(|i| i.max_texture_side) as u32;
        if image.width() > max_side || image.height() > max_side {
            self.status = format!(
                "Image is {}×{}, larger than the GPU limit of {max_side}px per side.",
                image.width(),
                image.height()
            );
            return None;
        }
        let texture = ctx.load_texture("drawing", to_color_image(&image), TextureOptions::LINEAR);
        let fg = foreground_mask(&image, &self.params);
        self.status = format!("Opened {name} ({}×{})", image.width(), image.height());
        let stem = Path::new(name).file_stem().unwrap_or_default().to_string_lossy().into_owned();
        Some(Doc {
            image,
            texture,
            fg,
            skeleton: Skeleton::empty(),
            skel_masks: Vec::new(),
            overlay: None,
            overlay_dirty: true,
            history: History::new(UNDO_LIMIT),
            name: stem,
            pose: Pose::default(),
            motion: Motion::default(),
            sprites: Vec::new(),
            sprite_tex: Vec::new(),
            sprites_dirty: true,
            image_bytes: bytes,
            image_ext: ext,
            project_path: None,
            dirty: false,
        })
    }

    /// Advance playback by this frame's time and keep the shown pose in step
    /// with the motion. Call once per frame.
    pub fn tick(&mut self, ctx: &egui::Context) {
        let Some(doc) = &mut self.doc else { return };
        if self.playing {
            // The playhead runs on a smooth clock; the pose shown is held per frame (below).
            self.playhead += ctx.input(|i| i.stable_dt).min(0.1);
            let end = doc.motion.duration.max(0.01);
            if self.playhead >= end {
                if self.looping {
                    self.playhead %= end;
                } else {
                    self.playhead = doc.motion.snap(end).min(end);
                    self.playing = false;
                }
            }
            ctx.request_repaint();
        }
        // A pose made with auto-key off stays on screen until the playhead
        // moves or playback runs; then the motion takes over again.
        if self.playing || (self.playhead - self.sampled_at).abs() > 1e-6 {
            self.unkeyed_pose = false;
        }
        // While a pose drag is in progress the dragged pose wins; otherwise
        // show the motion at the playhead (if there is one).
        let posing = matches!(self.drag, Drag::PoseRotate { .. } | Drag::PoseMove);
        if self.step.shows_pose() && !posing && !self.unkeyed_pose && !doc.motion.is_empty() {
            let t = if self.playing {
                // Hold each frame until the next one starts, exactly like the export.
                (self.playhead * doc.motion.fps.max(1) as f32).floor() / doc.motion.fps.max(1) as f32
            } else {
                self.playhead
            };
            doc.pose = doc.motion.sample(t, &doc.skeleton);
        }
        self.sampled_at = self.playhead;
    }

    /// The playhead on its nearest whole frame: where keys and pastes go.
    pub fn key_time(&self) -> f32 {
        self.doc.as_ref().map_or(self.playhead, |d| d.motion.snap(self.playhead))
    }

    /// Stop playback, leaving the playhead on the nearest frame.
    pub fn pause(&mut self) {
        self.playing = false;
        self.playhead = self.key_time();
    }

    /// Move the playhead one frame back (`dir < 0`) or forward, pausing playback.
    pub fn step_frame(&mut self, dir: i32) {
        let Some(doc) = &self.doc else { return };
        let (frame, last) = (doc.motion.frame_at(self.playhead), doc.motion.frame_at(doc.motion.duration));
        let frame = (frame + dir.signum() as i64).clamp(0, last);
        let t = (frame as f32 / doc.motion.fps.max(1) as f32).min(doc.motion.duration);
        self.playing = false;
        self.playhead = t;
    }

    /// Save the current pose at the playhead: keys for the parts that differ
    /// from what the motion already has there (all changed parts if there is
    /// no motion yet). Bound to K and the "Key pose" button.
    pub fn key_pose(&mut self) {
        let before = self.snapshot();
        let t = self.key_time();
        let Some(doc) = &mut self.doc else { return };
        let current = doc.motion.sample(t, &doc.skeleton);
        doc.motion.set_keys(t, &doc.skeleton, &doc.pose, Some(&current));
        let changed = before.as_ref().is_none_or(|b| b.motion != doc.motion);
        self.status = if changed {
            format!("Keyed pose at frame {} ({t:.2} s)", doc.motion.frame_at(t))
        } else {
            "Nothing to key: the pose already matches the motion here".into()
        };
        self.unkeyed_pose = false;
        self.commit(before);
    }

    pub fn toggle_play(&mut self) {
        if self.playing {
            self.pause();
            return;
        }
        let Some(doc) = &self.doc else { return };
        if !self.looping && self.playhead >= doc.motion.duration {
            self.playhead = 0.0;
        }
        self.playing = true;
    }

    /// Key every part at the playhead, as currently posed.
    pub fn key_all(&mut self) {
        let before = self.snapshot();
        let t = self.key_time();
        let Some(doc) = &mut self.doc else { return };
        doc.motion.set_keys(t, &doc.skeleton, &doc.pose, None);
        self.commit(before);
    }

    /// Start dragging the selected keys along the timeline; `grab_t` is the
    /// time of the key under the pointer.
    pub fn begin_key_drag(&mut self, grab_t: f32) {
        let Some(doc) = &self.doc else { return };
        if self.key_sel.is_empty() {
            return;
        }
        self.key_drag =
            Some(KeyDrag { grab_t, sel: self.key_sel.clone(), motion: doc.motion.clone(), before: self.snapshot() });
        self.playing = false;
    }

    /// Move the dragged keys so the grabbed one sits on the frame nearest `t`.
    /// Each call redoes the whole move from the keys as they were when the
    /// drag began, so passing over other keys doesn't eat them.
    pub fn update_key_drag(&mut self, t: f32) {
        let (Some(drag), Some(doc)) = (&self.key_drag, &mut self.doc) else { return };
        let earliest = drag.sel.iter().map(|r| r.t).fold(f32::INFINITY, f32::min);
        let dt = (drag.motion.snap(t) - drag.grab_t).max(-earliest);
        doc.motion = drag.motion.clone();
        self.key_sel = doc.motion.move_keys(&drag.sel, dt);
        self.playhead = drag.grab_t + dt;
        self.unkeyed_pose = false;
    }

    /// Finish a key drag as one undo step.
    pub fn end_key_drag(&mut self) {
        let Some(drag) = self.key_drag.take() else { return };
        if let Some(doc) = &self.doc {
            let frames = doc.motion.frame_at(self.playhead) - doc.motion.frame_at(drag.grab_t);
            if frames != 0 {
                self.status = format!("Moved {} keys by {frames:+} frames", self.key_sel.len());
            }
        }
        self.commit(drag.before);
    }

    /// The ease shared by every selected key, None if they differ (or none are selected).
    pub fn selected_ease(&self) -> Option<Ease> {
        let doc = self.doc.as_ref()?;
        let mut eases = self.key_sel.iter().filter_map(|r| doc.motion.key(r)).map(|k| k.ease);
        let first = eases.next()?;
        eases.all(|e| e == first).then_some(first)
    }

    /// Set how the selected keys travel to their next key, as one undo step.
    pub fn set_selected_ease(&mut self, ease: Ease) {
        let before = self.snapshot();
        let Some(doc) = &mut self.doc else { return };
        doc.motion.set_ease(&self.key_sel, ease);
        self.status = format!("{} keys set to {ease:?}", self.key_sel.len());
        self.commit(before);
    }

    /// Select every key on the timeline (Ctrl+A in Pose mode).
    pub fn select_all_keys(&mut self) {
        let Some(doc) = &self.doc else { return };
        self.key_sel = doc.motion.key_times().into_iter().flat_map(|t| doc.motion.keys_at(t)).collect();
        if !self.key_sel.is_empty() {
            self.status = format!("Selected all {} keys", self.key_sel.len());
        }
    }

    /// Copy the selected keys, to the app and to the system clipboard (as
    /// text, so they can be pasted into another project too).
    pub fn copy_keys(&mut self, ctx: &egui::Context) {
        let Some(doc) = &self.doc else { return };
        if self.key_sel.is_empty() {
            return;
        }
        let clip = doc.motion.copy(&self.key_sel);
        ctx.copy_text(clip.to_clipboard_text());
        self.status = format!("Copied {} keys", clip.keys.len());
        self.clip = Some(clip);
    }

    /// Copy the selected keys, then delete them (one undo step).
    pub fn cut_keys(&mut self, ctx: &egui::Context) {
        if self.key_sel.is_empty() {
            return;
        }
        self.copy_keys(ctx);
        let n = self.key_sel.len();
        self.delete_selected_keys();
        self.status = format!("Cut {n} keys");
    }

    /// Paste keys at the playhead: `text` from the system clipboard if it holds
    /// copied keys, else the last keys copied in this app. The pasted keys
    /// become the selection.
    pub fn paste_keys(&mut self, text: Option<&str>) {
        let Some(clip) = text.and_then(Clip::from_clipboard_text).or_else(|| self.clip.clone()) else { return };
        if clip.is_empty() {
            return;
        }
        let before = self.snapshot();
        let t = self.key_time();
        let Some(doc) = &mut self.doc else { return };
        self.key_sel = doc.motion.paste(&clip, t);
        self.status = format!("Pasted {} keys at frame {}", clip.keys.len(), doc.motion.frame_at(t));
        self.commit(before);
    }

    pub fn delete_selected_keys(&mut self) {
        if self.key_sel.is_empty() {
            return;
        }
        let before = self.snapshot();
        let Some(doc) = &mut self.doc else { return };
        doc.motion.remove(&self.key_sel);
        self.key_sel.clear();
        self.commit(before);
    }

    /// Move the playhead to the previous (`dir < 0`) or next key.
    pub fn jump_key(&mut self, dir: i32) {
        let Some(doc) = &self.doc else { return };
        let times = doc.motion.key_times();
        let eps = anim_core::motion::KEY_EPSILON;
        let target = if dir < 0 {
            times.iter().rev().find(|&&t| t < self.playhead - eps)
        } else {
            times.iter().find(|&&t| t > self.playhead + eps)
        };
        if let Some(&t) = target {
            self.playhead = t;
            self.playing = false;
        }
    }

    fn set_doc(&mut self, doc: Doc) {
        self.playhead = 0.0;
        self.unkeyed_pose = false;
        self.key_sel.clear();
        self.playing = false;
        self.doc = Some(doc);
        self.skel_sel = SkelSel::None;
        self.drag = Drag::None;
        self.drag_before = None;
        self.field_before = None;
        self.fit_pending = true;
    }

    /// Save to the current project file, or ask where if it has none yet.
    pub fn save(&mut self) {
        match self.doc.as_ref().and_then(|d| d.project_path.clone()) {
            Some(path) => self.save_to(&path),
            None => self.save_as(),
        }
    }

    pub fn save_as(&mut self) {
        let Some(doc) = &self.doc else { return };
        let Some(mut path) = rfd::FileDialog::new()
            .set_title("Save project")
            .add_filter("PuppetMotion project", &[project::EXTENSION])
            .set_file_name(format!("{}.{}", doc.name, project::EXTENSION))
            .save_file()
        else {
            return;
        };
        if path.extension().is_none_or(|e| e != project::EXTENSION) {
            path.set_extension(project::EXTENSION);
        }
        self.save_to(&path);
    }

    /// What a saved project holds for the open drawing.
    pub fn project_data(&self) -> Option<ProjectData> {
        let doc = self.doc.as_ref()?;
        Some(ProjectData {
            format_version: project::FORMAT_VERSION,
            image_name: format!("{}.{}", doc.name, doc.image_ext),
            params: self.params,
            skeleton: doc.skeleton.clone(),
            pose: doc.pose.clone(),
            overlap: self.overlap,
            motion: doc.motion.clone(),
        })
    }

    pub fn save_to(&mut self, path: &Path) {
        let Some(data) = self.project_data() else { return };
        let Some(doc) = &mut self.doc else { return };
        self.status = match save_project(path, &data, &doc.image_bytes, &doc.image_ext) {
            Ok(()) => {
                doc.project_path = Some(path.to_path_buf());
                doc.dirty = false;
                // Saved for real, so the crash-recovery copy isn't needed any more.
                self.unsaved.clear_recovery();
                format!("Saved {}", path.file_name().unwrap_or_default().to_string_lossy())
            }
            Err(e) => format!("Save failed: {e}"),
        };
    }

    /// Open `path`, first asking to save if the current drawing has unsaved changes.
    pub fn request_open(&mut self, ctx: &egui::Context, path: &Path) {
        if self.doc.as_ref().is_some_and(|d| d.dirty) {
            self.unsaved.prompt = Some(Pending::Open(path.to_path_buf()));
        } else {
            self.open(ctx, path);
        }
    }

    /// Show the Open dialog, first asking to save if there are unsaved changes.
    pub fn request_open_dialog(&mut self, ctx: &egui::Context) {
        if self.doc.as_ref().is_some_and(|d| d.dirty) {
            self.unsaved.prompt = Some(Pending::OpenDialog);
        } else {
            self.open_dialog(ctx);
        }
    }

    pub fn open_dialog(&mut self, ctx: &egui::Context) {
        let images = ["png", "jpg", "jpeg", "webp", "bmp", "gif"];
        let all: Vec<&str> = images.iter().copied().chain([project::EXTENSION]).collect();
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Images and projects", &all)
            .add_filter("PuppetMotion project", &[project::EXTENSION])
            .add_filter("Images", &images)
            .pick_file()
        {
            self.open(ctx, &path);
        }
    }

    /// Window title: app name, drawing name, and a * when there are unsaved changes.
    pub fn title(&self) -> String {
        match &self.doc {
            Some(d) => format!("PuppetMotion 2D — {}{}", d.name, if d.dirty { " *" } else { "" }),
            None => "PuppetMotion 2D".into(),
        }
    }

    pub fn snapshot(&self) -> Option<Snapshot> {
        self.doc.as_ref().map(Doc::snapshot)
    }

    /// Record a finished edit for undo. `before` is the state it started from.
    pub fn commit(&mut self, before: Option<Snapshot>) {
        if let (Some(doc), Some(before)) = (&mut self.doc, before) {
            // A bone's keys are stored under its name, so they follow a finished rename.
            let (old, new) = (&before.skeleton.parts, &doc.skeleton.parts);
            if old.len() == new.len() {
                for (o, n) in old.iter().zip(new) {
                    if o.name != n.name && !doc.motion.tracks.contains_key(&n.name) {
                        doc.motion.rename_track(&o.name, &n.name);
                    }
                }
            }
            let after = doc.snapshot();
            if before != after {
                doc.dirty = true;
            }
            doc.history.record(before, &after);
        }
    }

    /// Put back a snapshot and recompute everything derived from it.
    pub fn restore(&mut self, s: Snapshot) {
        if let Some(doc) = &mut self.doc {
            doc.dirty = true;
            doc.restore(s);
            doc.recompute_skeleton();
        }
        self.fix_selection();
    }

    pub fn undo(&mut self) {
        let Some(doc) = &mut self.doc else { return };
        let current = doc.snapshot();
        if let Some(prev) = doc.history.undo(current) {
            self.restore(prev);
        }
    }

    pub fn redo(&mut self) {
        let Some(doc) = &mut self.doc else { return };
        let current = doc.snapshot();
        if let Some(next) = doc.history.redo(current) {
            self.restore(next);
        }
    }

    /// Drop selections that no longer point at anything.
    fn fix_selection(&mut self) {
        let Some(doc) = &self.doc else { return };
        self.key_sel.retain(|r| doc.motion.has_key(r));
        let valid = match self.skel_sel {
            SkelSel::None => true,
            SkelSel::Joint(j) => j < doc.skeleton.joints.len(),
            SkelSel::Part(p) => p < doc.skeleton.parts.len(),
        };
        if !valid {
            self.skel_sel = SkelSel::None;
        }
    }

    /// Run `edit` on the skeleton as one undoable step, then recompute masks.
    /// The pose goes back to rest, since parts may have been added or removed.
    pub fn edit_skeleton(&mut self, edit: impl FnOnce(&mut Skeleton)) {
        let before = self.snapshot();
        let Some(doc) = &mut self.doc else { return };
        edit(&mut doc.skeleton);
        doc.reset_pose();
        doc.recompute_skeleton();
        self.commit(before);
        self.fix_selection();
    }

    /// Rebuild the cut-out part images if the parts or the overlap changed.
    pub fn refresh_sprites(&mut self, ctx: &egui::Context) {
        let Some(doc) = &mut self.doc else { return };
        if !doc.sprites_dirty {
            return;
        }
        doc.sprites_dirty = false;
        doc.sprites = build_sprites(&doc.image, &doc.fg, &doc.skeleton, &doc.skel_masks, self.overlap);
        doc.sprite_tex = doc
            .sprites
            .iter()
            .map(|s| {
                (s.image.width() > 0)
                    .then(|| ctx.load_texture(format!("part {}", s.part), to_color_image(&s.image), TextureOptions::LINEAR))
            })
            .collect();
    }

    /// Delete the selected joint or bone (Rig step).
    pub fn delete_skel_selection(&mut self) {
        match std::mem::replace(&mut self.skel_sel, SkelSel::None) {
            SkelSel::Joint(j) => self.edit_skeleton(|s| s.remove_joint(j)),
            SkelSel::Part(p) => self.edit_skeleton(|s| s.remove_part(p)),
            SkelSel::None => {}
        }
    }

    /// Rebuild the tint overlay texture if anything changed. Import dims the
    /// background so the found character stands out; Rig tints each part.
    pub fn refresh_overlay(&mut self, ctx: &egui::Context) {
        let Some(doc) = &mut self.doc else { return };
        if !doc.overlay_dirty {
            return;
        }
        doc.overlay_dirty = false;
        let mut img = RgbaImage::new(doc.image.width(), doc.image.height());
        match self.step {
            Step::Review => doc.fg.inverted().paint_onto(&mut img, [30, 30, 36, 210]),
            Step::Rig if self.show_parts => {
                for (i, m) in doc.skel_masks.iter().enumerate() {
                    let [r, g, b] = PALETTE[i % PALETTE.len()];
                    m.paint_onto(&mut img, [r, g, b, 140]);
                }
            }
            _ => {}
        }
        let color = to_color_image(&img);
        match &mut doc.overlay {
            Some(tex) => tex.set(color, TextureOptions::NEAREST),
            None => doc.overlay = Some(ctx.load_texture("overlay", color, TextureOptions::NEAREST)),
        }
    }

    /// The bar across the top: file buttons, the four steps, undo/redo.
    pub fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let has_doc = self.doc.is_some();
            if ui.button("Open…").on_hover_text("A drawing or a project (Ctrl+O)").clicked() {
                self.request_open_dialog(ui.ctx());
            }
            if ui.add_enabled(has_doc, egui::Button::new("Save")).on_hover_text("Ctrl+S").clicked() {
                self.save();
            }
            if ui.add_enabled(has_doc, egui::Button::new("Save As…")).on_hover_text("Ctrl+Shift+S").clicked() {
                self.save_as();
            }
            ui.separator();
            for (i, step) in Step::ALL.into_iter().enumerate() {
                if i > 0 {
                    ui.label(egui::RichText::new("›").weak());
                }
                let text = egui::RichText::new(step.label()).size(15.0).strong();
                let button = egui::Button::new(text).selected(self.step == step);
                // Everything after Review needs a drawing.
                let enabled = has_doc || step == Step::Review;
                if ui.add_enabled(enabled, button).on_hover_text(step.hint()).clicked() {
                    self.set_step(step);
                }
            }
            ui.separator();
            let (can_undo, can_redo) = self.doc.as_ref().map_or((false, false), |d| (d.history.can_undo(), d.history.can_redo()));
            if ui.add_enabled(can_undo, egui::Button::new("⟲ Undo")).on_hover_text("Ctrl+Z").clicked() {
                self.undo();
            }
            if ui.add_enabled(can_redo, egui::Button::new("⟳ Redo")).on_hover_text("Ctrl+Shift+Z").clicked() {
                self.redo();
            }
            if self.doc.as_ref().is_some_and(|d| d.dirty) {
                ui.label(egui::RichText::new("● unsaved").color(Color32::from_rgb(255, 190, 40)))
                    .on_hover_text("Changes since the last save (Ctrl+S)");
            }
        });
    }

    /// The line along the bottom of the window: what just happened.
    pub fn status_bar(&self, ui: &mut egui::Ui) {
        ui.label(&self.status);
    }

    /// The side panel: the current step's title and controls.
    pub fn side_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading(self.step.label());
        ui.label(egui::RichText::new(self.step.hint()).weak());
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match self.step {
            Step::Review => self.review_section(ui),
            Step::Rig => self.rig_section(ui),
            Step::Animate => self.animate_section(ui),
            Step::Export => crate::export::panel(ui, self),
        });
    }

    /// The shortcut list at the bottom of a step's panel: a little larger than
    /// egui's small text so it's easy to read.
    fn shortcut_help(ui: &mut egui::Ui, text: &str) {
        ui.label(egui::RichText::new(text).size(11.5).weak());
    }

    fn review_section(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = &self.doc else {
            ui.label("Open a drawing of one character on a plain background. A PNG with a transparent background works best.");
            ui.add_space(6.0);
            let open = egui::Button::new(egui::RichText::new("Open a drawing…").size(15.0).strong());
            if ui.add(open.min_size(egui::vec2(ui.available_width(), 30.0))).clicked() {
                self.request_open_dialog(ui.ctx());
            }
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Or drop a file on the window. Saved projects (.pm2d) open the same way.").weak());
            return;
        };
        ui.label(format!("{}  ·  {}×{} px", doc.name, doc.image.width(), doc.image.height()));
        ui.add_space(4.0);
        ui.label("Your character stays bright and the background is dimmed.");
        ui.label(
            egui::RichText::new("If some background is included, or part of the character is dimmed, fine-tune the cutout below.")
                .weak(),
        );
        ui.add_space(4.0);
        let before = self.params;
        egui::CollapsingHeader::new("Fine-tune cutout").default_open(false).show(ui, |ui| {
            ui.add(egui::Slider::new(&mut self.params.threshold, 5.0..=150.0).text("background tolerance"))
                .on_hover_text("How different from the background a colour must be to count as the character");
            ui.add(egui::Slider::new(&mut self.params.gap_close, 0..=20).text("close outline gaps (px)"))
                .on_hover_text("Seals small breaks in a sketchy outline so the background can't leak inside");
            ui.add(egui::Slider::new(&mut self.params.min_area, 0..=5000).logarithmic(true).text("ignore specks under (px)"))
                .on_hover_text("Drops tiny dots of dirt or noise");
            ui.add(egui::Slider::new(&mut self.params.corner_patch, 2..=64).text("corner sample (px)"))
                .on_hover_text("The background colour is read from squares this size in each corner");
            if ui.button("Reset").clicked() {
                self.params = Params::default();
            }
        });
        if self.params != before
            && let Some(doc) = &mut self.doc
        {
            doc.fg = foreground_mask(&doc.image, &self.params);
            doc.dirty = true;
            doc.recompute_skeleton();
        }
    }

    fn animate_section(&mut self, ui: &mut egui::Ui) {
        if self.doc.is_none() {
            return;
        }
        ui.label("Drag the white dots to pose. Press K (or Keep pose) to save the pose at this frame, then move along the timeline and pose again.");
        ui.add_space(4.0);
        if ui.button("Reset pose").on_hover_text("Back to as drawn (removes the keys at this frame)").clicked() {
            let before = self.snapshot();
            let t = self.key_time();
            if let Some(doc) = &mut self.doc {
                doc.motion.remove_keys_at(t);
                doc.reset_pose();
            }
            self.commit(before);
        }
        ui.checkbox(&mut self.ik, "Move whole limbs (IK)")
            .on_hover_text("Dragging a hand or foot bends the elbow or knee to follow");
        ui.checkbox(&mut self.show_bones, "Show bones").on_hover_text("B: hide them to watch just the animation");
        ui.checkbox(&mut self.show_ghost, "Show original drawing behind");
        egui::CollapsingHeader::new("Fine-tune joints").default_open(false).show(ui, |ui| {
            let overlap = self.overlap;
            ui.add(egui::Slider::new(&mut self.overlap, 0.0..=2.0).text("joint fill"))
                .on_hover_text("How big a round patch each part keeps at its joints, so turning a part leaves no gap");
            if overlap != self.overlap
                && let Some(doc) = &mut self.doc
            {
                doc.sprites_dirty = true;
                doc.dirty = true;
            }
        });
        ui.separator();
        Self::shortcut_help(
            ui,
            "Drag a hand or foot: the whole limb follows (IK)\n\
             Drag any other bone end: rotate that part (children follow)\n\
             Drag the root's start dot: move the whole figure\n\
             B: show/hide bones   Space: play/pause   K: key pose\n\
             , and .: previous / next frame   Ctrl+A: select all keys\n\
             Ctrl+Z: undo   Ctrl+Shift+Z: redo\n\
             Middle-drag or Space+drag: pan   Scroll: zoom   F: fit",
        );
    }

    fn rig_section(&mut self, ui: &mut egui::Ui) {
        if self.doc.is_none() {
            return;
        }
        ui.label("Drag the dots onto the character's joints. Each coloured area moves with its bone.");
        ui.add_space(4.0);
        let show_parts = self.show_parts;
        ui.checkbox(&mut self.show_parts, "Show part colours");
        if show_parts != self.show_parts
            && let Some(doc) = &mut self.doc
        {
            doc.overlay_dirty = true;
        }
        ui.horizontal(|ui| {
            if ui.button("Person template").on_hover_text("Replace with a stick person fitted to the figure").clicked() {
                let before = self.snapshot();
                if let Some(doc) = &mut self.doc {
                    doc.fit_skeleton();
                }
                self.commit(before);
                self.skel_sel = SkelSel::None;
            }
            if ui.button("Clear").on_hover_text("Remove all bones and start from scratch").clicked() {
                self.edit_skeleton(|s| *s = Skeleton::empty());
            }
        });

        let doc = self.doc.as_ref().unwrap();
        if doc.skeleton.parts.is_empty() {
            ui.label("No bones. Shift+drag on the drawing to add one.");
        }
        let mut clicked = None;
        for (i, depth) in doc.skeleton.tree_order() {
            let text = egui::RichText::new(&doc.skeleton.parts[i].name).color(part_color(i));
            ui.horizontal(|ui| {
                ui.add_space(16.0 * depth as f32);
                if ui.selectable_label(self.skel_sel == SkelSel::Part(i), text).clicked() {
                    clicked = Some(i);
                }
            });
        }
        if let Some(i) = clicked {
            self.skel_sel = SkelSel::Part(i);
        }

        // Rename / delete the selection.
        let sel = self.skel_sel;
        if sel != SkelSel::None {
            ui.separator();
            let label = if matches!(sel, SkelSel::Joint(_)) { "Joint name" } else { "Bone name" };
            let before = self.snapshot();
            let doc = self.doc.as_mut().unwrap();
            let name = match sel {
                SkelSel::Joint(j) => &mut doc.skeleton.joints[j].name,
                SkelSel::Part(p) => &mut doc.skeleton.parts[p].name,
                SkelSel::None => unreachable!(),
            };
            let resp = ui.horizontal(|ui| {
                ui.label(label);
                ui.text_edit_singleline(name)
            });
            let resp = resp.inner;
            // One undo step per rename: from gaining focus to losing it.
            if resp.gained_focus() {
                self.field_before = before;
            }
            if resp.lost_focus() {
                let before = self.field_before.take();
                self.commit(before);
            }
            if let SkelSel::Part(p) = sel {
                self.parent_picker(ui, p);
                self.limit_editor(ui, p);
            }
            let delete = if matches!(sel, SkelSel::Joint(_)) { "Delete joint and its bones" } else { "Delete bone" };
            if ui.button(delete).clicked() {
                self.delete_skel_selection();
            }
        }

        ui.separator();
        Self::shortcut_help(
            ui,
            "Drag a joint: move it\n\
             Shift+drag from a joint: grow a new bone\n\
             Shift+drag on empty space: start a new bone\n\
             Click a joint or bone: select it   Delete: remove it\n\
             Ctrl+Z: undo   Ctrl+Shift+Z: redo\n\
             Middle-drag or Space+drag: pan   Scroll: zoom   F: fit",
        );
    }

    /// Rotation limit for bone `p`: on/off plus min/max in degrees. A whole
    /// drag of a value (or one typed edit) is a single undo step.
    fn limit_editor(&mut self, ui: &mut egui::Ui, p: usize) {
        let before = self.snapshot();
        let Some(doc) = &mut self.doc else { return };
        let limit = doc.skeleton.parts[p].limit;
        let mut on = limit.is_some();
        let [mut lo, mut hi] = limit.map_or([-90.0, 90.0], |[a, b]| [a.to_degrees(), b.to_degrees()]);
        let mut started = false;
        let mut finished = false;
        ui.horizontal(|ui| {
            let r = ui.checkbox(&mut on, "Limit rotation");
            finished |= r.changed();
            started |= r.changed();
            if on {
                for (value, range) in [(&mut lo, -180.0..=0.0), (&mut hi, 0.0..=180.0)] {
                    let r = ui.add(egui::DragValue::new(value).range(range).suffix("°").speed(1.0));
                    started |= r.drag_started() || r.gained_focus();
                    finished |= r.drag_stopped() || r.lost_focus();
                }
            }
        })
        .response
        .on_hover_text("How far this bone can turn from the way it's drawn (relative to the bone it hangs from)");
        let new = on.then(|| [lo.to_radians(), hi.to_radians()]);
        if new != limit {
            doc.skeleton.parts[p].limit = new;
            // Keep the current pose inside the new range.
            let a = doc.pose.angle(p);
            doc.pose.set_angle(p, anim_core::clamp_to_limit(&doc.skeleton, p, a));
        }
        if started && self.field_before.is_none() {
            self.field_before = before;
        }
        if finished {
            let before = self.field_before.take();
            self.commit(before);
        }
    }

    /// "Attached to" dropdown: which part bone `p` hangs from. Its own
    /// descendants are left out so the hierarchy can't loop.
    fn parent_picker(&mut self, ui: &mut egui::Ui, p: usize) {
        let Some(doc) = &self.doc else { return };
        let skel = &doc.skeleton;
        let current = skel.parts[p].parent;
        let name = |q: Option<usize>| q.map_or("nothing (root)".to_string(), |q| skel.parts[q].name.clone());
        let excluded = skel.descendants(p);
        let options: Vec<Option<usize>> = std::iter::once(None)
            .chain((0..skel.parts.len()).filter(|&q| q != p && !excluded.contains(&q)).map(Some))
            .collect();
        let mut chosen = current;
        ui.horizontal(|ui| {
            ui.label("Attached to");
            egui::ComboBox::from_id_salt(("parent", p)).selected_text(name(current)).show_ui(ui, |ui| {
                for &q in &options {
                    ui.selectable_value(&mut chosen, q, name(q));
                }
            });
        });
        if chosen != current {
            self.edit_skeleton(|s| {
                s.set_parent(p, chosen);
            });
        }
    }

}

pub fn to_color_image(img: &RgbaImage) -> ColorImage {
    ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_sample() -> AppState {
        let ctx = egui::Context::default();
        let mut state = AppState::default();
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../testdata/synthetic_sketch.png");
        state.open(&ctx, &path);
        assert!(state.doc.is_some(), "{}", state.status);
        state
    }

    fn bone_count(state: &AppState) -> usize {
        state.doc.as_ref().unwrap().skeleton.parts.len()
    }

    #[test]
    fn skeleton_edits_undo_and_redo() {
        let mut state = open_sample();
        assert_eq!(bone_count(&state), 10, "starts with the person template");
        assert!(!state.doc.as_ref().unwrap().history.can_undo());

        // Grow a tail from the pelvis (joint 2), then delete the head bone.
        state.edit_skeleton(|s| {
            let tip = s.add_joint([300.0, 300.0]);
            s.add_part(2, tip);
        });
        state.skel_sel = SkelSel::Part(0);
        state.delete_skel_selection();
        assert_eq!(bone_count(&state), 10);
        assert_eq!(state.doc.as_ref().unwrap().skel_masks.len(), 10, "masks follow the skeleton");

        state.undo();
        assert_eq!(bone_count(&state), 11, "head is back");
        state.undo();
        assert_eq!(bone_count(&state), 10, "tail is gone");
        assert!(!state.doc.as_ref().unwrap().history.can_undo());

        state.redo();
        state.redo();
        assert_eq!(bone_count(&state), 10);
        let names: Vec<_> = state.doc.as_ref().unwrap().skeleton.parts.iter().map(|p| p.name.clone()).collect();
        assert!(!names.contains(&"head".to_string()) && names.contains(&"bone 1".to_string()));
    }

    #[test]
    fn pose_undo_and_reset_on_skeleton_edit() {
        let mut state = open_sample();
        let ctx = egui::Context::default();
        state.refresh_sprites(&ctx);
        let doc = state.doc.as_ref().unwrap();
        assert_eq!(doc.sprites.len(), 10);
        // The auto-placed template misses the raised right arm (its forearm
        // points where nothing is drawn), so not every part gets pixels.
        assert!(doc.sprite_tex.iter().filter(|t| t.is_some()).count() >= 8);

        // Pose the left arm as one undoable step (what a drag does).
        let before = state.snapshot();
        state.doc.as_mut().unwrap().pose.set_angle(2, 1.0);
        state.commit(before);
        state.undo();
        assert_eq!(state.doc.as_ref().unwrap().pose.angle(2), 0.0, "undo returns to rest");
        state.redo();
        assert_eq!(state.doc.as_ref().unwrap().pose.angle(2), 1.0);

        // Changing the bones resets the pose; undo brings back bones and pose together.
        state.edit_skeleton(|s| s.remove_part(0));
        assert!(state.doc.as_ref().unwrap().pose.angles.iter().all(|&a| a == 0.0));
        state.undo();
        assert_eq!(bone_count(&state), 10);
        assert_eq!(state.doc.as_ref().unwrap().pose.angle(2), 1.0);
    }

    #[test]
    fn save_and_reopen_project() {
        let mut state = open_sample();
        assert!(!state.doc.as_ref().unwrap().dirty, "freshly opened image is clean");

        // Pose, tweak a setting and a limit, then save.
        let before = state.snapshot();
        let doc = state.doc.as_mut().unwrap();
        doc.pose.set_angle(3, 0.8);
        doc.skeleton.parts[3].limit = Some([-0.5, 1.2]);
        state.commit(before);
        assert!(state.doc.as_ref().unwrap().dirty, "an edit marks it unsaved");
        state.params.gap_close = 5;
        state.overlap = 0.6;
        let path = std::env::temp_dir().join(format!("pm2d_app_test_{}.pm2d", std::process::id()));
        state.save_to(&path);
        assert!(!state.doc.as_ref().unwrap().dirty, "{}", state.status);

        // Reopen in a fresh app.
        let ctx = egui::Context::default();
        let mut fresh = AppState::default();
        fresh.open(&ctx, &path);
        let (a, b) = (state.doc.as_ref().unwrap(), fresh.doc.as_ref().expect(&fresh.status));
        assert_eq!(b.skeleton, a.skeleton);
        assert_eq!(b.pose, a.pose);
        assert_eq!(b.image_bytes, a.image_bytes, "image stored byte for byte");
        assert_eq!(fresh.params.gap_close, 5);
        assert_eq!(fresh.overlap, 0.6);
        assert_eq!(b.skel_masks.len(), 10, "masks recomputed on open");
        assert!(!b.dirty && b.project_path.as_deref() == Some(path.as_path()));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn posing_auto_keys_and_plays_back() {
        let mut state = open_sample();
        state.set_step(Step::Animate);
        state.auto_key = true;
        let forearm = {
            let s = &state.doc.as_ref().unwrap().skeleton;
            s.parts.iter().position(|p| p.name == "left forearm").unwrap()
        };

        // A pose drag at 1.0 s that moves only the forearm (what canvas does).
        state.playhead = 1.0;
        let before = state.snapshot();
        state.drag = Drag::PoseRotate { part: forearm };
        state.doc.as_mut().unwrap().pose.set_angle(forearm, 0.9);
        crate::canvas::key_after_drag(&mut state, before.as_ref());
        state.drag = Drag::None;
        state.commit(before);
        let m = &state.doc.as_ref().unwrap().motion;
        assert_eq!(m.tracks.keys().collect::<Vec<_>>(), vec!["left forearm"], "only the moved part is keyed");
        assert_eq!(m.key_times(), vec![1.0]);

        // Playback shows the keyed pose; undo removes the key.
        let ctx = egui::Context::default();
        state.playhead = 0.0;
        state.tick(&ctx);
        assert_eq!(state.doc.as_ref().unwrap().pose.angle(forearm), 0.9, "held before the only key");
        state.undo();
        assert!(state.doc.as_ref().unwrap().motion.is_empty());

        // Looping wraps the playhead.
        state.redo();
        state.playhead = state.doc.as_ref().unwrap().motion.duration - 1e-4;
        state.playing = true;
        state.tick(&ctx);
        assert!(state.playhead < 1.0, "wrapped to the start, at {}", state.playhead);

        // Saved with the project.
        let path = std::env::temp_dir().join(format!("pm2d_motion_test_{}.pm2d", std::process::id()));
        state.save_to(&path);
        let mut fresh = AppState::default();
        fresh.open(&ctx, &path);
        assert_eq!(fresh.doc.as_ref().unwrap().motion, state.doc.as_ref().unwrap().motion);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn auto_key_off_previews_until_keyed() {
        let mut state = open_sample();
        let ctx = egui::Context::default();
        state.set_step(Step::Animate);
        assert!(!state.auto_key, "off by default");
        let head = 0;

        // Some motion exists, so the shown pose normally comes from it.
        state.playhead = 0.0;
        state.key_all();
        state.playhead = 1.0;
        state.tick(&ctx);

        // A pose drag with auto-key off: nothing recorded, pose stays on screen.
        let before = state.snapshot();
        state.drag = Drag::PoseRotate { part: head };
        state.doc.as_mut().unwrap().pose.set_angle(head, 0.5);
        crate::canvas::key_after_drag(&mut state, before.as_ref());
        state.drag = Drag::None;
        state.commit(before);
        assert!(state.unkeyed_pose);
        assert_eq!(state.doc.as_ref().unwrap().motion.key_times(), vec![0.0]);
        state.tick(&ctx);
        assert_eq!(state.doc.as_ref().unwrap().pose.angle(head), 0.5, "preview survives a frame");

        // K keys it: only the changed part, at the playhead.
        state.key_pose();
        let m = &state.doc.as_ref().unwrap().motion;
        assert_eq!(m.key_times(), vec![0.0, 1.0]);
        assert_eq!(m.keys_at(1.0).len(), 1);
        assert!(!state.unkeyed_pose);

        // An unkeyed change is dropped when the playhead moves.
        state.doc.as_mut().unwrap().pose.set_angle(head, -0.3);
        state.unkeyed_pose = true;
        state.playhead = 0.0;
        state.tick(&ctx);
        assert!(!state.unkeyed_pose);
        assert_eq!(state.doc.as_ref().unwrap().pose.angle(head), 0.0, "back to the motion");
    }

    #[test]
    fn frames_step_snap_and_key_on_the_grid() {
        let mut state = open_sample();
        let ctx = egui::Context::default();
        state.set_step(Step::Animate);
        state.doc.as_mut().unwrap().motion.fps = 10;

        // Stepping moves one frame and stops at both ends.
        state.step_frame(1);
        state.step_frame(1);
        assert!((state.playhead - 0.2).abs() < 1e-6);
        state.step_frame(-1);
        state.step_frame(-1);
        state.step_frame(-1);
        assert_eq!(state.playhead, 0.0);

        // Pausing mid-frame lands on the nearest frame; keys go there too.
        state.playing = true;
        state.playhead = 0.73;
        state.toggle_play();
        assert!(!state.playing && (state.playhead - 0.7).abs() < 1e-6);
        state.playhead = 1.26;
        state.key_all();
        assert_eq!(state.doc.as_ref().unwrap().motion.key_times(), vec![1.3]);

        // Playback holds each frame: 1.29 s still shows frame 12 (1.2 s).
        state.key_sel.clear();
        state.playhead = 0.0;
        state.key_all();
        state.playing = true;
        state.playhead = 1.29 - ctx.input(|i| i.stable_dt).min(0.1);
        state.tick(&ctx);
        let m = &state.doc.as_ref().unwrap().motion;
        let expected = m.sample(1.2, &state.doc.as_ref().unwrap().skeleton);
        assert_eq!(state.doc.as_ref().unwrap().pose, expected);
    }

    #[test]
    fn dragging_keys_retimes_them_and_ease_is_set_per_key() {
        let mut state = open_sample();
        state.set_step(Step::Animate);
        state.playhead = 0.0;
        state.key_all();
        state.playhead = 1.0;
        state.key_all();
        state.playhead = 2.0;
        state.key_all();

        // Drag the keys at 1.0 s: passing over the 2.0 s keys doesn't eat
        // them, and they land on a frame (24 fps).
        state.key_sel = state.doc.as_ref().unwrap().motion.keys_at(1.0);
        state.begin_key_drag(1.0);
        state.update_key_drag(2.3);
        state.update_key_drag(1.51);
        state.end_key_drag();
        assert_eq!(state.doc.as_ref().unwrap().motion.key_times(), vec![0.0, 1.5, 2.0]);
        assert_eq!(state.playhead, 1.5, "playhead follows the grabbed key");
        assert!(state.key_sel.iter().all(|r| r.t == 1.5), "selection follows the keys");

        // One undo step puts them back.
        state.undo();
        assert_eq!(state.doc.as_ref().unwrap().motion.key_times(), vec![0.0, 1.0, 2.0]);

        // Ease: set on the selection, shared value read back, undoable.
        state.key_sel = state.doc.as_ref().unwrap().motion.keys_at(0.0);
        assert_eq!(state.selected_ease(), Some(Ease::Smooth));
        state.set_selected_ease(Ease::Hold);
        assert_eq!(state.selected_ease(), Some(Ease::Hold));
        state.key_sel.extend(state.doc.as_ref().unwrap().motion.keys_at(1.0));
        assert_eq!(state.selected_ease(), None, "mixed");
        state.undo();
        state.key_sel = state.doc.as_ref().unwrap().motion.keys_at(0.0);
        assert_eq!(state.selected_ease(), Some(Ease::Smooth));
    }

    #[test]
    fn copy_paste_delete_selected_keys() {
        let mut state = open_sample();
        let ctx = egui::Context::default();
        state.playhead = 0.5;
        state.key_all();
        let n = state.doc.as_ref().unwrap().motion.keys_at(0.5).len();

        // Select everything at 0.5, copy, paste at 2.0.
        state.key_sel = state.doc.as_ref().unwrap().motion.keys_at(0.5);
        state.copy_keys(&ctx);
        let text = state.clip.as_ref().unwrap().to_clipboard_text();
        state.playhead = 2.0;
        state.paste_keys(Some(&text));
        let m = &state.doc.as_ref().unwrap().motion;
        assert_eq!(m.key_times(), vec![0.5, 2.0]);
        assert_eq!(state.key_sel.len(), n, "pasted keys become the selection");

        // Plain text on the clipboard falls back to the app's own copy.
        state.playhead = 2.5;
        state.paste_keys(Some("not keys"));
        assert_eq!(state.doc.as_ref().unwrap().motion.key_times(), vec![0.5, 2.0, 2.5]);

        // Cut = copy + delete in one undo step; paste brings them back.
        state.key_sel = state.doc.as_ref().unwrap().motion.keys_at(2.0);
        state.cut_keys(&ctx);
        assert_eq!(state.doc.as_ref().unwrap().motion.key_times(), vec![0.5, 2.5]);
        state.playhead = 2.0;
        state.paste_keys(None);
        assert_eq!(state.doc.as_ref().unwrap().motion.key_times(), vec![0.5, 2.0, 2.5]);
        state.undo();
        state.undo();
        assert_eq!(state.doc.as_ref().unwrap().motion.key_times(), vec![0.5, 2.0, 2.5], "undoing the cut restores the keys");
        state.redo();
        state.redo();

        // Ctrl+A selects every key across all three times.
        state.select_all_keys();
        assert_eq!(state.key_sel.len(), 3 * n);
        state.key_sel = state.doc.as_ref().unwrap().motion.keys_at(2.5);

        // Delete the selection (the 2.5 keys), then undo it.
        state.delete_selected_keys();
        assert_eq!(state.doc.as_ref().unwrap().motion.key_times(), vec![0.5, 2.0]);
        assert!(state.key_sel.is_empty());
        state.undo();
        assert_eq!(state.doc.as_ref().unwrap().motion.key_times(), vec![0.5, 2.0, 2.5]);
    }

    #[test]
    fn finished_rename_moves_the_track() {
        let mut state = open_sample();
        let before = state.snapshot();
        let doc = state.doc.as_mut().unwrap();
        let mut pose = doc.pose.clone();
        pose.set_angle(0, 0.3);
        doc.motion.set_keys(0.5, &doc.skeleton, &pose, None);
        doc.skeleton.parts[0].name = "big head".into();
        state.commit(before);
        let m = &state.doc.as_ref().unwrap().motion;
        assert!(m.tracks.contains_key("big head") && !m.tracks.contains_key("head"));
    }

    #[test]
    fn steps_follow_the_workflow() {
        let mut state = open_sample();
        assert_eq!(state.step, Step::Review, "a new drawing starts at Review");

        // Leaving the posed steps stops playback; Export starts at the timeline's fps.
        state.set_step(Step::Animate);
        state.doc.as_mut().unwrap().motion.fps = 12;
        state.playing = true;
        state.set_step(Step::Export);
        assert!(state.playing, "previewing carries on into Export");
        assert_eq!(state.export.settings.fps, 12);
        state.set_step(Step::Rig);
        assert!(!state.playing);

        // A saved project reopens where work left off: Animate once it has keys.
        state.key_all();
        let path = std::env::temp_dir().join(format!("pm2d_steps_test_{}.pm2d", std::process::id()));
        state.save_to(&path);
        let mut fresh = AppState::default();
        fresh.open(&egui::Context::default(), &path);
        assert_eq!(fresh.step, Step::Animate);
        std::fs::remove_file(&path).unwrap();
    }
}
