//! The timeline strip shown under the canvas in Pose mode: playback controls,
//! then one row of keys per animated part (plus an "All keys" summary row and
//! a "Position" row for whole-figure moves). Click a key to select it,
//! Shift+click to add to the selection, click empty space to move the playhead.

use crate::state::{AppState, box_color};
use anim_core::motion::KEY_EPSILON;
use anim_core::{KeyRef, Motion, TrackId};
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
    let on_key = !doc.motion.keys_at(state.playhead).is_empty();
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
        ui.monospace(format!("{:5.2} s / {:.2} s", state.playhead, duration));
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
        if ui.button("Key pose").on_hover_text("K: save the current pose at the playhead").clicked() {
            state.key_pose();
        }
        if ui.button("Key all").on_hover_text("Pin every part's current angle at this time").clicked() {
            state.key_all();
        }
        if ui.add_enabled(on_key, egui::Button::new("Delete keys here")).clicked() {
            state.delete_keys_here();
        }
        ui.separator();
        if ui.add_enabled(selected > 0, egui::Button::new("Copy")).on_hover_text("Ctrl+C").clicked() {
            state.copy_keys(ui.ctx());
        }
        let can_paste = state.clip.as_ref().is_some_and(|c| !c.is_empty());
        if ui.add_enabled(can_paste, egui::Button::new("Paste")).on_hover_text("Ctrl+V: at the playhead").clicked() {
            state.paste_keys(None);
        }
        if ui.add_enabled(selected > 0, egui::Button::new("Delete")).on_hover_text("Delete key").clicked() {
            state.delete_selected_keys();
        }
        if selected > 0 {
            ui.label(format!("{selected} selected"));
        }
        if state.unkeyed_pose {
            ui.label(egui::RichText::new("● unkeyed pose: K to keep it").color(Color32::from_rgb(255, 170, 60)));
        }
        ui.separator();
        ui.label("Length");
        length_editor(ui, state);
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
    let keys = lane_keys(&doc.motion, lane);
    let height = if matches!(lane, Lane::All) { SUMMARY_HEIGHT } else { ROW_HEIGHT };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    let label_rect = Rect::from_min_size(rect.min, vec2(LABEL_WIDTH, height));
    let bar = Rect::from_min_max(pos2(rect.left() + LABEL_WIDTH, rect.top()), rect.max);
    let inner = bar.shrink2(vec2(8.0, 0.0));
    let x_of = |t: f32| inner.left() + inner.width() * (t / duration).clamp(0.0, 1.0);
    let t_of = |x: f32| ((x - inner.left()) / inner.width()).clamp(0.0, 1.0) * duration;

    // Label and background.
    let (name, color) = match lane {
        Lane::All => ("All keys".to_string(), Color32::WHITE),
        Lane::Position => ("Position".to_string(), Color32::LIGHT_GRAY),
        Lane::Part(i, name) => (name.clone(), box_color(*i)),
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
    for s in 0..=duration.floor() as i32 {
        let x = x_of(s as f32);
        painter.line_segment([pos2(x, bar.top()), pos2(x, bar.bottom())], Stroke::new(1.0, Color32::from_gray(55)));
    }

    // Keys.
    for (t, refs) in &keys {
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
        diamond(&painter, pos2(x_of(*t), bar.center().y), if selected { 6.0 } else { 5.0 }, fill, outline);
    }
    // Playhead.
    let x = x_of(state.playhead);
    painter.line_segment([pos2(x, bar.top()), pos2(x, bar.bottom())], Stroke::new(2.0, Color32::from_rgb(90, 190, 255)));

    // Clicks: on a key → select it (Shift adds); elsewhere → move the playhead.
    let shift = ui.input(|i| i.modifiers.shift);
    if response.clicked()
        && let Some(p) = response.interact_pointer_pos()
        && bar.contains(p)
    {
        let hit = keys
            .iter()
            .map(|(t, refs)| ((x_of(*t) - p.x).abs(), *t, refs))
            .filter(|(d, ..)| *d <= KEY_HIT)
            .min_by(|a, b| a.0.total_cmp(&b.0));
        match hit {
            Some((_, t, refs)) => {
                if shift {
                    let all_on = refs.iter().all(|r| is_selected(&state.key_sel, r));
                    if all_on {
                        state.key_sel.retain(|s| !refs.iter().any(|r| r.track == s.track && (r.t - s.t).abs() < KEY_EPSILON));
                    } else {
                        let new: Vec<KeyRef> =
                            refs.iter().filter(|r| !is_selected(&state.key_sel, r)).cloned().collect();
                        state.key_sel.extend(new);
                    }
                } else {
                    state.key_sel = refs.clone();
                }
                state.playhead = t;
            }
            None => {
                if !shift {
                    state.key_sel.clear();
                }
                state.playhead = t_of(p.x);
            }
        }
        state.playing = false;
    } else if response.dragged()
        && let Some(p) = response.interact_pointer_pos()
    {
        // Dragging anywhere on a row scrubs.
        state.playhead = t_of(p.x);
        state.playing = false;
    }
}

/// Motion length in seconds. A whole drag of the value is one undo step.
fn length_editor(ui: &mut egui::Ui, state: &mut AppState) {
    let before = state.snapshot();
    let Some(doc) = &mut state.doc else { return };
    let last_key = doc.motion.key_times().last().copied().unwrap_or(0.0);
    let r = ui.add(
        egui::DragValue::new(&mut doc.motion.duration)
            .range(last_key.max(0.1)..=600.0)
            .speed(0.05)
            .suffix(" s"),
    );
    if (r.drag_started() || r.gained_focus()) && state.field_before.is_none() {
        state.field_before = before;
    }
    if r.drag_stopped() || r.lost_focus() {
        let before = state.field_before.take();
        state.commit(before);
    }
    state.playhead = state.playhead.min(state.doc.as_ref().unwrap().motion.duration);
}

fn diamond(painter: &egui::Painter, c: Pos2, r: f32, fill: Color32, outline: Color32) {
    let pts = vec![c + vec2(0.0, -r), c + vec2(r, 0.0), c + vec2(0.0, r), c + vec2(-r, 0.0)];
    painter.add(egui::Shape::convex_polygon(pts, fill, Stroke::new(1.0, outline)));
}
