use crate::canvas::{Drag, View};
use crate::export::ExportUi;
use crate::panel::{card, collapsing, fold, intro, shortcuts};
use crate::unsaved::{Pending, Unsaved};
use anim_core::{
    Clip, Cycle, Ease, History, KeyRef, PartMesh, Template, Toward, WeightPaint, apply_paint, build_meshes, cell_size, smooth_weights, Mask, Motion, Params, Pose, ProjectData, RectI, Skeleton, Sprite, build_sprites,
    NO_PART, distance_transform, foreground_mask, project, project_from_bytes, save_project, masks_from_labels, part_labels,
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

/// How the drawing, its parts and the tint overlay are filtered: smooth when
/// zoomed in, and with mipmaps when zoomed out, so a big drawing shrunk to fit
/// the window stays smooth instead of jagged.
const SMOOTH: TextureOptions = TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear));

/// Joint softness for new drawings: elbows and knees bend smoothly.
const DEFAULT_SOFTNESS: f32 = 0.8;

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

/// What the weight brush does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Brush {
    Add,
    Remove,
    Smooth,
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
    pub paint: WeightPaint,
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
    /// The timeline's length when the drag began. The rows keep that time
    /// scale during the drag, so keys dragged past the end make the motion
    /// longer without the scale shifting under the pointer.
    pub axis: f32,
}

/// A loaded drawing and everything derived from it.
pub struct Doc {
    pub image: RgbaImage,
    pub texture: TextureHandle,
    pub fg: Mask,
    /// `distance_transform(fg)`, kept because it only changes with `fg` and
    /// every joint move needs it. Change `fg` through `set_fg`.
    pub dt: Vec<f32>,
    /// The cutout settings `fg` was found with.
    pub fg_params: Params,
    /// `fg.bbox()`, kept for the same reason as `dt`.
    pub fg_box: Option<RectI>,
    pub skeleton: Skeleton,
    /// Per-part masks, built from `skel_labels` when the sprites are rebuilt.
    pub skel_masks: Vec<Mask>,
    /// The same split as one part-number-per-pixel map (`NO_PART` = background),
    /// which the Rig overlay is painted from in one pass.
    pub skel_labels: Vec<u16>,
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
    /// Each part's shape as a white texture (for tinting, e.g. the weight heat map).
    pub silhouette_tex: Vec<Option<TextureHandle>>,
    /// One triangle mesh per part over its sprite, rebuilt with the sprites.
    /// The canvas draws the character as these, bent by the bones.
    pub meshes: Vec<PartMesh>,
    /// Bone weights painted with the weight brush, on top of the automatic ones.
    pub paint: WeightPaint,
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
            paint: self.paint.clone(),
        }
    }

    /// Put back a snapshot's skeleton, pose, motion and painted weights (masks
    /// are recomputed by the caller).
    fn restore(&mut self, s: Snapshot) {
        self.skeleton = s.skeleton;
        self.pose = s.pose;
        self.motion = s.motion;
        self.paint = s.paint;
    }

    /// Work out every mesh point's bone weights: the automatic blend at the
    /// joints (`softness`), then the painted changes. Quick, so a brush
    /// stroke can redo it every frame.
    pub fn update_weights(&mut self, softness: f32) {
        smooth_weights(&mut self.meshes, &self.skeleton, &self.dt, self.fg.w as usize, softness);
        apply_paint(&mut self.meshes, &self.skeleton, &self.paint);
    }

    /// Back to the pose as drawn. Needed whenever bones are added or removed,
    /// since pose angles are stored by part number.
    pub fn reset_pose(&mut self) {
        self.pose = Pose::rest(self.skeleton.parts.len());
    }

    pub fn size(&self) -> egui::Vec2 {
        egui::vec2(self.image.width() as f32, self.image.height() as f32)
    }

    /// Re-split the character into parts after the joints or the cutout
    /// changed. Runs every frame of a joint drag, so it only updates the
    /// part map; the per-part masks are built when the sprites need them.
    pub fn recompute_skeleton(&mut self) {
        part_labels(&self.fg, &self.dt, self.fg_box, &self.skeleton, &mut self.skel_labels);
        self.overlay_dirty = true;
        self.sprites_dirty = true;
    }

    /// Find the character again with `params` (after the cutout settings change).
    pub fn set_fg(&mut self, params: &Params) {
        self.fg = foreground_mask(&self.image, params);
        self.dt = distance_transform(&self.fg);
        self.fg_box = self.fg.bbox();
        self.fg_params = *params;
        self.recompute_skeleton();
    }

    /// The character's bounding box (the whole image if nothing was found).
    pub fn figure_box(&self) -> RectI {
        let (w, h) = self.image.dimensions();
        self.fg.bbox().unwrap_or(RectI::new(0, 0, w as i32, h as i32))
    }

    /// Replace the skeleton with `template`, fitted over the character.
    pub fn fit_skeleton(&mut self, template: &Template) {
        self.skeleton = template.fit(self.figure_box());
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
    /// Shift+drag on a timeline row: (row, time the drag started), selecting
    /// the keys in between when it ends.
    pub key_range: Option<(usize, f32)>,
    /// How far the timeline is zoomed in (1 = the whole motion fits).
    pub timeline_zoom: f32,
    /// Sideways scroll to apply to the timeline rows next frame (after a zoom).
    pub timeline_scroll_to: Option<f32>,
    /// Last copied keys, used if the system clipboard has something else on it.
    pub clip: Option<Clip>,
    /// Selected joint or bone (Rig step).
    pub skel_sel: SkelSel,
    /// Rig step: tint each part's pixels in its colour.
    pub show_parts: bool,
    /// Joint disc radius as a multiple of the child part's thickness (0 = no discs).
    pub overlap: f32,
    /// How smoothly the drawing bends at the joints: the blend band's half-width
    /// as a multiple of the drawing's thickness there (0 = rigid pieces). Saved.
    pub softness: f32,
    /// Guess what's hidden behind parts (shown when they move away). Saved.
    pub fill_behind: bool,
    /// Animate step: show the original drawing faintly behind the posed parts.
    pub show_ghost: bool,
    /// Animate step: draw bones and joint dots over the parts (B toggles).
    pub show_bones: bool,
    /// Rig and Animate: draw the triangle mesh over the character.
    pub show_mesh: bool,
    /// Rig: dragging on the drawing paints the selected bone's weights.
    pub paint_mode: bool,
    /// What the weight brush does: add, remove or smooth the selected bone's pull.
    pub brush: Brush,
    /// Brush radius, in screen points (so it feels the same at any zoom).
    pub brush_size: f32,
    /// How much one dab changes a weight (0..1).
    pub brush_strength: f32,
    /// Part whose row the pointer is on in the Layers list; the canvas lights it up.
    pub highlight_part: Option<usize>,
    /// Skeleton templates: built-in ones, then the user's own.
    pub templates: Vec<Template>,
    /// The template picked in Rig; new drawings start with it.
    pub template_choice: usize,
    /// Name typed under "Save as template".
    pub template_name: String,
    /// Ready-made motions offered in Animate (those that fit the skeleton).
    pub cycles: Vec<Cycle>,
    /// Side-view templates are placed facing left (mirrored) instead of right.
    pub facing_left: bool,
    /// Screen x of the canvas's left edge last frame (the side panel's right
    /// edge), so the step buttons can start there.
    pub canvas_left: f32,
    pub status: String,
    /// Set when a new image is loaded so the canvas fits it on the next frame.
    pub fit_pending: bool,
    /// Export settings and any export in progress.
    pub export: ExportUi,
    /// The app icon, shown next to the name in the browser's top bar (loaded when first needed).
    pub app_icon: Option<TextureHandle>,
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
            key_range: None,
            timeline_zoom: 1.0,
            timeline_scroll_to: None,
            clip: None,
            skel_sel: SkelSel::None,
            show_parts: true,
            overlap: 1.0,
            softness: DEFAULT_SOFTNESS,
            // Off by default until its guesses are more reliable.
            fill_behind: false,
            show_ghost: false,
            show_bones: true,
            show_mesh: false,
            paint_mode: false,
            brush: Brush::Add,
            brush_size: 30.0,
            brush_strength: 0.1,
            highlight_part: None,
            templates: crate::templates::all(),
            template_choice: 0,
            template_name: String::new(),
            cycles: anim_core::builtin_cycles(),
            facing_left: false,
            canvas_left: 0.0,
            status: "Open or drop a drawing to start.".into(),
            fit_pending: false,
            export: ExportUi::default(),
            app_icon: None,
            unsaved: Unsaved::new(crate::unsaved::recovery_dir()),
        }
    }
}

