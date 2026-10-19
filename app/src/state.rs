use crate::canvas::{Drag, View};
use anim_core::{
    Axis, History, Mask, Params, PartBox, Pose, RectI, Skeleton, Sprite, build_sprites, foreground_mask, part_masks,
    skeleton_masks, write_parts,
};
use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions};
use image::RgbaImage;
use std::path::Path;

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

pub fn box_color(i: usize) -> Color32 {
    let [r, g, b] = PALETTE[i % PALETTE.len()];
    Color32::from_rgb(r, g, b)
}

/// What the canvas is for: placing joints, drawing boxes, or posing the cut-out parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Boxes,
    Joints,
    Pose,
}

/// What is selected in joints mode.
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
    pub boxes: Vec<PartBox>,
    pub pose: Pose,
}

pub struct BoxItem {
    pub part: PartBox,
    pub mask: Mask,
    pub pixels: usize,
}

/// A loaded drawing and everything derived from it.
pub struct Doc {
    pub image: RgbaImage,
    pub texture: TextureHandle,
    pub fg: Mask,
    pub boxes: Vec<BoxItem>,
    /// Pixels claimed by more than one box (before resolution).
    pub overlap: Mask,
    pub skeleton: Skeleton,
    /// Per-part masks and measured thickness from the skeleton.
    pub skel_masks: Vec<Mask>,
    pub skel_radii: Vec<f32>,
    pub overlay: Option<TextureHandle>,
    pub overlay_dirty: bool,
    pub history: History<Snapshot>,
    /// File name without extension, used to name exported parts.
    pub name: String,
    pub pose: Pose,
    /// Cut-out parts and their textures (None for an empty part), rebuilt when
    /// `sprites_dirty` is set.
    pub sprites: Vec<Sprite>,
    pub sprite_tex: Vec<Option<TextureHandle>>,
    pub sprites_dirty: bool,
}

impl Doc {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            skeleton: self.skeleton.clone(),
            boxes: self.boxes.iter().map(|b| b.part).collect(),
            pose: self.pose.clone(),
        }
    }

    /// Put back a snapshot's joints, boxes and pose (masks are recomputed by the caller).
    fn restore(&mut self, s: Snapshot) {
        self.skeleton = s.skeleton;
        self.boxes = s.boxes.into_iter().map(|part| BoxItem { part, mask: Mask::new(0, 0), pixels: 0 }).collect();
        self.pose = s.pose;
    }

    /// Back to the pose as drawn. Needed whenever bones are added or removed,
    /// since pose angles are stored by part number.
    pub fn reset_pose(&mut self) {
        self.pose = Pose::rest(self.skeleton.parts.len());
    }

    pub fn size(&self) -> egui::Vec2 {
        egui::vec2(self.image.width() as f32, self.image.height() as f32)
    }

    pub fn add_box(&mut self, rect: RectI) {
        let part = PartBox { rect, axis: Axis::Auto };
        self.boxes.push(BoxItem { part, mask: Mask::new(0, 0), pixels: 0 });
    }

    /// Recompute every part mask. Boxes affect each other through overlaps,
    /// so any box change recomputes all of them (each is box-sized work).
    pub fn recompute_parts(&mut self, params: &Params, resolve: bool) {
        let parts: Vec<PartBox> = self.boxes.iter().map(|b| b.part).collect();
        let pm = part_masks(&self.fg, &parts, params.keep_ratio, resolve);
        for (b, m) in self.boxes.iter_mut().zip(pm.masks) {
            b.pixels = m.count();
            b.mask = m;
        }
        self.overlap = pm.overlap;
        self.overlay_dirty = true;
    }

    pub fn recompute_skeleton(&mut self) {
        let sm = skeleton_masks(&self.fg, &self.skeleton);
        self.skel_masks = sm.masks;
        self.skel_radii = sm.radii;
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
    pub mode: Mode,
    pub params: Params,
    pub view: View,
    pub drag: Drag,
    /// Pointer position (image pixels) during the current drag.
    pub drag_pointer: Option<egui::Pos2>,
    /// State before the current drag, recorded for undo when the drag ends.
    pub drag_before: Option<Snapshot>,
    /// State before a name edit, recorded for undo when the text box loses focus.
    pub rename_before: Option<Snapshot>,
    /// Selected box (boxes mode).
    pub selected: Option<usize>,
    /// Selected joint or bone (joints mode).
    pub skel_sel: SkelSel,
    pub show_fg: bool,
    pub show_parts: bool,
    pub show_overlap: bool,
    /// Give each overlap pixel to one part (nearest bone) instead of sharing it.
    pub resolve_overlaps: bool,
    /// Draw each skeleton part's measured thickness as a translucent capsule.
    pub show_capsules: bool,
    /// Joint disc radius as a multiple of the child part's thickness (0 = no discs).
    pub overlap: f32,
    /// Pose mode: show the original drawing faintly behind the posed parts.
    pub show_ghost: bool,
    pub status: String,
    /// Set when a new image is loaded so the canvas fits it on the next frame.
    pub fit_pending: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            doc: None,
            mode: Mode::Joints,
            params: Params::default(),
            view: View::default(),
            drag: Drag::None,
            drag_pointer: None,
            drag_before: None,
            rename_before: None,
            selected: None,
            skel_sel: SkelSel::None,
            show_fg: false,
            show_parts: true,
            show_overlap: true,
            resolve_overlaps: true,
            show_capsules: false,
            overlap: 1.0,
            show_ghost: false,
            status: "Open or drop an image to start.".into(),
            fit_pending: false,
        }
    }
}

