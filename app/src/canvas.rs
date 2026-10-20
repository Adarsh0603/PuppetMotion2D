//! Image canvas: pan/zoom view, and what each step shows and edits on the
//! drawing: the found character (Import), draggable joints (Rig), the posed
//! character (Animate), and the exported area (Export).

use crate::state::{AppState, SkelSel, Step, part_color};
use anim_core::skeleton::dist_to_segment;
use anim_core::{Affine2, RectI, aim_part, ik_parent, posed_joint, solve_two_bone, world_transforms};
use eframe::egui::{self, Color32, Key, PointerButton, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};

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
    /// Moving a joint. `new_bone` is set when the joint is the tip of a bone
    /// created by this drag (Shift+drag), which is dropped if left too short.
    Joint { index: usize, new_bone: bool },
    /// Animate step: rotating a part by its tip.
    PoseRotate { part: usize },
    /// Animate step: moving the whole figure by a root's pivot.
    PoseMove,
}

/// A bone shorter than this (image px) after Shift+drag is discarded.
const MIN_BONE_PX: f32 = 3.0;

pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let canvas = response.rect;
    painter.rect_filled(canvas, 0.0, Color32::from_gray(40));

    let Some(image_size) = state.doc.as_ref().map(|d| d.size()) else {
        painter.text(
            canvas.center(),
            egui::Align2::CENTER_CENTER,
            "Drop a drawing here, or click Open…",
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
    if state.step == Step::Rig && (key(Key::Delete) || key(Key::Backspace)) {
        state.delete_skel_selection();
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
    if state.step.shows_pose() {
        state.refresh_sprites(ui.ctx());
    }

    let view = state.view;
    let doc = state.doc.as_ref().unwrap();
    let image_rect = Rect::from_min_size(view.image_to_screen(canvas, Pos2::ZERO), image_size * view.zoom);
    let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
    let painter = painter.with_clip_rect(canvas);
    match state.step {
        Step::Review | Step::Rig => {
            painter.image(doc.texture.id(), image_rect, uv, Color32::WHITE);
            if let Some(overlay) = &doc.overlay {
                painter.image(overlay.id(), image_rect, uv, Color32::WHITE);
            }
            if state.step == Step::Rig {
                draw_skeleton(&painter, canvas, state, response.hover_pos());
            }
        }
        Step::Animate => {
            if state.show_ghost {
                painter.image(doc.texture.id(), image_rect, uv, Color32::from_white_alpha(50));
            }
            draw_posed(&painter, canvas, state, response.hover_pos(), state.show_bones);
        }
        Step::Export => {
            draw_posed(&painter, canvas, state, None, false);
            // The area each exported frame will show.
            if let Some(area) = crate::export::frame_rect(state) {
                let r = view.rect_to_screen(canvas, &area);
                let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
                let stroke = Stroke::new(1.5, Color32::from_rgb(90, 190, 255));
                painter.add(egui::Shape::dashed_line(&pts, stroke, 6.0, 4.0));
            }
        }
    }
}

/// Posed joint positions (image coordinates), one per joint.
fn posed_joints(state: &AppState) -> Vec<[f32; 2]> {
    let Some(doc) = &state.doc else { return Vec::new() };
    let t = world_transforms(&doc.skeleton, &doc.pose);
    (0..doc.skeleton.joints.len()).map(|j| posed_joint(&doc.skeleton, &t, j)).collect()
}

/// Index of the point (image coordinates) nearest to screen position `p`, within a few points.
fn hit_point(state: &AppState, canvas: Rect, points: &[[f32; 2]], p: Pos2) -> Option<usize> {
    points
        .iter()
        .enumerate()
        .map(|(i, q)| (i, state.view.image_to_screen(canvas, pos2(q[0], q[1])).distance(p)))
        .filter(|&(_, d)| d <= JOINT_HIT_PX)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// The posed character: every part's sprite moved by its part's transform,
/// parents first so children sit on top, then (with `bones`) the skeleton.
fn draw_posed(painter: &egui::Painter, canvas: Rect, state: &AppState, hover: Option<Pos2>, bones: bool) {
    let view = state.view;
    let doc = state.doc.as_ref().unwrap();
    let skel = &doc.skeleton;
    let t = world_transforms(skel, &doc.pose);
    let to_screen = |p: [f32; 2]| view.image_to_screen(canvas, pos2(p[0], p[1]));

    for p in skel.draw_order() {
        let (Some(sprite), Some(Some(tex))) = (doc.sprites.get(p), doc.sprite_tex.get(p)) else { continue };
        let [ox, oy] = [sprite.origin[0] as f32, sprite.origin[1] as f32];
        let [w, h] = [sprite.image.width() as f32, sprite.image.height() as f32];
        let corners = [[ox, oy], [ox + w, oy], [ox + w, oy + h], [ox, oy + h]];
        let uvs = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
        let mut mesh = egui::Mesh::with_texture(tex.id());
        for (c, uv) in corners.iter().zip(uvs) {
            mesh.vertices.push(egui::epaint::Vertex { pos: to_screen(t[p].apply(*c)), uv, color: Color32::WHITE });
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(egui::Shape::mesh(mesh));
    }

    // "Show bones" off (B): just the animation. Dragging still works; the dots are only hidden.
    if !bones {
        return;
    }

    // While rotating a part, show the range its limit allows (and its upper
    // part's, when IK is moving both).
    if let Drag::PoseRotate { part } = state.drag {
        draw_limit_arc(painter, canvas, state, &t, part);
        if state.ik
            && let Some(upper) = ik_parent(skel, part)
        {
            draw_limit_arc(painter, canvas, state, &t, upper);
        }
    }

    let joints = posed_joints(state);
    for (i, part) in skel.parts.iter().enumerate() {
        let (a, b) = (to_screen(joints[part.pivot]), to_screen(joints[part.tip]));
        painter.line_segment([a, b], Stroke::new(3.0, Color32::from_black_alpha(140)));
        painter.line_segment([a, b], Stroke::new(1.5, part_color(i)));
    }
    let active = match state.drag {
        Drag::PoseRotate { part } => Some(skel.parts[part].tip),
        _ => hover.and_then(|h| hit_point(state, canvas, &joints, h)),
    };
    for (j, &pos) in joints.iter().enumerate() {
        let r = if active == Some(j) { 7.0 } else { 5.0 };
        let fill = if pose_drag_for_joint(skel, j).is_some() { Color32::WHITE } else { Color32::GRAY };
        painter.circle(to_screen(pos), r, fill, Stroke::new(1.5, Color32::BLACK));
    }
    if let Some(j) = active {
        label(painter, to_screen(joints[j]) + vec2(10.0, -10.0), &skel.joints[j].name);
    }
}

/// A wedge at a part's pivot showing the directions its limit allows the bone
/// to point, measured from where the parent currently holds it.
fn draw_limit_arc(painter: &egui::Painter, canvas: Rect, state: &AppState, t: &[Affine2], part: usize) {
    let doc = state.doc.as_ref().unwrap();
    let skel = &doc.skeleton;
    let Some([lo, hi]) = skel.parts[part].limit else { return };
    let (pivot, tip) = skel.bone(part);
    // The parent's transform: the part's own transform with its own rotation undone.
    let parent = t[part].mul(&Affine2::rotate_about(pivot, -doc.pose.angle(part)));
    let center = parent.apply(pivot);
    let rest_dir = parent.apply_vec([tip[0] - pivot[0], tip[1] - pivot[1]]);
    let base = rest_dir[1].atan2(rest_dir[0]);
    let c = state.view.image_to_screen(canvas, pos2(center[0], center[1]));
    let r = 45.0;
    let steps = 32;
    let mut points = vec![c];
    for i in 0..=steps {
        let a = base + lo + (hi - lo) * i as f32 / steps as f32;
        points.push(c + vec2(a.cos(), a.sin()) * r);
    }
    points.push(c);
    let fill = Color32::from_rgba_unmultiplied(255, 220, 80, 40);
    let stroke = Stroke::new(1.5, Color32::from_rgba_unmultiplied(255, 220, 80, 200));
    // A wedge wider than 180° isn't convex, so fill it as a triangle fan.
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(c, fill);
    for p in &points[1..points.len() - 1] {
        mesh.colored_vertex(*p, fill);
    }
    for i in 1..steps as u32 + 1 {
        mesh.add_triangle(0, i, i + 1);
    }
    painter.add(egui::Shape::mesh(mesh));
    painter.add(egui::Shape::line(points, stroke));
}

/// What dragging joint `j` does in Pose mode: rotate the part it is the tip
/// of, or move the figure if it is a root's pivot. Grey (undraggable) otherwise.
fn pose_drag_for_joint(skel: &anim_core::Skeleton, j: usize) -> Option<Drag> {
    if let Some(part) = skel.parts.iter().position(|p| p.tip == j) {
        return Some(Drag::PoseRotate { part });
    }
    skel.parts.iter().any(|p| p.pivot == j && p.parent.is_none()).then_some(Drag::PoseMove)
}

fn draw_skeleton(painter: &egui::Painter, canvas: Rect, state: &AppState, hover: Option<Pos2>) {
    let view = state.view;
    let doc = state.doc.as_ref().unwrap();
    let skel = &doc.skeleton;
    let to_screen = |p: [f32; 2]| view.image_to_screen(canvas, pos2(p[0], p[1]));
    for i in 0..skel.parts.len() {
        let (a, b) = skel.bone(i);
        let (a, b) = (to_screen(a), to_screen(b));
        let selected = state.skel_sel == SkelSel::Part(i);
        let (outline, core) = if selected { (Color32::WHITE, 4.0) } else { (Color32::from_black_alpha(160), 2.0) };
        painter.line_segment([a, b], Stroke::new(core + 2.0, outline));
        painter.line_segment([a, b], Stroke::new(core, part_color(i)));
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
            doc.reset_pose();
            Drag::Joint { index: tip, new_bone: true }
        }
        (None, true) => {
            let pivot = doc.skeleton.add_joint(p);
            let tip = doc.skeleton.add_joint(p);
            state.skel_sel = SkelSel::Part(doc.skeleton.add_part(pivot, tip));
            doc.reset_pose();
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
        if pan && space {
            state.space_panned = true;
        }
        state.drag = if pan {
            Drag::Pan
        } else if let (true, Some(o)) = (response.dragged_by(PointerButton::Primary), origin) {
            match state.step {
                Step::Rig => begin_joint_drag(state, canvas, o, shift),
                Step::Animate => {
                    let joints = posed_joints(state);
                    let skel = &state.doc.as_ref().unwrap().skeleton;
                    let drag = hit_point(state, canvas, &joints, o).and_then(|j| pose_drag_for_joint(skel, j));
                    if drag.is_some() && state.playing {
                        state.pause();
                    }
                    drag.unwrap_or(Drag::None)
                }
                // Nothing to edit on the drawing in these steps: a plain drag pans.
                Step::Review | Step::Export => Drag::Pan,
            }
        } else {
            Drag::None
        };
    }

    if response.dragged() {
        match &mut state.drag {
            Drag::Pan => state.view.offset += response.drag_delta(),
            &mut Drag::Joint { index, .. } => {
                if let (Some(p), Some(doc)) = (pointer_img, &mut state.doc) {
                    let (w, h) = doc.image.dimensions();
                    doc.skeleton.joints[index].pos = [p.x.clamp(0.0, w as f32), p.y.clamp(0.0, h as f32)];
                    doc.recompute_skeleton();
                }
            }
            &mut Drag::PoseRotate { part } => {
                if let (Some(p), Some(doc)) = (pointer_img, &mut state.doc) {
                    let target = [p.x, p.y];
                    if state.ik {
                        solve_two_bone(&doc.skeleton, &mut doc.pose, part, target);
                    } else {
                        aim_part(&doc.skeleton, &mut doc.pose, part, target);
                    }
                }
            }
            Drag::PoseMove => {
                if let Some(doc) = &mut state.doc {
                    let d = response.drag_delta() / view.zoom;
                    doc.pose.offset[0] += d.x;
                    doc.pose.offset[1] += d.y;
                }
            }
            Drag::None => {}
        }
    }

    if response.drag_stopped() {
        let before = state.drag_before.take();
        key_after_drag(state, before.as_ref());
        if is_stub_bone(state)
            && let Some(before) = before
        {
            // A Shift+click without real movement: undo the bone silently.
            state.restore(before);
        } else {
            state.commit(before);
        }
        state.drag = Drag::None;
    }

    if state.step == Step::Rig
        && response.clicked()
        && let Some(screen) = response.interact_pointer_pos()
    {
        state.skel_sel = match (hit_joint(state, canvas, screen), hit_part(state, canvas, screen)) {
            (Some(j), _) => SkelSel::Joint(j),
            (None, Some(p)) => SkelSel::Part(p),
            (None, None) => SkelSel::None,
        };
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

/// After a pose drag: with auto-key on, save the parts it moved as keys at the
/// playhead; with it off, just remember that the shown pose isn't keyed yet
/// (K saves it; scrubbing or playing discards it).
pub(crate) fn key_after_drag(state: &mut AppState, before: Option<&crate::state::Snapshot>) {
    if !matches!(state.drag, Drag::PoseRotate { .. } | Drag::PoseMove) {
        return;
    }
    let (Some(doc), Some(before)) = (&mut state.doc, before) else { return };
    if doc.pose == before.pose {
        return;
    }
    if state.auto_key {
        doc.motion.set_keys(state.playhead, &doc.skeleton, &doc.pose, Some(&before.pose));
    } else {
        state.unkeyed_pose = true;
    }
}