impl AppState {
    /// Open an image or a `.pm2d` project file from disk.
    pub fn open(&mut self, ctx: &egui::Context, path: &Path) {
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        match std::fs::read(path) {
            Ok(bytes) => self.open_bytes(ctx, &name, bytes, Some(path)),
            Err(e) => self.status = format!("Could not open {}: {e}", path.display()),
        }
    }

    /// Open an image or a `.pm2d` project from its contents, chosen by the
    /// file name's extension. `path` is where it came from on disk, if
    /// anywhere (a project then saves back there; in the browser there's none).
    pub fn open_bytes(&mut self, ctx: &egui::Context, name: &str, bytes: Vec<u8>, path: Option<&Path>) {
        let ext = Path::new(name).extension().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
        if ext == project::EXTENSION {
            self.open_project(ctx, name, &bytes, path);
        } else {
            self.open_image(ctx, name, bytes, ext);
        }
    }

    fn open_image(&mut self, ctx: &egui::Context, name: &str, bytes: Vec<u8>, ext: String) {
        let Some(mut doc) = self.load_doc(ctx, name, bytes, ext) else { return };
        // A new drawing bends smoothly by default (an opened project keeps its own setting).
        self.softness = DEFAULT_SOFTNESS;
        // Start with the template last used (Person (front) at first), facing the same way.
        if let Some(t) = self.templates.get(self.template_choice).or(self.templates.first()) {
            doc.fit_skeleton(&self.facing(t));
        }
        self.set_doc(doc);
        // A new drawing starts at Review, so the cutout can be checked first.
        self.set_step(Step::Review);
    }

