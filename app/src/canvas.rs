//! Image canvas: pan/zoom view, box tools, and draggable skeleton joints.

use crate::state::{AppState, Mode, SkelSel, box_color};
use anim_core::skeleton::dist_to_segment;
use anim_core::{PartBox, RectI};
use eframe::egui::{self, Color32, Key, PointerButton, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};

const HANDLE_PX: f32 = 6.0;
const JOINT_HIT_PX: f32 = 10.0;
const BONE_HIT_PX: f32 = 6.0;
const MIN_ZOOM: f32 = 0.05;
const MAX_ZOOM: f32 = 32.0;

/// Maps image pixels to screen points: `screen = canvas.min + offset + img * zoom`.
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub offset: Vec2,
    pub zoom: f32,
}

impl Default for View {
    fn default() -> Self {
        Self { offset: Vec2::ZERO, zoom: 1.0 }
    }
}

impl View {
    pub fn image_to_screen(&self, canvas: Rect, p: Pos2) -> Pos2 {
        canvas.min + self.offset + p.to_vec2() * self.zoom
    }

    pub fn screen_to_image(&self, canvas: Rect, p: Pos2) -> Pos2 {
        ((p - canvas.min - self.offset) / self.zoom).to_pos2()
    }

    pub fn rect_to_screen(&self, canvas: Rect, r: &RectI) -> Rect {
        Rect::from_min_max(
            self.image_to_screen(canvas, pos2(r.x as f32, r.y as f32)),
            self.image_to_screen(canvas, pos2(r.right() as f32, r.bottom() as f32)),
        )
    }

    pub fn fit(canvas: Rect, image: Vec2) -> Self {
        let zoom = ((canvas.width() / image.x).min(canvas.height() / image.y) * 0.95).clamp(MIN_ZOOM, MAX_ZOOM);
        Self { offset: (canvas.size() - image * zoom) / 2.0, zoom }
    }

    /// Zoom by `factor` keeping the image point under `anchor` fixed on screen.
    fn zoom_around(&mut self, canvas: Rect, anchor: Pos2, factor: f32) {
        let before = self.screen_to_image(canvas, anchor);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.offset = anchor - canvas.min - before.to_vec2() * self.zoom;
    }
}

/// In-progress mouse interaction. Positions are in image pixels.
#[derive(Clone, Copy, Debug, Default)]
pub enum Drag {
    #[default]
    None,
    Pan,
    Create { start: Pos2, current: Pos2 },
    Move { index: usize, grab: Vec2 },
    /// `anchor` is the fixed corner opposite the dragged handle.
    Resize { index: usize, anchor: Pos2 },
    /// Moving a joint. `new_bone` is set when the joint is the tip of a bone
    /// created by this drag (Shift+drag), which is dropped if left too short.
    Joint { index: usize, new_bone: bool },
}

/// A bone shorter than this (image px) after Shift+drag is discarded.
const MIN_BONE_PX: f32 = 3.0;

fn round(p: Pos2) -> (i32, i32) {
    (p.x.round() as i32, p.y.round() as i32)
}

fn corners(r: &RectI) -> [Pos2; 4] {
    let (x0, y0, x1, y1) = (r.x as f32, r.y as f32, r.right() as f32, r.bottom() as f32);
    [pos2(x0, y0), pos2(x1, y0), pos2(x0, y1), pos2(x1, y1)]
}

pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let canvas = response.rect;
    painter.rect_filled(canvas, 0.0, Color32::from_gray(40));

    let Some(image_size) = state.doc.as_ref().map(|d| d.size()) else {
        painter.text(
            canvas.center(),
            egui::Align2::CENTER_CENTER,
            "Drop an image here or click Open…",
            egui::FontId::proportional(18.0),
            Color32::GRAY,
        );
        return;
    };

    // Single-key shortcuts are off while typing in a text box.
    let typing = ui.ctx().text_edit_focused();
    let key = |k: Key| !typing && ui.input(|i| i.key_pressed(k));
    if state.fit_pending || (response.hovered() && key(Key::F)) {
        state.view = View::fit(canvas, image_size);
        state.fit_pending = false;
    }
    if key(Key::Delete) || key(Key::Backspace) {
        match state.mode {
            Mode::Boxes => state.delete_selected(),
            Mode::Joints => state.delete_skel_selection(),
        }
    }
    if state.mode == Mode::Boxes && response.hovered() && key(Key::A) {
        state.cycle_axis();
    }

    // Zoom with scroll wheel / pinch around the cursor.
    if let Some(hover) = response.hover_pos() {
        let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        let factor = (scroll * 0.0015).exp() * pinch;
        if factor != 1.0 {
            state.view.zoom_around(canvas, hover, factor);
        }
    }

    handle_pointer(ui, &response, canvas, state);
    state.refresh_overlay(ui.ctx());

    // Draw image, overlay, boxes.
    let view = state.view;
    let doc = state.doc.as_ref().unwrap();
    let image_rect = Rect::from_min_size(view.image_to_screen(canvas, Pos2::ZERO), image_size * view.zoom);
    let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
    let painter = painter.with_clip_rect(canvas);
    painter.image(doc.texture.id(), image_rect, uv, Color32::WHITE);
    if let Some(overlay) = &doc.overlay {
        painter.image(overlay.id(), image_rect, uv, Color32::WHITE);
    }
    match state.mode {
        Mode::Boxes => draw_boxes(&painter, canvas, state),
        Mode::Joints => draw_skeleton(&painter, canvas, state, response.hover_pos()),
    }
}

