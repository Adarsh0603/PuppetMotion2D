//! The timeline strip shown under the canvas in Pose mode: playback controls,
//! then one row of keys per animated part (plus an "All keys" summary row and
//! a "Position" row for whole-figure moves). Click a key to select it,
//! Shift+click to add to the selection, drag keys to move them in time,
//! right-click for their ease, click empty space to move the playhead.

use crate::state::{AppState, part_color};
use anim_core::motion::{KEY_EPSILON, MAX_FPS};
use anim_core::{Ease, KeyRef, Motion, TrackId};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, pos2, vec2};

const LABEL_WIDTH: f32 = 130.0;
const SUMMARY_HEIGHT: f32 = 24.0;
const ROW_HEIGHT: f32 = 20.0;
/// How close (points) a click must be to a key's diamond to pick it.
const KEY_HIT: f32 = 6.0;

/// One row of the timeline.
enum Lane {
    /// Every key at each time, all tracks together.
    All,
    /// Whole-figure movement (offset x and y keys together).
    Position,
    /// One part's angle keys: part number (for its color) and name.
    Part(usize, String),
}

pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    let Some(doc) = &state.doc else { return };
    let duration = doc.motion.duration;
    let selected = state.key_sel.len();

    ui.horizontal(|ui| {
        let play = if state.playing { "⏸ Pause" } else { "▶ Play" };
        if ui.button(play).on_hover_text("Space").clicked() {
            state.toggle_play();
        }
        if ui.button("⏮").on_hover_text("Previous key").clicked() {
            state.jump_key(-1);
        }
        if ui.button("⏭").on_hover_text("Next key").clicked() {
            state.jump_key(1);
        }
        if ui.button("◀").on_hover_text("Previous frame ( , )").clicked() {
            state.step_frame(-1);
        }
        if ui.button("▶").on_hover_text("Next frame ( . )").clicked() {
            state.step_frame(1);
        }
        let m = &state.doc.as_ref().unwrap().motion;
        let (frame, frames) = (m.frame_at(state.playhead), m.frame_at(duration));
        ui.monospace(format!("frame {frame:>3} / {frames}  {:5.2} s", state.playhead));
        ui.separator();
        // Record button: when on, every pose drag is saved as keys right away.
        let rec_color = if state.auto_key { Color32::from_rgb(255, 80, 80) } else { Color32::GRAY };
        let rec = egui::Button::new(egui::RichText::new("⏺ Auto-key").color(rec_color)).selected(state.auto_key);
        if ui
            .add(rec)
            .on_hover_text("On: posing saves keys at the playhead.\nOff: posing is a preview until you press K.")
            .clicked()
        {
            state.auto_key = !state.auto_key;
        }
        if ui.button("Key all").on_hover_text("Pin every part's current angle at this time").clicked() {
            state.key_all();
        }
        ui.separator();
        if ui.add_enabled(selected > 0, egui::Button::new("Copy")).on_hover_text("Ctrl+C").clicked() {
            state.copy_keys(ui.ctx());
        }
        if ui.add_enabled(selected > 0, egui::Button::new("Cut")).on_hover_text("Ctrl+X").clicked() {
            state.cut_keys(ui.ctx());
        }
        let can_paste = state.clip.as_ref().is_some_and(|c| !c.is_empty());
        if ui.add_enabled(can_paste, egui::Button::new("Paste")).on_hover_text("Ctrl+V: at the playhead").clicked() {
            state.paste_keys(None);
        }
        if ui.add_enabled(selected > 0, egui::Button::new("Delete")).on_hover_text("Delete: remove the selected keys\nCtrl+A: select every key").clicked() {
            state.delete_selected_keys();
        }
        if selected > 0 {
            ui.label(format!("{selected} selected · ease"));
            ease_picker(ui, state);
        }
        // Only there when a pose was changed with auto-key off and isn't saved yet.
        if state.unkeyed_pose {
            let keep = egui::Button::new(egui::RichText::new("● Keep pose (K)").color(Color32::BLACK).strong())
                .fill(Color32::from_rgb(255, 170, 60));
            if ui.add(keep).on_hover_text("Save the parts you changed as keys at this frame.\nMove the playhead or play to discard them.").clicked() {
                state.key_pose();
            }
        }
        ui.separator();
        ui.label("Length");
        length_editor(ui, state);
        fps_editor(ui, state);
        ui.checkbox(&mut state.looping, "Loop");
    });

    let lanes = lanes(state);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        for (i, lane) in lanes.iter().enumerate() {
            lane_row(ui, state, lane, i);
        }
    });
}