    fn open_project(&mut self, ctx: &egui::Context, name: &str, bytes: &[u8], path: Option<&Path>) {
        let loaded = match project_from_bytes(bytes) {
            Ok(l) => l,
            Err(e) => {
                self.status = format!("Could not open {name}: {e}");
                return;
            }
        };
        let data = loaded.data;
        // Settings first: the figure is found using the project's own cutout settings.
        self.params = data.params;
        self.overlap = data.overlap;
        self.softness = data.softness;
        self.fill_behind = data.fill_behind;
        let Some(mut doc) = self.load_doc(ctx, &data.image_name, loaded.image_bytes, loaded.image_ext) else { return };
        doc.restore(Snapshot { skeleton: data.skeleton, pose: data.pose, motion: data.motion, paint: data.weight_paint });
        doc.recompute_skeleton();
        doc.project_path = path.map(Path::to_path_buf);
        self.status = format!("Opened project {name}");
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
        let texture = ctx.load_texture("drawing", to_color_image(&image), SMOOTH);
        let fg = foreground_mask(&image, &self.params);
        let dt = distance_transform(&fg);
        let fg_box = fg.bbox();
        self.status = format!("Opened {name} ({}×{})", image.width(), image.height());
        let stem = Path::new(name).file_stem().unwrap_or_default().to_string_lossy().into_owned();
        Some(Doc {
            image,
            texture,
            fg,
            fg_box,
            dt,
            fg_params: self.params,
            skeleton: Skeleton::empty(),
            skel_masks: Vec::new(),
            skel_labels: Vec::new(),
            overlay: None,
            overlay_dirty: true,
            history: History::new(UNDO_LIMIT),
            name: stem,
            pose: Pose::default(),
            motion: Motion::default(),
            sprites: Vec::new(),
            sprite_tex: Vec::new(),
            silhouette_tex: Vec::new(),
            meshes: Vec::new(),
            paint: WeightPaint::default(),
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

    /// Replace the motion with a ready-made one fitted to this character, and play it.
    pub fn apply_cycle(&mut self, cycle: &Cycle) {
        let before = self.snapshot();
        let Some(doc) = &mut self.doc else { return };
        let figure_h = doc.figure_box().h as f32;
        let facing_left = anim_core::faces_left(&doc.skeleton);
        doc.motion = cycle.to_motion(&doc.skeleton, figure_h, doc.motion.fps, facing_left);
        doc.pose = doc.motion.sample(0.0, &doc.skeleton);
        self.key_sel.clear();
        self.unkeyed_pose = false;
        self.playhead = 0.0;
        self.sampled_at = 0.0;
        self.playing = true;
        self.status = format!("{}: change any key on the timeline to make it your own", cycle.name);
        self.commit(before);
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
        self.key_drag = Some(KeyDrag {
            grab_t,
            sel: self.key_sel.clone(),
            motion: doc.motion.clone(),
            before: self.snapshot(),
            axis: doc.motion.duration,
        });
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
        self.timeline_zoom = 1.0;
        self.doc = Some(doc);
        self.skel_sel = SkelSel::None;
        self.drag = Drag::None;
        self.drag_before = None;
        self.field_before = None;
        self.fit_pending = true;
    }

    /// Save to the current project file, or ask where if it has none yet.
    /// In the browser, the project is downloaded.
    pub fn save(&mut self) {
        match self.doc.as_ref().and_then(|d| d.project_path.clone()) {
            Some(path) => self.save_to(&path),
            None => self.save_as(),
        }
    }

    /// Browser: there's nowhere to save to, so the project file is downloaded.
    #[cfg(target_arch = "wasm32")]
    pub fn save_as(&mut self) {
        let Some(data) = self.project_data() else { return };
        let Some(doc) = &mut self.doc else { return };
        let file = format!("{}.{}", doc.name, project::EXTENSION);
        self.status = match anim_core::project_to_bytes(&data, &doc.image_bytes, &doc.image_ext) {
            Ok(bytes) => {
                crate::platform::download(&file, &bytes, "application/zip");
                doc.dirty = false;
                format!("Downloaded {file}")
            }
            Err(e) => format!("Save failed: {e}"),
        };
    }

    #[cfg(not(target_arch = "wasm32"))]
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
            softness: self.softness,
            fill_behind: self.fill_behind,
            weight_paint: doc.paint.clone(),
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
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))] // desktop only: browsers have no paths
    pub fn request_open(&mut self, ctx: &egui::Context, path: &Path) {
        if self.doc.as_ref().is_some_and(|d| d.dirty) {
            self.unsaved.prompt = Some(Pending::Open(path.to_path_buf()));
        } else {
            self.open(ctx, path);
        }
    }