impl AppState {
    pub fn open(&mut self, ctx: &egui::Context, path: &Path) {
        let image = match image::open(path) {
            Ok(img) => img.to_rgba8(),
            Err(e) => {
                self.status = format!("Could not open {}: {e}", path.display());
                return;
            }
        };
        let max_side = ctx.input(|i| i.max_texture_side) as u32;
        if image.width() > max_side || image.height() > max_side {
            self.status = format!(
                "Image is {}×{}, larger than the GPU limit of {max_side}px per side.",
                image.width(),
                image.height()
            );
            return;
        }
        let texture = ctx.load_texture("drawing", to_color_image(&image), TextureOptions::LINEAR);
        let t = std::time::Instant::now();
        let fg = foreground_mask(&image, &self.params);
        self.status = format!(
            "{} — {}×{}, foreground in {} ms",
            path.file_name().unwrap_or_default().to_string_lossy(),
            image.width(),
            image.height(),
            t.elapsed().as_millis()
        );
        let overlap = Mask::new(image.width(), image.height());
        let mut doc = Doc {
            image,
            texture,
            fg,
            boxes: Vec::new(),
            overlap,
            skeleton: Skeleton { joints: Vec::new(), parts: Vec::new() },
            skel_masks: Vec::new(),
            skel_radii: Vec::new(),
            overlay: None,
            overlay_dirty: true,
            history: History::new(UNDO_LIMIT),
            name: path.file_stem().unwrap_or_default().to_string_lossy().into_owned(),
            pose: Pose::default(),
            sprites: Vec::new(),
            sprite_tex: Vec::new(),
            sprites_dirty: true,
        };
        doc.fit_skeleton();
        self.doc = Some(doc);
        self.selected = None;
        self.skel_sel = SkelSel::None;
        self.drag = Drag::None;
        self.drag_before = None;
        self.rename_before = None;
        self.fit_pending = true;
    }

    pub fn recompute_parts(&mut self) {
        if let Some(doc) = &mut self.doc {
            doc.recompute_parts(&self.params, self.resolve_overlaps);
        }
    }

    pub fn snapshot(&self) -> Option<Snapshot> {
        self.doc.as_ref().map(Doc::snapshot)
    }

    /// Record a finished edit for undo. `before` is the state it started from.
    pub fn commit(&mut self, before: Option<Snapshot>) {
        if let (Some(doc), Some(before)) = (&mut self.doc, before) {
            let after = doc.snapshot();
            doc.history.record(before, &after);
        }
    }

