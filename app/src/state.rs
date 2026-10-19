use crate::canvas::{Drag, View};
use anim_core::{Axis, Mask, Params, PartBox, RectI, Skeleton, foreground_mask, part_masks, skeleton_masks};
use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions};
use image::RgbaImage;
use std::path::Path;

/// Tint colors for part masks, cycled per box.
pub const PALETTE: [[u8; 3]; 6] =
    [[255, 80, 80], [80, 170, 255], [90, 220, 110], [255, 190, 40], [200, 100, 255], [40, 220, 220]];

pub fn box_color(i: usize) -> Color32 {
    let [r, g, b] = PALETTE[i % PALETTE.len()];
    Color32::from_rgb(r, g, b)
}

/// How parts are labeled: by drawing boxes, or by placing skeleton joints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Boxes,
    Joints,
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
}

impl Doc {
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
    }

    /// Place the template skeleton over the figure's bounding box.
    pub fn fit_skeleton(&mut self) {
        let (w, h) = self.image.dimensions();
        let bbox = self.fg.bbox().unwrap_or(RectI::new(0, 0, w as i32, h as i32));
        self.skeleton = Skeleton::person_front(bbox);
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
    pub selected: Option<usize>,
    pub show_fg: bool,
    pub show_parts: bool,
    pub show_overlap: bool,
    /// Give each overlap pixel to one part (nearest bone) instead of sharing it.
    pub resolve_overlaps: bool,
    /// Draw each skeleton part's measured thickness as a translucent capsule.
    pub show_capsules: bool,
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
            selected: None,
            show_fg: false,
            show_parts: true,
            show_overlap: true,
            resolve_overlaps: true,
            show_capsules: false,
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
        };
        doc.fit_skeleton();
        self.doc = Some(doc);
        self.selected = None;
        self.drag = Drag::None;
        self.fit_pending = true;
    }

    pub fn recompute_parts(&mut self) {
        if let Some(doc) = &mut self.doc {
            doc.recompute_parts(&self.params, self.resolve_overlaps);
        }
    }

    pub fn delete_selected(&mut self) {
        if let (Some(doc), Some(i)) = (&mut self.doc, self.selected.take()) {
            doc.boxes.remove(i);
            self.recompute_parts();
        }
    }

    /// Cycle the bone axis of the selected box.
    pub fn cycle_axis(&mut self) {
        if let (Some(doc), Some(i)) = (&mut self.doc, self.selected) {
            let axis = &mut doc.boxes[i].part.axis;
            *axis = axis.next();
            self.recompute_parts();
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
                Mode::Joints => doc.skel_masks.iter().collect(),
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
        ui.separator();

        let mode = self.mode;
        ui.horizontal(|ui| {
            ui.strong("Label with");
            ui.selectable_value(&mut self.mode, Mode::Joints, "Joints");
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
        ui.checkbox(&mut self.show_fg, "Show full foreground");
        ui.checkbox(&mut self.show_parts, "Show part masks");
        if self.mode == Mode::Boxes {
            ui.checkbox(&mut self.show_overlap, "Show overlaps (orange)");
        } else {
            ui.checkbox(&mut self.show_capsules, "Show part thickness");
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
        }
    }

    fn joints_section(&mut self, ui: &mut egui::Ui) {
        ui.strong("Skeleton");
        let Some(doc) = &mut self.doc else { return };
        if ui.button("Fit skeleton to figure").clicked() {
            doc.fit_skeleton();
        }
        for (i, part) in doc.skeleton.parts.iter().enumerate() {
            let px = doc.skel_masks.get(i).map_or(0, Mask::count);
            let r = doc.skel_radii.get(i).copied().unwrap_or(0.0);
            let text = format!("{}  thickness {:.0} px  mask {} px", part.name, 2.0 * r, px);
            ui.label(egui::RichText::new(text).color(box_color(i)));
        }
        ui.separator();
        ui.small(
            "Drag a joint dot onto the drawing: pivots (where a part attaches) and tips.\n\
             Middle-drag or Space+drag: pan   Scroll: zoom   F: fit",
        );
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
             Scroll: zoom   F: fit   A: cycle bone   Delete: remove box",
        );
    }
}

pub fn to_color_image(img: &RgbaImage) -> ColorImage {
    ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw())
}
