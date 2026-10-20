// Release builds run as a normal windowed app (no console window).
// Debug builds keep the console so errors and panics stay visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod canvas;
mod export;
mod state;
mod timeline;

use eframe::egui;
use state::AppState;
use std::path::PathBuf;

fn main() -> eframe::Result {
    let initial = std::env::args_os().nth(1).map(PathBuf::from);
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).expect("built-in icon");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PuppetMotion 2D")
            .with_icon(icon)
            .with_inner_size([1280.0, 800.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "PuppetMotion 2D",
        options,
        Box::new(move |cc| {
            let mut app = App { state: AppState::default(), title: String::new() };
            if let Some(path) = initial {
                app.state.open(&cc.egui_ctx, &path);
            }
            Ok(Box::new(app))
        }),
    )
}

struct App {
    state: AppState,
    /// Window title last sent, so it's only updated when it changes.
    title: String,
}

impl App {
    /// Global keys: undo/redo, save, open. Skipped while a text box has focus
    /// (it has its own undo) and mid-drag (the drag's undo step isn't recorded yet).
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.text_edit_focused() {
            return;
        }
        if self.state.field_before.is_some() && !ctx.egui_is_using_pointer() {
            // A field edit (name, limit) ended without a lost-focus or
            // drag-stopped event; record it now.
            let before = self.state.field_before.take();
            self.state.commit(before);
        }
        if !matches!(self.state.drag, canvas::Drag::None) || self.state.key_drag.is_some() {
            return;
        }
        use egui::{Key, KeyboardShortcut, Modifiers};
        let pressed = |ctx: &egui::Context, mods: Modifiers, key: Key| {
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(mods, key)))
        };
        // Check Shift versions first: plain Ctrl+Z also matches Ctrl+Shift+Z.
        let redo = pressed(ctx, Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) || pressed(ctx, Modifiers::COMMAND, Key::Y);
        if redo {
            self.state.redo();
        } else if pressed(ctx, Modifiers::COMMAND, Key::Z) {
            self.state.undo();
        }
        if pressed(ctx, Modifiers::COMMAND | Modifiers::SHIFT, Key::S) {
            self.state.save_as();
        } else if pressed(ctx, Modifiers::COMMAND, Key::S) {
            self.state.save();
        }
        if pressed(ctx, Modifiers::COMMAND, Key::O) {
            self.state.open_dialog(ctx);
        }
        if self.state.mode == state::Mode::Pose {
            self.key_shortcuts(ctx);
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::B)) {
                self.state.show_bones = !self.state.show_bones;
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::K)) {
                self.state.key_pose();
            }
            if pressed(ctx, Modifiers::COMMAND, Key::A) {
                self.state.select_all_keys();
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Comma)) {
                self.state.step_frame(-1);
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Period)) {
                self.state.step_frame(1);
            }
        }
        // Space plays/pauses in Pose mode, unless it was held to pan (Space+drag).
        if ctx.input(|i| i.key_released(Key::Space)) {
            if self.state.mode == state::Mode::Pose && !self.state.space_panned {
                self.state.toggle_play();
            }
            self.state.space_panned = false;
        }
    }
}

impl App {
    /// Timeline keys in Pose mode: Ctrl+C copies the selected keys, Ctrl+X cuts
    /// them, Ctrl+V pastes at the playhead, Delete removes them. egui turns
    /// Ctrl+C/X/V into Copy, Cut and Paste events (Paste carries the system
    /// clipboard's text).
    fn key_shortcuts(&mut self, ctx: &egui::Context) {
        let (copy, cut, paste, paste_other) = ctx.input(|i| {
            let copy = i.events.iter().any(|e| matches!(e, egui::Event::Copy));
            let cut = i.events.iter().any(|e| matches!(e, egui::Event::Cut));
            let paste = i.events.iter().find_map(|e| match e {
                egui::Event::Paste(text) => Some(text.clone()),
                _ => None,
            });
            let paste_other = i.events.iter().any(|e| matches!(e, egui::Event::PasteImage(_)));
            (copy, cut, paste, paste_other)
        });
        if copy {
            self.state.copy_keys(ctx);
        }
        if cut {
            self.state.cut_keys(ctx);
        }
        if let Some(text) = paste {
            self.state.paste_keys(Some(&text));
        } else if paste_other {
            self.state.paste_keys(None);
        }
        let delete = ctx.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace));
        if delete && !self.state.key_sel.is_empty() {
            self.state.delete_selected_keys();
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
        self.state.tick(&ctx);

        egui::Panel::left("side").resizable(true).default_size(260.0).show(ui, |ui| {
            self.state.side_panel(ui);
        });
        if self.state.mode == state::Mode::Pose && self.state.doc.is_some() {
            egui::Panel::bottom("timeline").resizable(true).default_size(160.0).min_size(70.0).max_size(600.0).show(ui, |ui| {
                timeline::show(ui, &mut self.state);
            });
        }
        egui::CentralPanel::default().show(ui, |ui| {
            canvas::show(ui, &mut self.state);
        });
        export::show(&ctx, &mut self.state);

        let title = self.state.title();
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}