/// The rows to show: summary, position (if it has keys), then every part
/// that has keys, in skeleton order.
fn lanes(state: &AppState) -> Vec<Lane> {
    let doc = state.doc.as_ref().unwrap();
    let m = &doc.motion;
    let mut out = vec![Lane::All];
    if !m.offset_x.is_empty() || !m.offset_y.is_empty() {
        out.push(Lane::Position);
    }
    for (i, part) in doc.skeleton.parts.iter().enumerate() {
        if m.tracks.get(&part.name).is_some_and(|k| !k.is_empty()) {
            out.push(Lane::Part(i, part.name.clone()));
        }
    }
    out
}

/// Key times shown on a lane, each with the keys a click there selects.
fn lane_keys(m: &Motion, lane: &Lane) -> Vec<(f32, Vec<KeyRef>)> {
    let refs_at = |ids: &[TrackId], t: f32| -> Vec<KeyRef> {
        ids.iter()
            .filter(|id| m.track(id).is_some_and(|k| k.iter().any(|k| (k.t - t).abs() < KEY_EPSILON)))
            .map(|id| KeyRef { track: id.clone(), t })
            .collect()
    };
    let times = |ids: &[TrackId]| {
        let mut ts: Vec<f32> = ids.iter().filter_map(|id| m.track(id)).flatten().map(|k| k.t).collect();
        ts.sort_by(f32::total_cmp);
        ts.dedup_by(|a, b| (*a - *b).abs() < KEY_EPSILON);
        ts
    };
    match lane {
        Lane::All => m.key_times().into_iter().map(|t| (t, m.keys_at(t))).collect(),
        Lane::Position => {
            let ids = [TrackId::OffsetX, TrackId::OffsetY];
            times(&ids).into_iter().map(|t| (t, refs_at(&ids, t))).collect()
        }
        Lane::Part(_, name) => {
            let ids = [TrackId::Part(name.clone())];
            times(&ids).into_iter().map(|t| (t, refs_at(&ids, t))).collect()
        }
    }
}

fn is_selected(sel: &[KeyRef], r: &KeyRef) -> bool {
    sel.iter().any(|s| s.track == r.track && (s.t - r.t).abs() < KEY_EPSILON)
}

