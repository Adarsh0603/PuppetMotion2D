mod canvas;
mod state;

use eframe::egui;
use state::AppState;
use std::path::PathBuf;

fn main() -> eframe::Result {
    let initial = std::env::args_os().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PuppetMotion 2D")
            .with_inner_size([1280.0, 800.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "PuppetMotion 2D",
        options,
        Box::new(move |cc| {
            let mut app = App { state: AppState::default() };
            if let Some(path) = initial {
                app.state.open(&cc.egui_ctx, &path);
            }
            Ok(Box::new(app))
        }),
    )
}

struct App {
    state: AppState,
}

impl App {
    /// Global undo/redo keys. Skipped while a text box has focus (it has its
    /// own undo) and mid-drag (the drag's undo step isn't recorded yet).
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.text_edit_focused() {
            return;
        }
        if self.state.rename_before.is_some() {
            // A name edit just ended without a lost-focus event; record it.
            let before = self.state.rename_before.take();
            self.state.commit(before);
        }
        if !matches!(self.state.drag, canvas::Drag::None) {
            return;
        }
        use egui::{Key, KeyboardShortcut, Modifiers};
        // Check the Shift version first: plain Ctrl+Z also matches Ctrl+Shift+Z.
        let (redo, undo) = ctx.input_mut(|i| {
            let redo = i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z))
                || i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Y));
            let undo = i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Z));
            (redo, undo)
        });
        if redo {
            self.state.redo();
        } else if undo {
            self.state.undo();
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let dropped = ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()));
        if let Some(path) = dropped {
            self.state.open(&ctx, &path);
        }
        self.shortcuts(&ctx);

        egui::Panel::left("side").resizable(true).default_size(260.0).show(ui, |ui| {
            self.state.side_panel(ui);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            canvas::show(ui, &mut self.state);
        });
    }
}