    /// Put back a snapshot and recompute everything derived from it.
    pub fn restore(&mut self, s: Snapshot) {
        if let Some(doc) = &mut self.doc {
            doc.restore(s);
            doc.recompute_skeleton();
        }
        self.recompute_parts();
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
        if self.selected.is_some_and(|i| i >= doc.boxes.len()) {
            self.selected = None;
        }
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

    /// Ask for a folder and write one PNG per part plus rig.json into `<folder>/<image>_parts/`.
    fn export_parts(&mut self, ctx: &egui::Context) {
        self.refresh_sprites(ctx);
        let Some(doc) = &self.doc else { return };
        let Some(folder) = rfd::FileDialog::new().set_title("Export parts into…").pick_folder() else { return };
        let dir = folder.join(format!("{}_parts", doc.name));
        self.status = match write_parts(&dir, &doc.sprites, &doc.skeleton) {
            Ok(()) => format!("Exported {} parts to {}", doc.sprites.len(), dir.display()),
            Err(e) => format!("Export failed: {e}"),
        };
    }

    /// Delete the selected joint or bone (joints mode).
    pub fn delete_skel_selection(&mut self) {
        match std::mem::replace(&mut self.skel_sel, SkelSel::None) {
            SkelSel::Joint(j) => self.edit_skeleton(|s| s.remove_joint(j)),
            SkelSel::Part(p) => self.edit_skeleton(|s| s.remove_part(p)),
            SkelSel::None => {}
        }
    }

    pub fn delete_selected(&mut self) {
        let before = self.snapshot();
        if let (Some(doc), Some(i)) = (&mut self.doc, self.selected.take()) {
            doc.boxes.remove(i);
            self.recompute_parts();
            self.commit(before);
        }
    }

    /// Cycle the bone axis of the selected box.
    pub fn cycle_axis(&mut self) {
        let before = self.snapshot();
        if let (Some(doc), Some(i)) = (&mut self.doc, self.selected) {
            let axis = &mut doc.boxes[i].part.axis;
            *axis = axis.next();
            self.recompute_parts();
            self.commit(before);
        }
    }

    /// Rebuild the tint overlay texture if anything changed.
    pub fn refresh_overlay(&mut self, ctx: &egui::Context) {
        let Some(doc) = &mut self.doc else { return };
        if !doc.overlay_dirty {
            return;
        }
        doc.overlay_dirty = false;
        let mut img = RgbaImage::new(doc.image.width(), doc.image.height());
        if self.show_fg {
            doc.fg.paint_onto(&mut img, [255, 0, 200, 90]);
        }
        if self.show_parts {
            let masks: Vec<&Mask> = match self.mode {
                Mode::Boxes => doc.boxes.iter().map(|b| &b.mask).collect(),
                Mode::Joints | Mode::Pose => doc.skel_masks.iter().collect(),
            };
            for (i, m) in masks.into_iter().enumerate() {
                let [r, g, bl] = PALETTE[i % PALETTE.len()];
                m.paint_onto(&mut img, [r, g, bl, 140]);
            }
        }
        if self.show_overlap && self.mode == Mode::Boxes {
            doc.overlap.and(&doc.fg).paint_onto(&mut img, [255, 120, 0, 230]);
        }
        let color = to_color_image(&img);
        match &mut doc.overlay {
            Some(tex) => tex.set(color, TextureOptions::NEAREST),
            None => doc.overlay = Some(ctx.load_texture("overlay", color, TextureOptions::NEAREST)),
        }
    }

    pub fn side_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Animator");
        if ui.button("Open…").clicked()
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("Images", &["png", "jpg", "jpeg", "webp", "bmp", "gif"])
                .pick_file()
        {
            self.open(ui.ctx(), &path);
        }
        ui.label(&self.status);
        if let Some(doc) = &self.doc {
            let (can_undo, can_redo) = (doc.history.can_undo(), doc.history.can_redo());
            ui.horizontal(|ui| {
                if ui.add_enabled(can_undo, egui::Button::new("⟲ Undo")).on_hover_text("Ctrl+Z").clicked() {
                    self.undo();
                }
                if ui.add_enabled(can_redo, egui::Button::new("⟳ Redo")).on_hover_text("Ctrl+Shift+Z").clicked() {
                    self.redo();
                }
            });
        }
        ui.separator();

        let mode = self.mode;
        ui.horizontal(|ui| {
            ui.strong("Mode");
            ui.selectable_value(&mut self.mode, Mode::Joints, "Joints");
            ui.selectable_value(&mut self.mode, Mode::Pose, "Pose");
            ui.selectable_value(&mut self.mode, Mode::Boxes, "Boxes");
        });
        if mode != self.mode {
            self.drag = Drag::None;
            if let Some(doc) = &mut self.doc {
                doc.overlay_dirty = true;
            }
        }
        ui.separator();

        ui.strong("Cutout");
        let before = self.params;
        ui.add(egui::Slider::new(&mut self.params.threshold, 5.0..=150.0).text("bg threshold"));
        ui.add(egui::Slider::new(&mut self.params.corner_patch, 2..=64).text("corner patch"));
        ui.add(egui::Slider::new(&mut self.params.min_area, 0..=5000).logarithmic(true).text("min area"));
        ui.add(egui::Slider::new(&mut self.params.keep_ratio, 0.0..=1.0).text("keep ratio"));
        ui.add(egui::Slider::new(&mut self.params.gap_close, 0..=20).text("close gaps (px)"));
        if ui.button("Reset").clicked() {
            self.params = Params::default();
        }
        if self.params != before
            && let Some(doc) = &mut self.doc
        {
            doc.fg = foreground_mask(&doc.image, &self.params);
            doc.recompute_skeleton();
            self.recompute_parts();
        }

        let toggles = (self.show_fg, self.show_parts, self.show_overlap);
        match self.mode {
            Mode::Boxes => {
                ui.checkbox(&mut self.show_fg, "Show full foreground");
                ui.checkbox(&mut self.show_parts, "Show part masks");
                ui.checkbox(&mut self.show_overlap, "Show overlaps (orange)");
            }
            Mode::Joints => {
                ui.checkbox(&mut self.show_fg, "Show full foreground");
                ui.checkbox(&mut self.show_parts, "Show part masks");
                ui.checkbox(&mut self.show_capsules, "Show part thickness");
            }
            Mode::Pose => {}
        }
        if toggles != (self.show_fg, self.show_parts, self.show_overlap)
            && let Some(doc) = &mut self.doc
        {
            doc.overlay_dirty = true;
        }
        ui.separator();

        match self.mode {
            Mode::Boxes => self.boxes_section(ui),
            Mode::Joints => self.joints_section(ui),
            Mode::Pose => self.pose_section(ui),
        }
    }