fn lane_row(ui: &mut egui::Ui, state: &mut AppState, lane: &Lane, row: usize) {
    let doc = state.doc.as_ref().unwrap();
    let duration = doc.motion.duration.max(0.01);
    // Each key time with the keys a click there picks, and their shared ease (None if mixed).
    let keys: Vec<(f32, Vec<KeyRef>, Option<Ease>)> = lane_keys(&doc.motion, lane)
        .into_iter()
        .map(|(t, refs)| {
            let mut eases = refs.iter().filter_map(|r| doc.motion.key(r)).map(|k| k.ease);
            let first = eases.next();
            let ease = first.filter(|&f| eases.all(|e| e == f));
            (t, refs, ease)
        })
        .collect();
    let height = if matches!(lane, Lane::All) { SUMMARY_HEIGHT } else { ROW_HEIGHT };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    let label_rect = Rect::from_min_size(rect.min, vec2(LABEL_WIDTH, height));
    let bar = Rect::from_min_max(pos2(rect.left() + LABEL_WIDTH, rect.top()), rect.max);
    let inner = bar.shrink2(vec2(8.0, 0.0));
    let x_of = |t: f32| inner.left() + inner.width() * (t / duration).clamp(0.0, 1.0);
    let t_of = |x: f32| ((x - inner.left()) / inner.width()).clamp(0.0, 1.0) * duration;
    let (fps, length) = (doc.motion.fps.max(1) as f32, doc.motion.duration);
    let snap = |t: f32| ((t * fps).round() / fps).min(length);

    // Label and background.
    let (name, color) = match lane {
        Lane::All => ("All keys".to_string(), Color32::WHITE),
        Lane::Position => ("Position".to_string(), Color32::LIGHT_GRAY),
        Lane::Part(i, name) => (name.clone(), part_color(*i)),
    };
    painter.rect_filled(label_rect, 0.0, Color32::from_gray(30));
    painter.text(
        label_rect.left_center() + vec2(6.0, 0.0),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::proportional(13.0),
        color,
    );
    let shade = if row.is_multiple_of(2) { 38 } else { 33 };
    painter.rect_filled(bar, 0.0, Color32::from_gray(shade));
    // Frame ticks, when they're far enough apart to read.
    let frames = doc.motion.frame_at(duration);
    if frames > 0 && inner.width() / frames as f32 >= 6.0 {
        for f in 0..=frames {
            let x = x_of(f as f32 * doc.motion.frame_len());
            painter.line_segment([pos2(x, bar.bottom() - 4.0), pos2(x, bar.bottom())], Stroke::new(1.0, Color32::from_gray(60)));
        }
    }
    for s in 0..=duration.floor() as i32 {
        let x = x_of(s as f32);
        painter.line_segment([pos2(x, bar.top()), pos2(x, bar.bottom())], Stroke::new(1.0, Color32::from_gray(55)));
    }

    // Keys.
    for (t, refs, ease) in &keys {
        let selected = refs.iter().any(|r| is_selected(&state.key_sel, r));
        let at_playhead = (t - state.playhead).abs() < KEY_EPSILON;
        let fill = if selected {
            Color32::from_rgb(255, 235, 120)
        } else if at_playhead {
            Color32::from_rgb(255, 170, 60)
        } else {
            Color32::from_rgb(170, 140, 70)
        };
        let outline = if selected { Color32::WHITE } else { Color32::BLACK };
        key_mark(&painter, pos2(x_of(*t), bar.center().y), if selected { 6.0 } else { 5.0 }, *ease, fill, outline);
    }
    // Playhead.
    let x = x_of(state.playhead);
    painter.line_segment([pos2(x, bar.top()), pos2(x, bar.bottom())], Stroke::new(2.0, Color32::from_rgb(90, 190, 255)));

    // Pointer: click a key to select it (Shift adds), drag keys to retime
    // them, right-click a key to pick its ease; click or drag empty space to
    // move the playhead.
    let shift = ui.input(|i| i.modifiers.shift);
    let hit_at = |p: Pos2| -> Option<(f32, Vec<KeyRef>)> {
        if !bar.contains(p) {
            return None;
        }
        keys.iter()
            .map(|(t, refs, _)| ((x_of(*t) - p.x).abs(), *t, refs))
            .filter(|(d, ..)| *d <= KEY_HIT)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, t, refs)| (t, refs.clone()))
    };
    if response.clicked()
        && let Some(p) = response.interact_pointer_pos()
        && bar.contains(p)
    {
        match hit_at(p) {
            Some((t, refs)) => {
                if shift {
                    toggle_keys(state, &refs);
                } else {
                    state.key_sel = refs;
                }
                state.playhead = t;
            }
            None => {
                if !shift {
                    state.key_sel.clear();
                }
                state.playhead = snap(t_of(p.x));
            }
        }
        state.playing = false;
    }

    if response.drag_started() {
        let origin = ui.input(|i| i.pointer.press_origin());
        if let Some((t, refs)) = origin.and_then(hit_at) {
            pick_keys(state, &refs, shift);
            state.begin_key_drag(t);
        }
    }
    if response.dragged()
        && let Some(p) = response.interact_pointer_pos()
    {
        if state.key_drag.is_some() {
            state.update_key_drag(t_of(p.x));
        } else {
            // Dragging empty space scrubs, frame by frame.
            state.playhead = snap(t_of(p.x));
            state.playing = false;
        }
    }
    if response.drag_stopped() {
        state.end_key_drag();
    }

    if response.secondary_clicked()
        && let Some((_, refs)) = response.interact_pointer_pos().and_then(hit_at)
    {
        pick_keys(state, &refs, shift);
    }
    if !state.key_sel.is_empty() {
        response.context_menu(|ui| ease_menu(ui, state));
    }
}

/// Shift+click on a key: add it to the selection, or take it out if it's already in.
fn toggle_keys(state: &mut AppState, refs: &[KeyRef]) {
    if refs.iter().all(|r| is_selected(&state.key_sel, r)) {
        state.key_sel.retain(|s| !is_selected(refs, s));
    } else {
        let new: Vec<KeyRef> = refs.iter().filter(|r| !is_selected(&state.key_sel, r)).cloned().collect();
        state.key_sel.extend(new);
    }
}

/// Make sure the keys under the pointer are selected before dragging them or
/// opening their menu: an unselected key replaces the selection (Shift adds
/// it); a selected one keeps the whole selection, so several move together.
fn pick_keys(state: &mut AppState, refs: &[KeyRef], shift: bool) {
    if refs.iter().all(|r| is_selected(&state.key_sel, r)) {
        return;
    }
    if shift {
        toggle_keys(state, refs);
    } else {
        state.key_sel = refs.to_vec();
    }
}