    /// Open a file from its contents (a drop or a browser pick), first asking
    /// to save if the current drawing has unsaved changes.
    pub fn request_open_bytes(&mut self, ctx: &egui::Context, name: String, bytes: Vec<u8>) {
        if self.doc.as_ref().is_some_and(|d| d.dirty) {
            self.unsaved.prompt = Some(Pending::OpenBytes(name, bytes));
        } else {
            self.open_bytes(ctx, &name, bytes, None);
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

    /// Browser: the browser's file picker answers later; the file arrives
    /// through `platform::take_picked` (handled in `main.rs`).
    #[cfg(target_arch = "wasm32")]
    pub fn open_dialog(&mut self, ctx: &egui::Context) {
        crate::platform::pick_file(ctx);
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_dialog(&mut self, ctx: &egui::Context) {
        let images = crate::platform::IMAGE_TYPES;
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
                        doc.paint.rename(&o.name, &n.name);
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
        doc.skel_masks = masks_from_labels(&doc.skel_labels, doc.fg.w, doc.fg.h, doc.skeleton.parts.len());
        doc.sprites = build_sprites(&doc.image, &doc.fg, &doc.dt, &doc.skeleton, &doc.skel_masks, self.overlap, self.fill_behind);
        doc.meshes = build_meshes(&doc.sprites, cell_size(doc.figure_box()));
        doc.update_weights(self.softness);
        doc.sprite_tex = doc
            .sprites
            .iter()
            .map(|s| {
                (s.image.width() > 0)
                    .then(|| ctx.load_texture(format!("part {}", s.part), to_color_image(&s.image), SMOOTH))
            })
            .collect();
        // Each part's shape in white, which the weight heat map is tinted
        // through so it follows the drawing's outline instead of mesh cells.
        doc.silhouette_tex = doc
            .sprites
            .iter()
            .map(|s| {
                (s.image.width() > 0).then(|| {
                    let white: Vec<Color32> =
                        s.image.pixels().map(|p| Color32::from_rgba_unmultiplied(255, 255, 255, p.0[3])).collect();
                    let image = ColorImage::new([s.image.width() as usize, s.image.height() as usize], white);
                    ctx.load_texture(format!("part {} shape", s.part), image, SMOOTH)
                })
            })
            .collect();
    }

    /// Move bone `p` (with the bones hanging from it) in the painting order,
    /// as one undo step. The pose is kept.
    pub fn change_layer(&mut self, p: usize, to: Toward) {
        let before = self.snapshot();
        let Some(doc) = &mut self.doc else { return };
        if p >= doc.skeleton.parts.len() || !doc.skeleton.move_layer(p, to) {
            return;
        }
        let name = &doc.skeleton.parts[p].name;
        self.status = match to {
            Toward::Front => format!("Moved {name} forward"),
            Toward::Back => format!("Moved {name} back"),
            Toward::FrontMost => format!("Moved {name} to the front"),
            Toward::BackMost => format!("Moved {name} to the back"),
        };
        self.commit(before);
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
        // Painted straight into egui's pixel format, one pass over the image
        // spread across the CPU cores (a joint drag repaints it every frame).
        let (w, h) = (doc.image.width() as usize, doc.image.height() as usize);
        let mut pixels = vec![Color32::TRANSPARENT; w * h];
        match self.step {
            Step::Review => {
                let dim = Color32::from_rgba_unmultiplied(30, 30, 36, 210);
                let fg = &doc.fg.data;
                anim_core::par::for_rows(&mut pixels, w, 0..h, |y, row| {
                    for (x, px) in row.iter_mut().enumerate() {
                        if fg[y * w + x] == 0 {
                            *px = dim;
                        }
                    }
                });
            }
            Step::Rig if self.show_parts && doc.skel_labels.len() == w * h => {
                let tints: Vec<Color32> =
                    PALETTE.iter().map(|&[r, g, b]| Color32::from_rgba_unmultiplied(r, g, b, 140)).collect();
                let labels = &doc.skel_labels;
                anim_core::par::for_rows(&mut pixels, w, 0..h, |y, row| {
                    for (x, px) in row.iter_mut().enumerate() {
                        let l = labels[y * w + x];
                        if l != NO_PART {
                            *px = tints[l as usize % tints.len()];
                        }
                    }
                });
            }
            _ => {}
        }
        let color = ColorImage::new([w, h], pixels);
        match &mut doc.overlay {
            Some(tex) => tex.set(color, SMOOTH),
            None => doc.overlay = Some(ctx.load_texture("overlay", color, SMOOTH)),
        }
    }

    /// The bar across the top: file buttons, the four steps, undo/redo.
    pub fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let has_doc = self.doc.is_some();
            // The browser has no window title, so the app names itself here.
            if crate::platform::IS_WEB {
                let icon = self.app_icon.get_or_insert_with(|| {
                    let png = image::load_from_memory(include_bytes!("../assets/icon.png")).expect("built-in icon");
                    ui.ctx().load_texture("app icon", to_color_image(&png.to_rgba8()), SMOOTH)
                });
                ui.image((icon.id(), egui::vec2(20.0, 20.0)));
                ui.label(egui::RichText::new("PuppetMotion 2D").size(16.0).strong());
                ui.separator();
            }
            if ui.button("Open…").on_hover_text("A drawing or a project (Ctrl+O)").clicked() {
                self.request_open_dialog(ui.ctx());
            }
            if crate::platform::IS_WEB {
                // The browser can't save to a file, only download one.
                if ui
                    .add_enabled(has_doc, egui::Button::new("Download project"))
                    .on_hover_text("Saves a .pm2d project file you can open again later (Ctrl+S)")
                    .clicked()
                {
                    self.save();
                }
            } else {
                if ui.add_enabled(has_doc, egui::Button::new("Save")).on_hover_text("Ctrl+S").clicked() {
                    self.save();
                }
                if ui.add_enabled(has_doc, egui::Button::new("Save As…")).on_hover_text("Ctrl+Shift+S").clicked() {
                    self.save_as();
                }
            }
            // Start the steps where the canvas starts, past the side panel.
            let gap = self.canvas_left + 8.0 - ui.cursor().left();
            ui.add_space(gap.max(8.0));
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
        // Set again below while the pointer is on a Layers row.
        self.highlight_part = None;
        ui.heading(self.step.label());
        ui.label(egui::RichText::new(self.step.hint()).weak());
        ui.separator();
        // Shortcuts are pinned to the bottom, always open; the rest scrolls above them.
        egui::Panel::bottom("shortcuts").resizable(false).show(ui, |ui| {
            egui::ScrollArea::horizontal().id_salt("shortcuts scroll").show(ui, |ui| {
                shortcuts(ui, step_shortcuts(self.step));
            });
        });
        // Scrolls both ways, so a small window or narrow panel can still reach everything.
        egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| match self.step {
            Step::Review => self.review_section(ui),
            Step::Rig => self.rig_section(ui),
            Step::Animate => self.animate_section(ui),
            Step::Export => crate::export::panel(ui, self),
        });
    }

    fn review_section(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = &self.doc else {
            card(ui, "Open", |ui| {
                ui.label("A drawing of one character on a plain background. A PNG with a transparent background works best.");
                let open = egui::Button::new(egui::RichText::new("Open a drawing…").size(15.0).strong());
                if ui.add(open.min_size(egui::vec2(ui.available_width(), 30.0))).clicked() {
                    self.request_open_dialog(ui.ctx());
                }
                ui.label(egui::RichText::new("Or drop a file on the window. Saved projects (.pm2d) open the same way.").weak());
            });
            return;
        };
        let (name, w, h) = (doc.name.clone(), doc.image.width(), doc.image.height());
        card(ui, "Drawing", |ui| {
            ui.label(egui::RichText::new(&name).strong());
            ui.label(egui::RichText::new(format!("{w} × {h} px")).weak());
            ui.label("Your character stays bright and the background is dimmed.");
            ui.label(
                egui::RichText::new("If some background is included, or part of the character is dimmed, fine-tune the cutout.")
                    .weak(),
            );
        });
        // Finding the character is slow on big drawings, so a slider applies
        // when it's let go (or typed into), not on every frame of the drag.
        let mut apply = false;
        fold(ui, "Fine-tune cutout", |ui| {
            let mut slider = |ui: &mut egui::Ui, s: egui::Slider, hint: &str| {
                let r = ui.add(s).on_hover_text(hint);
                apply |= r.drag_stopped() || (r.changed() && !r.dragged());
            };
            slider(
                ui,
                egui::Slider::new(&mut self.params.threshold, 5.0..=150.0).text("background tolerance"),
                "How different from the background a colour must be to count as the character",
            );
            slider(
                ui,
                egui::Slider::new(&mut self.params.gap_close, 0..=20).text("close outline gaps (px)"),
                "Seals small breaks in a sketchy outline so the background can't leak inside",
            );
            slider(
                ui,
                egui::Slider::new(&mut self.params.min_area, 0..=5000).logarithmic(true).text("ignore specks under (px)"),
                "Drops tiny dots of dirt or noise",
            );
            slider(
                ui,
                egui::Slider::new(&mut self.params.corner_patch, 2..=64).text("corner sample (px)"),
                "The background colour is read from squares this size in each corner",
            );
            if ui.button("Reset").clicked() {
                self.params = Params::default();
                apply = true;
            }
        });
        if apply
            && let Some(doc) = &mut self.doc
            && doc.fg_params != self.params
        {
            doc.set_fg(&self.params);
            doc.dirty = true;
        }
    }

    fn animate_section(&mut self, ui: &mut egui::Ui) {
        if self.doc.is_none() {
            return;
        }
        intro(ui, "Pick a ready-made motion, or drag the white dots to pose. Press K to keep the pose at this frame, then move along the timeline and pose again.");
        self.quick_motions(ui);
        card(ui, "Pose", |ui| {
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
        });
        card(ui, "View", |ui| {
            ui.checkbox(&mut self.show_bones, "Show bones").on_hover_text("B: hide them to watch just the animation");
            ui.checkbox(&mut self.show_ghost, "Show original drawing behind");
            ui.checkbox(&mut self.show_mesh, "Show mesh").on_hover_text("The triangles the drawing bends with");
        });
        card(ui, "Joints", |ui| {
            let before = self.softness;
            ui.add(egui::Slider::new(&mut self.softness, 0.0..=2.0).text("softness"))
                .on_hover_text("How smoothly elbows, knees and other joints bend.\n0 = stiff cut-out pieces, higher = a wider, softer bend.");
            if before != self.softness
                && let Some(doc) = &mut self.doc
            {
                // Only the weights change, which is quick enough to redo while dragging.
                doc.update_weights(self.softness);
                doc.dirty = true;
            }
            let fill = self.fill_behind;
            ui.checkbox(&mut self.fill_behind, "Fill behind parts").on_hover_text(
                "Guess what's hidden behind an arm or the head, so moving them shows more body instead of a hole",
            );
            if fill != self.fill_behind
                && let Some(doc) = &mut self.doc
            {
                doc.sprites_dirty = true;
                doc.dirty = true;
            }
        });
        self.layers_list(ui);
        fold(ui, "Fine-tune joints", |ui| {
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
    }

    /// "Quick motions": one button per ready-made motion that fits the skeleton.
    fn quick_motions(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = &self.doc else { return };
        let fitting: Vec<Cycle> = self.cycles.iter().filter(|c| c.matches(&doc.skeleton)).cloned().collect();
        let has_keys = !doc.motion.is_empty();
        let mut chosen = None;
        card(ui, "Quick motions", |ui| {
            if fitting.is_empty() {
                ui.label(
                    egui::RichText::new("None fit this skeleton. Ready-made motions need a rig from the Person templates.")
                        .small()
                        .weak(),
                );
                return;
            }
            ui.horizontal_wrapped(|ui| {
                for c in &fitting {
                    let hint = if has_keys { "Replaces the current animation (Ctrl+Z undoes)" } else { "Fills the timeline with this motion" };
                    if ui.button(&c.name).on_hover_text(hint).clicked() {
                        chosen = Some(c.clone());
                    }
                }
            });
        });
        if let Some(c) = chosen {
            self.apply_cycle(&c);
        }
    }

    /// "Layers": every part from front (top) to back, with buttons to move it
    /// one row or to either end. Parts attached to it move with it.
    fn layers_list(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = &self.doc else { return };
        let skel = &doc.skeleton;
        let front_to_back: Vec<usize> = skel.paint_order().into_iter().rev().collect();
        let mut change = None;
        let mut hovered = None;
        fold(ui, "Layers (what's in front)", |ui| {
            ui.label(
                egui::RichText::new("The top of the list is in front. Parts attached to a part move with it.").weak(),
            );
            for (row, &p) in front_to_back.iter().enumerate() {
                let (top, bottom) = (row == 0, row + 1 == front_to_back.len());
                let line = ui.horizontal(|ui| {
                    let buttons = [
                        ("⏫", "To the front", Toward::FrontMost, !top),
                        ("▲", "Up one", Toward::Front, !top),
                        ("▼", "Down one", Toward::Back, !bottom),
                        ("⏬", "To the back", Toward::BackMost, !bottom),
                    ];
                    for (icon, hint, to, enabled) in buttons {
                        if ui.add_enabled(enabled, egui::Button::new(icon).small()).on_hover_text(hint).clicked() {
                            change = Some((p, to));
                        }
                    }
                    ui.label(egui::RichText::new(&skel.parts[p].name).color(part_color(p)));
                });
                // Pointing at a row lights up that part's bone on the canvas.
                if line.response.contains_pointer() {
                    hovered = Some(p);
                }
            }
        });
        self.highlight_part = hovered;
        if let Some((p, to)) = change {
            self.change_layer(p, to);
        }
    }

    /// Template picker (Use replaces the bones), Clear, and "Save as template".
    fn template_controls(&mut self, ui: &mut egui::Ui) {
        self.template_choice = self.template_choice.min(self.templates.len().saturating_sub(1));
        ui.horizontal(|ui| {
            let current = self.templates.get(self.template_choice).map_or("", |t| t.name.as_str()).to_string();
            egui::ComboBox::from_id_salt("template").selected_text(current).show_ui(ui, |ui| {
                for (i, t) in self.templates.iter().enumerate() {
                    ui.selectable_value(&mut self.template_choice, i, &t.name);
                }
            });
            if ui.button("Use").on_hover_text("Replace the bones with this template, fitted to the character").clicked() {
                self.apply_template(self.template_choice);
            }
            if ui.button("Clear").on_hover_text("Remove all bones and start from scratch").clicked() {
                self.edit_skeleton(|s| *s = Skeleton::empty());
            }
        });
        // Side views: which way the character faces, and which limbs are in front.
        let side_template = self.templates.get(self.template_choice).is_some_and(Template::is_side_view);
        let has_near_far = self.doc.as_ref().is_some_and(|d| d.skeleton.joints.iter().any(|j| j.name.starts_with("near ")));
        if side_template {
            ui.horizontal(|ui| {
                ui.label("Facing");
                let before = self.facing_left;
                ui.selectable_value(&mut self.facing_left, false, "Right ▶");
                ui.selectable_value(&mut self.facing_left, true, "◀ Left");
                if before != self.facing_left {
                    // Turning the template around re-places it.
                    self.apply_template(self.template_choice);
                }
            });
        }
        if has_near_far
            && ui
                .button("Swap near/far")
                .on_hover_text("Use if the bones called \"near\" landed on the limbs behind the body")
                .clicked()
        {
            self.edit_skeleton(|s| {
                s.swap_near_far();
            });
            self.status = "Swapped near and far limbs".into();
        }
        collapsing(ui, "Save as template", |ui| {
            ui.label(egui::RichText::new("Keep these bones to use on other drawings, e.g. for a monster or an animal.").weak());
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.template_name).hint_text("Name").desired_width(140.0));
                let name = self.template_name.trim().to_string();
                let ok = !name.is_empty() && !crate::templates::is_builtin(&name);
                if ui.add_enabled(ok, egui::Button::new("Save")).clicked() {
                    self.save_template(&name);
                }
            });
            if crate::templates::is_builtin(&self.template_name) {
                ui.label(egui::RichText::new("That name belongs to a built-in template; pick another.").weak());
            }
        });
    }

    /// Replace the skeleton with template `i`, as one undo step. It's also the
    /// template new drawings start with from now on.
    pub fn apply_template(&mut self, i: usize) {
        let Some(t) = self.templates.get(i).map(|t| self.facing(t)) else { return };
        let before = self.snapshot();
        let Some(doc) = &mut self.doc else { return };
        doc.fit_skeleton(&t);
        self.template_choice = i;
        self.skel_sel = SkelSel::None;
        self.status = format!("Using the {} template", t.name);
        self.commit(before);
    }

    /// `t` turned to face the chosen way (side views only; it's drawn facing right).
    fn facing(&self, t: &Template) -> Template {
        if self.facing_left && t.is_side_view() { t.mirrored() } else { t.clone() }
    }

    /// Save the current bones as a template named `name` and select it.
    pub fn save_template(&mut self, name: &str) {
        let Some(doc) = &self.doc else { return };
        let t = Template::from_skeleton(name, &doc.skeleton, doc.figure_box());
        self.status = match crate::templates::save(&t) {
            Ok(_) => {
                self.templates = crate::templates::all();
                self.template_choice = self.templates.iter().position(|o| o.name == t.name).unwrap_or(0);
                self.template_name.clear();
                format!("Saved template {}", t.name)
            }
            Err(e) => format!("Could not save the template: {e}"),
        };
    }

    fn rig_section(&mut self, ui: &mut egui::Ui) {
        if self.doc.is_none() {
            return;
        }
        intro(ui, "Drag the dots onto the character's joints. Each coloured area moves with its bone.");
        card(ui, "Template", |ui| self.template_controls(ui));
        card(ui, "Bones", |ui| self.bone_tree(ui));
        let sel = self.skel_sel;
        if sel != SkelSel::None {
            let doc = self.doc.as_ref().unwrap();
            let title = match sel {
                SkelSel::Joint(j) => format!("Selected joint: {}", doc.skeleton.joints[j].name),
                SkelSel::Part(p) => format!("Selected bone: {}", doc.skeleton.parts[p].name),
                SkelSel::None => unreachable!(),
            };
            card(ui, &title, |ui| self.selection_editor(ui, sel));
        }
        match self.skel_sel {
            SkelSel::Part(p) => card(ui, "Weight brush", |ui| self.weight_brush(ui, p)),
            // The brush paints one bone, so it needs one selected.
            _ => self.paint_mode = false,
        }
    }

    /// The weight brush for bone `p`: paint where it pulls the drawing.
    fn weight_brush(&mut self, ui: &mut egui::Ui, p: usize) {
        let name = self.doc.as_ref().map_or(String::new(), |d| d.skeleton.parts[p].name.clone());
        ui.toggle_value(&mut self.paint_mode, "Paint weights")
            .on_hover_text("Drag on the drawing to change where this bone pulls it");
        if !self.paint_mode {
            ui.label(egui::RichText::new("Fix spots that follow the wrong bone, like hair that should move with the head.").weak());
            return;
        }
        ui.label(egui::RichText::new(format!("Red follows {name} fully, blue not at all.")).weak());
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.brush, Brush::Add, "Add").on_hover_text("Make it follow this bone more");
            ui.selectable_value(&mut self.brush, Brush::Remove, "Remove").on_hover_text("Make it follow this bone less");
            ui.selectable_value(&mut self.brush, Brush::Smooth, "Smooth")
                .on_hover_text("Soften hard edges in what you painted, for a smoother bend");
        });
        ui.add(egui::Slider::new(&mut self.brush_size, 5.0..=150.0).text("size"));
        ui.add(egui::Slider::new(&mut self.brush_strength, 0.02..=0.5).text("strength"));
        let painted = self.doc.as_ref().is_some_and(|d| d.paint.bones.get(&name).is_some_and(|v| !v.is_empty()));
        if ui.add_enabled(painted, egui::Button::new(format!("Clear painting for {name}"))).clicked() {
            let before = self.snapshot();
            if let Some(doc) = &mut self.doc {
                doc.paint.clear(&name);
                doc.update_weights(self.softness);
            }
            self.commit(before);
        }
    }

    /// One dab of the weight brush at `at` (drawing px) for the selected bone.
    pub fn paint_dab(&mut self, at: [f32; 2]) {
        let SkelSel::Part(p) = self.skel_sel else { return };
        let (zoom, brush, size, strength, softness) =
            (self.view.zoom, self.brush, self.brush_size, self.brush_strength, self.softness);
        let Some(doc) = &mut self.doc else { return };
        let Some(name) = doc.skeleton.parts.get(p).map(|b| b.name.clone()) else { return };
        let (cell, radius) = (cell_size(doc.figure_box()), size / zoom.max(1e-3));
        match brush {
            Brush::Add => doc.paint.dab(&name, at, radius, strength, cell),
            Brush::Remove => doc.paint.dab(&name, at, radius, -strength, cell),
            Brush::Smooth => doc.paint.smooth(&name, at, radius, (strength * 4.0).min(1.0)),
        }
        doc.update_weights(softness);
    }

    /// The bones as a tree, children under their parent. Click one to select it.
    fn bone_tree(&mut self, ui: &mut egui::Ui) {
        let show_parts = self.show_parts;
        ui.checkbox(&mut self.show_parts, "Show part colours");
        if show_parts != self.show_parts
            && let Some(doc) = &mut self.doc
        {
            doc.overlay_dirty = true;
        }
        ui.checkbox(&mut self.show_mesh, "Show mesh")
            .on_hover_text("The triangles each part bends with (updated when you let go of a joint)");
        let Some(doc) = &self.doc else { return };
        if doc.skeleton.parts.is_empty() {
            ui.label(egui::RichText::new("No bones yet. Shift+drag on the drawing to add one.").weak());
        }
        let mut clicked = None;
        ui.spacing_mut().item_spacing.y = 2.0;
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
    }

    /// The selected joint or bone: name, and for a bone its parent, limit and
    /// layer, laid out as a label/control grid; then Delete.
    fn selection_editor(&mut self, ui: &mut egui::Ui, sel: SkelSel) {
        egui::Grid::new("selection").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
            ui.label("Name");
            let before = self.snapshot();
            let doc = self.doc.as_mut().unwrap();
            let name = match sel {
                SkelSel::Joint(j) => &mut doc.skeleton.joints[j].name,
                SkelSel::Part(p) => &mut doc.skeleton.parts[p].name,
                SkelSel::None => unreachable!(),
            };
            let resp = ui.text_edit_singleline(name);
            // One undo step per rename: from gaining focus to losing it.
            if resp.gained_focus() {
                self.field_before = before;
            }
            if resp.lost_focus() {
                let before = self.field_before.take();
                self.commit(before);
            }
            ui.end_row();
            if let SkelSel::Part(p) = sel {
                ui.label("Attached to");
                self.parent_picker(ui, p);
                ui.end_row();
                ui.label("Limit").on_hover_text("How far this bone can turn from the way it's drawn");
                self.limit_editor(ui, p);
                ui.end_row();
                ui.label("Layer").on_hover_text("What's drawn in front. Bones attached to this one move with it.");
                ui.horizontal_wrapped(|ui| {
                    let buttons = [
                        ("To back", "Behind everything", Toward::BackMost),
                        ("Back", "One place back  [", Toward::Back),
                        ("Forward", "One place forward  ]", Toward::Front),
                        ("To front", "In front of everything", Toward::FrontMost),
                    ];
                    for (label, hint, to) in buttons {
                        if ui.small_button(label).on_hover_text(hint).clicked() {
                            self.change_layer(p, to);
                        }
                    }
                });
                ui.end_row();
            }
        });
        let delete = if matches!(sel, SkelSel::Joint(_)) { "Delete joint and its bones" } else { "Delete bone" };
        if ui.button(delete).clicked() {
            self.delete_skel_selection();
        }
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
            let r = ui.checkbox(&mut on, "");
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
        egui::ComboBox::from_id_salt(("parent", p)).selected_text(name(current)).show_ui(ui, |ui| {
            for &q in &options {
                ui.selectable_value(&mut chosen, q, name(q));
            }
        });
        if chosen != current {
            self.edit_skeleton(|s| {
                s.set_parent(p, chosen);
            });
        }
    }

}