fn draw_skeleton(painter: &egui::Painter, canvas: Rect, state: &AppState, hover: Option<Pos2>) {
    let view = state.view;
    let doc = state.doc.as_ref().unwrap();
    let skel = &doc.skeleton;
    let to_screen = |p: [f32; 2]| view.image_to_screen(canvas, pos2(p[0], p[1]));
    for i in 0..skel.parts.len() {
        let (a, b) = skel.bone(i);
        let (a, b) = (to_screen(a), to_screen(b));
        if state.show_capsules {
            let r = doc.skel_radii.get(i).copied().unwrap_or(0.0) * view.zoom;
            let fill = box_color(i).gamma_multiply(0.25);
            painter.line_segment([a, b], Stroke::new(2.0 * r, fill));
            painter.circle_filled(a, r, fill);
            painter.circle_filled(b, r, fill);
        }
        let selected = state.skel_sel == SkelSel::Part(i);
        let (outline, core) = if selected { (Color32::WHITE, 4.0) } else { (Color32::from_black_alpha(160), 2.0) };
        painter.line_segment([a, b], Stroke::new(core + 2.0, outline));
        painter.line_segment([a, b], Stroke::new(core, box_color(i)));
        if selected {
            label(painter, a.lerp(b, 0.5) + vec2(8.0, 8.0), &skel.parts[i].name);
        }
    }
    let active = match state.drag {
        Drag::Joint { index, .. } => Some(index),
        _ => hover.and_then(|h| hit_joint(state, canvas, h)),
    };
    for (i, j) in skel.joints.iter().enumerate() {
        let p = to_screen(j.pos);
        let r = if active == Some(i) { 7.0 } else { 5.0 };
        let fill = if state.skel_sel == SkelSel::Joint(i) { Color32::YELLOW } else { Color32::WHITE };
        painter.circle(p, r, fill, Stroke::new(1.5, Color32::BLACK));
    }
    if let Some(i) = active {
        label(painter, to_screen(skel.joints[i].pos) + vec2(10.0, -10.0), &skel.joints[i].name);
    }
}

/// Small text tag with a dark background, top-left at `pos`.
fn label(painter: &egui::Painter, pos: Pos2, text: &str) {
    let galley = painter.layout_no_wrap(text.to_owned(), egui::FontId::proportional(14.0), Color32::WHITE);
    let bg = Rect::from_min_size(pos, galley.size()).expand(3.0);
    painter.rect_filled(bg, 3.0, Color32::from_black_alpha(200));
    painter.galley(pos, galley, Color32::WHITE);
}