/// The eases a key can have, with what each one does.
const EASES: [(Ease, &str, &str); 3] = [
    (Ease::Smooth, "◆ Smooth", "Starts slow, speeds up, ends slow"),
    (Ease::Linear, "● Linear", "Constant speed"),
    (Ease::Hold, "■ Hold", "Stays on this pose, then jumps to the next key"),
];

/// Right-click menu on keys: how the selected keys travel to the next key.
fn ease_menu(ui: &mut egui::Ui, state: &mut AppState) {
    ui.label(format!("{} selected: move to the next key with", state.key_sel.len()));
    let current = state.selected_ease();
    for (ease, name, hint) in EASES {
        if ui.radio(current == Some(ease), name).on_hover_text(hint).clicked() {
            state.set_selected_ease(ease);
            ui.close();
        }
    }
}

/// Ease picker in the timeline bar, for the selected keys.
fn ease_picker(ui: &mut egui::Ui, state: &mut AppState) {
    let current = state.selected_ease();
    let text = current.and_then(|c| EASES.iter().find(|e| e.0 == c)).map_or("Mixed", |e| e.1);
    egui::ComboBox::from_id_salt("ease")
        .selected_text(text)
        .show_ui(ui, |ui| {
            for (ease, name, hint) in EASES {
                if ui.selectable_label(current == Some(ease), name).on_hover_text(hint).clicked() {
                    state.set_selected_ease(ease);
                }
            }
        })
        .response
        .on_hover_text("How the selected keys move to the next key.\nRight-click a key for the same choice.");
}

/// Motion length in seconds, moving a frame at a time. A whole drag of the
/// value is one undo step.
fn length_editor(ui: &mut egui::Ui, state: &mut AppState) {
    let before = state.snapshot();
    let Some(doc) = &mut state.doc else { return };
    let m = &mut doc.motion;
    let last_key = m.key_times().last().copied().unwrap_or(0.0);
    let frame = m.frame_len();
    let min = last_key.max(frame);
    let r = ui.add(egui::DragValue::new(&mut m.duration).range(min..=600.0).speed(frame).suffix(" s"));
    if r.changed() {
        m.duration = m.snap(m.duration).max(min);
    }
    if (r.drag_started() || r.gained_focus()) && state.field_before.is_none() {
        state.field_before = before;
    }
    if r.drag_stopped() || r.lost_focus() {
        let before = state.field_before.take();
        state.commit(before);
    }
    state.playhead = state.playhead.min(state.doc.as_ref().unwrap().motion.duration);
}

/// Frames per second of the timeline grid (saved with the project). Keys stay
/// where they are in seconds; only the playhead is put back on a frame.
fn fps_editor(ui: &mut egui::Ui, state: &mut AppState) {
    let before = state.snapshot();
    let Some(doc) = &mut state.doc else { return };
    let r = ui
        .add(egui::DragValue::new(&mut doc.motion.fps).range(1..=MAX_FPS).speed(0.2).suffix(" fps"))
        .on_hover_text("Frame rate: the timeline snaps to these frames, and export uses it by default");
    if (r.drag_started() || r.gained_focus()) && state.field_before.is_none() {
        state.field_before = before;
    }
    if r.changed() {
        state.playhead = state.key_time();
    }
    if r.drag_stopped() || r.lost_focus() {
        let before = state.field_before.take();
        state.commit(before);
    }
}

/// A key on the timeline; its shape shows its ease: ◆ Smooth (or mixed), ● Linear, ■ Hold.
fn key_mark(painter: &egui::Painter, c: Pos2, r: f32, ease: Option<Ease>, fill: Color32, outline: Color32) {
    let stroke = Stroke::new(1.0, outline);
    match ease {
        Some(Ease::Linear) => {
            painter.circle(c, r * 0.85, fill, stroke);
        }
        Some(Ease::Hold) => {
            let s = r * 0.8;
            let pts = vec![c + vec2(-s, -s), c + vec2(s, -s), c + vec2(s, s), c + vec2(-s, s)];
            painter.add(egui::Shape::convex_polygon(pts, fill, stroke));
        }
        _ => {
            let pts = vec![c + vec2(0.0, -r), c + vec2(r, 0.0), c + vec2(0.0, r), c + vec2(-r, 0.0)];
            painter.add(egui::Shape::convex_polygon(pts, fill, stroke));
        }
    }
}