/// The shortcuts listed at the bottom of each step's panel: (keys, what they do).
fn step_shortcuts(step: Step) -> &'static [(&'static str, &'static str)] {
    match step {
        Step::Review => &[
            ("Ctrl+O", "Open a drawing or project"),
            ("Ctrl+S", "Save"),
            ("Drag", "Pan the view"),
            ("Scroll", "Zoom"),
            ("F", "Fit to the window"),
        ],
        Step::Rig => &[
            ("Drag joint", "Move it"),
            ("Shift+drag", "Grow a new bone"),
            ("Click", "Select a joint or bone"),
            ("Delete", "Remove the selection"),
            ("[  ]", "Send bone back / forward"),
            ("Drag (brush on)", "Paint the selected bone's weights"),
            ("Ctrl+Z", "Undo"),
            ("Ctrl+Shift+Z", "Redo"),
            ("Middle-drag", "Pan the view"),
            ("Scroll", "Zoom"),
            ("F", "Fit to the window"),
        ],
        Step::Animate => &[
            ("Drag hand/foot", "Move the whole limb (IK)"),
            ("Drag bone end", "Turn that part"),
            ("Drag root dot", "Move the whole figure"),
            ("K", "Keep the pose at this frame"),
            ("Space", "Play / pause"),
            (",  .", "Previous / next frame"),
            ("B", "Show / hide bones"),
            ("Ctrl+A", "Select all keys"),
            ("Shift+drag row", "Select the keys in that stretch"),
            ("Ctrl+scroll row", "Zoom the timeline"),
            ("Ctrl+C X V", "Copy / cut / paste keys"),
            ("Delete", "Delete the selected keys"),
            ("Ctrl+Z", "Undo"),
            ("Middle-drag", "Pan the view"),
            ("Scroll", "Zoom"),
        ],
        Step::Export => &[
            ("Space", "Play / stop the preview"),
            ("Drag", "Pan the view"),
            ("Scroll", "Zoom"),
            ("F", "Fit to the window"),
        ],
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

    /// How many parts the part map uses (highest part number + 1).
    fn parts_in_map(doc: &Doc) -> usize {
        doc.skel_labels.iter().filter(|&&l| l != NO_PART).map(|&l| l as usize + 1).max().unwrap_or(0)
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
        assert_eq!(parts_in_map(state.doc.as_ref().unwrap()), 10, "the part split follows the skeleton");

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
        assert_eq!(parts_in_map(b), 10, "parts split again on open");
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
    fn layers_change_paint_order_keep_the_pose_and_undo() {
        let mut state = open_sample();
        state.set_step(Step::Animate);
        let arm = {
            let s = &state.doc.as_ref().unwrap().skeleton;
            s.parts.iter().position(|p| p.name == "left upper arm").unwrap()
        };
        state.doc.as_mut().unwrap().pose.set_angle(arm, 0.4);
        let before = state.doc.as_ref().unwrap().skeleton.paint_order();
        state.change_layer(arm, Toward::BackMost);
        let doc = state.doc.as_ref().unwrap();
        assert_eq!(doc.skeleton.paint_order()[0], arm, "sent behind everything");
        assert_eq!(doc.pose.angle(arm), 0.4, "the pose is kept");
        assert!(doc.dirty);
        state.undo();
        assert_eq!(state.doc.as_ref().unwrap().skeleton.paint_order(), before);
    }

    #[test]
    fn a_quick_motion_fills_the_timeline_plays_and_undoes() {
        let mut state = open_sample();
        let side = state.templates.iter().position(|t| t.name == "Person (side)").expect("built in");
        state.apply_template(side);
        state.set_step(Step::Animate);
        let skel = state.doc.as_ref().unwrap().skeleton.clone();
        let fitting: Vec<Cycle> = state.cycles.iter().filter(|c| c.matches(&skel)).cloned().collect();
        assert!(fitting.iter().any(|c| c.name == "Walk") && !fitting.iter().any(|c| c.name == "Wave"));

        let walk = fitting.iter().find(|c| c.name == "Walk").unwrap().clone();
        state.apply_cycle(&walk);
        let m = &state.doc.as_ref().unwrap().motion;
        assert_eq!(m.duration, walk.duration);
        assert!(m.tracks.contains_key("near thigh") && !m.offset_y.is_empty());
        assert!(state.playing);
        state.undo();
        assert!(state.doc.as_ref().unwrap().motion.is_empty());
    }

    #[test]
    fn templates_apply_save_and_start_new_drawings() {
        let mut state = open_sample();
        let side = state.templates.iter().position(|t| t.name == "Person (side)").expect("built in");

        // Use the side template: one undo step, far limbs behind the torso.
        state.apply_template(side);
        let skel = &state.doc.as_ref().unwrap().skeleton;
        assert!(skel.parts.iter().any(|p| p.name == "near foot"));
        assert_eq!(skel.parts[skel.paint_order()[0]].name, "far upper arm");
        state.undo();
        assert!(state.doc.as_ref().unwrap().skeleton.parts.iter().any(|p| p.name == "left forearm"));
        state.redo();

        // Facing left mirrors it: the toes point left.
        state.facing_left = true;
        state.apply_template(side);
        let skel = &state.doc.as_ref().unwrap().skeleton;
        let pos = |n: &str| skel.joints.iter().find(|j| j.name == n).unwrap().pos;
        assert!(pos("near toe")[0] < pos("near ankle")[0]);
        let knee = pos("near knee");
        state.edit_skeleton(|s| {
            s.swap_near_far();
        });
        let skel = &state.doc.as_ref().unwrap().skeleton;
        assert_eq!(skel.joints.iter().find(|j| j.name == "far knee").unwrap().pos, knee);
        state.facing_left = false;
        state.apply_template(side);

        // Save it as my own template (in the test app-data folder): it's listed and picked.
        let name = format!("Test monster {}", std::process::id());
        state.save_template(&name);
        assert_eq!(state.templates[state.template_choice].name, name, "{}", state.status);
        assert!(crate::templates::all().iter().any(|t| t.name == name));

        // The next drawing starts with the chosen template.
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../testdata/synthetic_sketch.png");
        state.open(&egui::Context::default(), &path);
        assert!(state.doc.as_ref().unwrap().skeleton.parts.iter().any(|p| p.name == "near foot"));
        let _ = std::fs::remove_dir_all(crate::paths::app_data_dir());
    }

    #[test]
    fn every_part_gets_a_mesh_that_follows_the_pose() {
        let mut state = open_sample();
        let ctx = egui::Context::default();
        state.set_step(Step::Animate);
        // A new drawing bends at the joints: some points follow two bones.
        state.refresh_sprites(&ctx);
        let doc = state.doc.as_ref().unwrap();
        assert!(state.softness > 0.0);
        assert!(doc.meshes.iter().any(|m| m.weights.iter().any(|w| w.len() == 2)), "blended near joints");
        // Softness 0: rigid pieces again.
        state.softness = 0.0;
        state.doc.as_mut().unwrap().sprites_dirty = true;
        state.refresh_sprites(&ctx);
        let doc = state.doc.as_ref().unwrap();
        assert_eq!(doc.meshes.len(), doc.skeleton.parts.len());
        for (m, s) in doc.meshes.iter().zip(&doc.sprites) {
            assert_eq!(m.triangles.is_empty(), s.image.width() == 0, "part {}: a mesh exactly when it has pixels", m.part);
        }
        // Rigid for now: a posed part's mesh moves exactly with its bone.
        let arm = doc.meshes.iter().position(|m| !m.triangles.is_empty() && m.part != 1).unwrap();
        let mut pose = doc.pose.clone();
        pose.set_angle(arm, 0.6);
        let t = anim_core::world_transforms(&doc.skeleton, &pose);
        let m = &doc.meshes[arm];
        for (moved, p) in m.deform(&t).iter().zip(&m.points) {
            let want = t[arm].apply(*p);
            assert!((moved[0] - want[0]).abs() < 1e-3 && (moved[1] - want[1]).abs() < 1e-3);
        }
    }

    #[test]
    fn weight_brush_paints_undoes_saves_and_follows_renames() {
        let mut state = open_sample();
        let ctx = egui::Context::default();
        state.set_step(Step::Rig);
        state.refresh_sprites(&ctx);
        let head = state.doc.as_ref().unwrap().skeleton.parts.iter().position(|p| p.name == "head").unwrap();
        let torso_mesh = 1;
        let pull = |state: &AppState| -> f32 {
            let m = &state.doc.as_ref().unwrap().meshes[torso_mesh];
            m.weights.iter().map(|w| w.iter().find(|x| x.0 == head).map_or(0.0, |x| x.1)).sum()
        };
        let before_paint = pull(&state);

        // Paint the head onto the middle of the torso: one stroke, one undo step.
        state.skel_sel = SkelSel::Part(head);
        state.paint_mode = true;
        let mid = {
            let m = &state.doc.as_ref().unwrap().meshes[torso_mesh];
            m.points[m.points.len() / 2]
        };
        let before = state.snapshot();
        for _ in 0..5 {
            state.paint_dab(mid);
        }
        state.commit(before);
        assert!(pull(&state) > before_paint + 0.5, "the torso now follows the head there");

        // Saved with the project, under the bone's name; renaming keeps it.
        let path = std::env::temp_dir().join(format!("pm2d_paint_test_{}.pm2d", std::process::id()));
        state.save_to(&path);
        let mut fresh = AppState::default();
        fresh.open(&ctx, &path);
        assert_eq!(fresh.doc.as_ref().unwrap().paint, state.doc.as_ref().unwrap().paint);
        std::fs::remove_file(&path).unwrap();
        let before = state.snapshot();
        state.doc.as_mut().unwrap().skeleton.parts[head].name = "big head".into();
        state.commit(before);
        assert!(state.doc.as_ref().unwrap().paint.bones.contains_key("big head"));

        // Undo takes the rename and then the stroke away.
        state.undo();
        state.undo();
        assert!(state.doc.as_ref().unwrap().paint.is_empty());
        state.refresh_sprites(&ctx);
        assert!((pull(&state) - before_paint).abs() < 1e-4);
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
