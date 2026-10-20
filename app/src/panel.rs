//! Layout helpers for the side panel: titled cards that group related
//! controls, foldable cards for things used less often, and the shortcut list.

use eframe::egui::{self, RichText};

/// Small uppercase heading above a card.
fn heading(ui: &egui::Ui, title: &str) -> RichText {
    RichText::new(title.to_uppercase()).size(11.0).strong().color(ui.visuals().weak_text_color())
}

/// The softly shaded box a card's controls sit in.
fn body<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(6)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            add(ui)
        })
        .inner
}

/// A titled group of controls.
pub fn card<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.add_space(10.0);
    ui.label(heading(ui, title));
    ui.add_space(2.0);
    body(ui, add)
}

/// A card that starts folded, for settings most people never need.
pub fn fold(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(10.0);
    let title = heading(ui, title);
    collapsing(ui, title, |ui| {
        body(ui, add);
    });
}

/// A section that opens on click and starts closed. Its contents sit straight
/// under the heading: no indent and no vertical guide line.
pub fn collapsing(ui: &mut egui::Ui, title: impl Into<egui::WidgetText>, add: impl FnOnce(&mut egui::Ui)) {
    let title = title.into();
    let id = title.text().to_string();
    // Only the body is unindented; the header keeps its normal arrow.
    egui::CollapsingHeader::new(title).id_salt(id).default_open(false).show_unindented(ui, add);
}

/// The step's keyboard and mouse shortcuts, folded by default: when opened, a
/// two-column list of the keys (styled like key caps) and what they do.
pub fn shortcuts(ui: &mut egui::Ui, list: &[(&str, &str)]) {
    ui.add_space(4.0);
    let title = heading(ui, "Shortcuts");
    collapsing(ui, title, |ui| {
        body(ui, |ui| {
            let cap = ui.visuals().code_bg_color;
            egui::Grid::new("shortcuts").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
                for &(keys, what) in list {
                    ui.label(RichText::new(format!(" {keys} ")).monospace().size(11.5).background_color(cap).strong());
                    ui.label(RichText::new(what).size(11.5));
                    ui.end_row();
                }
            });
        });
    });
    ui.add_space(4.0);
}

/// A one-line explanation under the step title.
pub fn intro(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).weak());
}