/// Bone under the screen position `p`, if any (nearest within a few points).
fn hit_part(state: &AppState, canvas: Rect, p: Pos2) -> Option<usize> {
    let doc = state.doc.as_ref()?;
    let skel = &doc.skeleton;
    let to_screen = |q: [f32; 2]| {
        let s = state.view.image_to_screen(canvas, pos2(q[0], q[1]));
        [s.x, s.y]
    };
    (0..skel.parts.len())
        .map(|i| {
            let (a, b) = skel.bone(i);
            (i, dist_to_segment([p.x, p.y], to_screen(a), to_screen(b)))
        })
        .filter(|&(_, d)| d <= BONE_HIT_PX)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// Start a primary-button drag in joints mode at screen position `o`.
/// Plain drag moves a joint; Shift+drag grows a new bone from a joint or,
/// on empty space, creates a new bone with two fresh joints.
fn begin_joint_drag(state: &mut AppState, canvas: Rect, o: Pos2, shift: bool) -> Drag {
    let hit = hit_joint(state, canvas, o);
    let p = state.view.screen_to_image(canvas, o);
    let Some(doc) = &mut state.doc else { return Drag::None };
    let p = [p.x, p.y];
    match (hit, shift) {
        (Some(index), false) => {
            state.skel_sel = SkelSel::Joint(index);
            Drag::Joint { index, new_bone: false }
        }
        (Some(pivot), true) => {
            let tip = doc.skeleton.add_joint(p);
            state.skel_sel = SkelSel::Part(doc.skeleton.add_part(pivot, tip));
            Drag::Joint { index: tip, new_bone: true }
        }
        (None, true) => {
            let pivot = doc.skeleton.add_joint(p);
            let tip = doc.skeleton.add_joint(p);
            state.skel_sel = SkelSel::Part(doc.skeleton.add_part(pivot, tip));
            Drag::Joint { index: tip, new_bone: true }
        }
        (None, false) => Drag::None,
    }
}

/// Joint under the screen position `p`, if any (nearest within a few points).
fn hit_joint(state: &AppState, canvas: Rect, p: Pos2) -> Option<usize> {
    let doc = state.doc.as_ref()?;
    doc.skeleton
        .joints
        .iter()
        .enumerate()
        .map(|(i, j)| (i, state.view.image_to_screen(canvas, pos2(j.pos[0], j.pos[1])).distance(p)))
        .filter(|&(_, d)| d <= JOINT_HIT_PX)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

fn draw_boxes(painter: &egui::Painter, canvas: Rect, state: &AppState) {
    let view = state.view;
    let doc = state.doc.as_ref().unwrap();
    for (i, b) in doc.boxes.iter().enumerate() {
        let selected = state.selected == Some(i);
        let live = PartBox { rect: live_rect(state, i, b.part.rect), axis: b.part.axis };
        let r = view.rect_to_screen(canvas, &live.rect);
        let width = if selected { 2.5 } else { 1.5 };
        painter.rect_stroke(r, 0.0, Stroke::new(width, box_color(i)), StrokeKind::Middle);
        // Stand-in bone used for overlap resolution.
        let (a, c) = live.bone();
        let (a, c) = (view.image_to_screen(canvas, pos2(a[0], a[1])), view.image_to_screen(canvas, c.into()));
        let bone = Stroke::new(1.5, box_color(i).gamma_multiply(0.8));
        painter.add(egui::Shape::dashed_line(&[a, c], bone, 6.0, 4.0));
        painter.circle_filled(a, 3.0, box_color(i));
        painter.circle_filled(c, 3.0, box_color(i));
        if selected {
            for c in [r.left_top(), r.right_top(), r.left_bottom(), r.right_bottom()] {
                painter.rect_filled(Rect::from_center_size(c, Vec2::splat(HANDLE_PX * 1.5)), 1.0, Color32::WHITE);
            }
        }
    }
    if let Drag::Create { start, current } = state.drag {
        let r = Rect::from_two_pos(view.image_to_screen(canvas, start), view.image_to_screen(canvas, current));
        painter.rect_stroke(r, 0.0, Stroke::new(1.5, Color32::YELLOW), StrokeKind::Middle);
    }
}

/// The rectangle to draw for box `i`, including an in-progress move/resize.
fn live_rect(state: &AppState, i: usize, rect: RectI) -> RectI {
    let Some(doc) = &state.doc else { return rect };
    let (w, h) = doc.image.dimensions();
    let pointer = state.drag_pointer;
    match (state.drag, pointer) {
        (Drag::Move { index, grab }, Some(p)) if index == i => {
            let (x, y) = round(p - grab);
            RectI::new(x.clamp(-rect.w + 1, w as i32 - 1), y.clamp(-rect.h + 1, h as i32 - 1), rect.w, rect.h)
        }
        (Drag::Resize { index, anchor }, Some(p)) if index == i => {
            let (ax, ay) = round(anchor);
            let (px, py) = round(p);
            RectI::from_corners(ax, ay, px, py).clamp_to(w, h)
        }
        _ => rect,
    }
}

fn handle_pointer(ui: &egui::Ui, response: &egui::Response, canvas: Rect, state: &mut AppState) {
    let view = state.view;
    let pointer_img = response.interact_pointer_pos().map(|p| view.screen_to_image(canvas, p));
    let space = ui.input(|i| i.key_down(Key::Space));

    if response.drag_started() {
        let pan = response.dragged_by(PointerButton::Middle) || (space && response.dragged_by(PointerButton::Primary));
        // Hit-test where the button went down, not where the drag threshold was crossed.
        let origin = ui.input(|i| i.pointer.press_origin()).or(response.interact_pointer_pos());
        let shift = ui.input(|i| i.modifiers.shift);
        // Snapshot for undo; recorded when the drag ends if anything changed.
        state.drag_before = state.snapshot();
        state.drag = if pan {
            Drag::Pan
        } else if let (true, Some(o)) = (response.dragged_by(PointerButton::Primary), origin) {
            match state.mode {
                Mode::Boxes => begin_drag(state, view.screen_to_image(canvas, o)),
                Mode::Joints => begin_joint_drag(state, canvas, o, shift),
            }
        } else {
            Drag::None
        };
    }

    if response.dragged() {
        match &mut state.drag {
            Drag::Pan => state.view.offset += response.drag_delta(),
            Drag::Create { current, .. } => {
                if let Some(p) = pointer_img {
                    *current = p;
                }
            }
            &mut Drag::Joint { index, .. } => {
                if let (Some(p), Some(doc)) = (pointer_img, &mut state.doc) {
                    let (w, h) = doc.image.dimensions();
                    doc.skeleton.joints[index].pos = [p.x.clamp(0.0, w as f32), p.y.clamp(0.0, h as f32)];
                    doc.recompute_skeleton();
                }
            }
            _ => {}
        }
        state.drag_pointer = pointer_img;
    }

    if response.drag_stopped() {
        finish_drag(state);
        let before = state.drag_before.take();
        if is_stub_bone(state)
            && let Some(before) = before
        {
            // A Shift+click without real movement: undo the bone silently.
            state.restore(before);
        } else {
            state.commit(before);
        }
        state.drag = Drag::None;
        state.drag_pointer = None;
    }

    if response.clicked()
        && let Some(screen) = response.interact_pointer_pos()
    {
        match state.mode {
            Mode::Boxes => state.selected = hit_box(state, view.screen_to_image(canvas, screen)),
            Mode::Joints => {
                state.skel_sel = match (hit_joint(state, canvas, screen), hit_part(state, canvas, screen)) {
                    (Some(j), _) => SkelSel::Joint(j),
                    (None, Some(p)) => SkelSel::Part(p),
                    (None, None) => SkelSel::None,
                };
            }
        }
    }
}

/// Whether the drag that just ended created a bone too short to keep.
fn is_stub_bone(state: &AppState) -> bool {
    let (Drag::Joint { index, new_bone: true }, Some(doc)) = (state.drag, &state.doc) else {
        return false;
    };
    let skel = &doc.skeleton;
    skel.parts.iter().any(|p| {
        p.tip == index && {
            let (a, b) = (skel.joints[p.pivot].pos, skel.joints[p.tip].pos);
            (a[0] - b[0]).hypot(a[1] - b[1]) < MIN_BONE_PX
        }
    })
}

fn begin_drag(state: &mut AppState, p: Pos2) -> Drag {
    let doc = state.doc.as_ref().unwrap();
    // Resize handles on the selected box take priority.
    if let Some(i) = state.selected {
        let cs = corners(&doc.boxes[i].part.rect);
        let tol = HANDLE_PX / state.view.zoom;
        for (k, c) in cs.iter().enumerate() {
            if (p - *c).length() <= tol * 1.5 {
                return Drag::Resize { index: i, anchor: cs[3 - k] };
            }
        }
    }
    if let Some(i) = hit_box(state, p) {
        state.selected = Some(i);
        let r = doc.boxes[i].part.rect;
        return Drag::Move { index: i, grab: p - pos2(r.x as f32, r.y as f32) };
    }
    state.selected = None;
    Drag::Create { start: p, current: p }
}

/// Topmost (last drawn) box containing `p`.
fn hit_box(state: &AppState, p: Pos2) -> Option<usize> {
    let doc = state.doc.as_ref()?;
    let (x, y) = (p.x.floor() as i32, p.y.floor() as i32);
    doc.boxes.iter().rposition(|b| b.part.rect.contains(x, y))
}

fn finish_drag(state: &mut AppState) {
    let drag = state.drag;
    let Some(doc) = &state.doc else { return };
    let (w, h) = doc.image.dimensions();
    let changed = match drag {
        Drag::Create { start, current } => {
            let (x0, y0) = round(start);
            let (x1, y1) = round(current);
            let r = RectI::from_corners(x0, y0, x1, y1).clamp_to(w, h);
            let ok = r.w >= 3 && r.h >= 3;
            if ok {
                let doc = state.doc.as_mut().unwrap();
                doc.add_box(r);
                state.selected = Some(doc.boxes.len() - 1);
            }
            ok
        }
        Drag::Move { index, .. } | Drag::Resize { index, .. } => {
            let old = doc.boxes[index].part.rect;
            let new = live_rect(state, index, old);
            let ok = new != old && new.w >= 3 && new.h >= 3;
            if ok {
                state.doc.as_mut().unwrap().boxes[index].part.rect = new;
            }
            ok
        }
        Drag::None | Drag::Pan | Drag::Joint { .. } => false,
    };
    if changed {
        state.recompute_parts();
    }
}