    fn pose_section(&mut self, ui: &mut egui::Ui) {
        ui.strong("Pose");
        if self.doc.is_none() {
            return;
        }
        if ui.button("Reset pose").clicked() {
            let before = self.snapshot();
            if let Some(doc) = &mut self.doc {
                doc.reset_pose();
            }
            self.commit(before);
        }
        let overlap = self.overlap;
        ui.add(egui::Slider::new(&mut self.overlap, 0.0..=2.0).text("joint overlap"))
            .on_hover_text("How big a round blob each part keeps at its joints so rotating a child leaves no gap");
        if overlap != self.overlap
            && let Some(doc) = &mut self.doc
        {
            doc.sprites_dirty = true;
        }
        ui.checkbox(&mut self.show_ghost, "Show original drawing behind");
        if ui.button("Export parts…").on_hover_text("One PNG per part plus rig.json").clicked() {
            self.export_parts(ui.ctx());
        }
        ui.separator();
        ui.small(
            "Drag a bone's end dot: rotate that part (children follow)\n\
             Drag the root's start dot: move the whole figure\n\
             Ctrl+Z: undo   Ctrl+Shift+Z: redo\n\
             Middle-drag or Space+drag: pan   Scroll: zoom   F: fit",
        );
    }

    fn joints_section(&mut self, ui: &mut egui::Ui) {
        ui.strong("Skeleton");
        if self.doc.is_none() {
            return;
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
        for (i, part) in doc.skeleton.parts.iter().enumerate() {
            let px = doc.skel_masks.get(i).map_or(0, Mask::count);
            let r = doc.skel_radii.get(i).copied().unwrap_or(0.0);
            let text = format!("{}  thickness {:.0} px  mask {} px", part.name, 2.0 * r, px);
            let text = egui::RichText::new(text).color(box_color(i));
            if ui.selectable_label(self.skel_sel == SkelSel::Part(i), text).clicked() {
                clicked = Some(i);
            }
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
                self.rename_before = before;
            }
            if resp.lost_focus() {
                let before = self.rename_before.take();
                self.commit(before);
            }
            if let SkelSel::Part(p) = sel {
                self.parent_picker(ui, p);
            }
            let delete = if matches!(sel, SkelSel::Joint(_)) { "Delete joint and its bones" } else { "Delete bone" };
            if ui.button(delete).clicked() {
                self.delete_skel_selection();
            }
        }

        ui.separator();
        ui.small(
            "Drag a joint: move it\n\
             Shift+drag from a joint: grow a new bone\n\
             Shift+drag on empty space: start a new bone\n\
             Click a joint or bone: select it   Delete: remove it\n\
             Ctrl+Z: undo   Ctrl+Shift+Z: redo\n\
             Middle-drag or Space+drag: pan   Scroll: zoom   F: fit",
        );
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

    fn boxes_section(&mut self, ui: &mut egui::Ui) {
        let resolve = self.resolve_overlaps;
        ui.checkbox(&mut self.resolve_overlaps, "Resolve overlaps (nearest bone)");
        if resolve != self.resolve_overlaps {
            self.recompute_parts();
        }
        ui.strong("Boxes");
        if let Some(doc) = &self.doc {
            if doc.boxes.is_empty() {
                ui.label("Drag on the drawing to add a box.");
            } else {
                let shared = doc.overlap.and(&doc.fg).count();
                ui.label(format!("Overlap: {shared} px claimed by 2+ boxes"));
            }
            let mut clicked = None;
            for (i, b) in doc.boxes.iter().enumerate() {
                let r = &b.part.rect;
                let text = egui::RichText::new(format!(
                    "Box {}  {}×{}  bone {}  mask {} px",
                    i + 1,
                    r.w,
                    r.h,
                    b.part.axis.label(),
                    b.pixels
                ))
                .color(box_color(i));
                if ui.selectable_label(self.selected == Some(i), text).clicked() {
                    clicked = Some(i);
                }
            }
            if clicked.is_some() {
                self.selected = clicked;
            }
        }
        if self.selected.is_some() {
            ui.horizontal(|ui| {
                if ui.button("Cycle bone (A)").clicked() {
                    self.cycle_axis();
                }
                if ui.button("Delete box").clicked() {
                    self.delete_selected();
                }
            });
        }
        ui.separator();
        ui.small(
            "Left-drag: new box / move box\nCorner handles: resize\nMiddle-drag or Space+drag: pan\n\
             Scroll: zoom   F: fit   A: cycle bone   Delete: remove box\n\
             Ctrl+Z: undo   Ctrl+Shift+Z: redo",
        );
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
        assert_eq!(bone_count(&state), 6, "starts with the person template");
        assert!(!state.doc.as_ref().unwrap().history.can_undo());

        // Grow a tail from the pelvis (joint 2), then delete the head bone.
        state.edit_skeleton(|s| {
            let tip = s.add_joint([300.0, 300.0]);
            s.add_part(2, tip);
        });
        state.skel_sel = SkelSel::Part(0);
        state.delete_skel_selection();
        assert_eq!(bone_count(&state), 6);
        assert_eq!(state.doc.as_ref().unwrap().skel_masks.len(), 6, "masks follow the skeleton");

        state.undo();
        assert_eq!(bone_count(&state), 7, "head is back");
        state.undo();
        assert_eq!(bone_count(&state), 6, "tail is gone");
        assert!(!state.doc.as_ref().unwrap().history.can_undo());

        state.redo();
        state.redo();
        assert_eq!(bone_count(&state), 6);
        let names: Vec<_> = state.doc.as_ref().unwrap().skeleton.parts.iter().map(|p| p.name.clone()).collect();
        assert!(!names.contains(&"head".to_string()) && names.contains(&"bone 1".to_string()));
    }

    #[test]
    fn pose_undo_and_reset_on_skeleton_edit() {
        let mut state = open_sample();
        let ctx = egui::Context::default();
        state.refresh_sprites(&ctx);
        let doc = state.doc.as_ref().unwrap();
        assert_eq!(doc.sprites.len(), 6);
        assert!(doc.sprite_tex.iter().all(Option::is_some), "every template part has pixels");

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
        assert_eq!(bone_count(&state), 6);
        assert_eq!(state.doc.as_ref().unwrap().pose.angle(2), 1.0);
    }

    #[test]
    fn box_edits_undo() {
        let mut state = open_sample();
        let before = state.snapshot();
        state.doc.as_mut().unwrap().add_box(RectI::new(10, 10, 50, 50));
        state.recompute_parts();
        state.commit(before);
        state.selected = Some(0);
        state.delete_selected();
        assert!(state.doc.as_ref().unwrap().boxes.is_empty());
        state.undo();
        assert_eq!(state.doc.as_ref().unwrap().boxes.len(), 1);
        state.undo();
        assert!(state.doc.as_ref().unwrap().boxes.is_empty());
    }
}
