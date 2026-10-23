// Release builds run as a normal windowed app (no console window).
// Debug builds keep the console so errors and panics stay visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod canvas;
mod export;
mod motions;
mod panel;
mod paths;
mod platform;
mod recent;
mod state;
mod templates;
mod timeline;
mod unsaved;

use eframe::egui;
use state::AppState;

/// Desktop: open the window (with a drawing or project given on the command line).
#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    let initial = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).expect("built-in icon");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PuppetMotion 2D")
            .with_icon(icon)
            .with_inner_size([1280.0, 800.0])
            .with_drag_and_drop(true),
        // The OpenGL renderer builds mipmaps for our textures, so a big
        // drawing shrunk to fit stays smooth.
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "PuppetMotion 2D",
        options,
        Box::new(move |cc| {
            let mut app = App::new();
            // Offer back anything a crash left unsaved.
            app.state.unsaved.find_recovery();
            if let Some(path) = initial {
                app.state.open(&cc.egui_ctx, &path);
            }
            Ok(Box::new(app))
        }),
    )
}

/// Browser: start the app on the page's canvas (see `index.html`).
#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast;
    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window().and_then(|w| w.document()).expect("running in a web page");
        let canvas = document
            .get_element_by_id("puppetmotion")
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("the page has a <canvas id=\"puppetmotion\">");
        let started = eframe::WebRunner::new()
            .start(canvas, eframe::WebOptions::default(), Box::new(|_cc| Ok(Box::new(App::new()))))
            .await;
        // Swap the "Loading…" text for the app, or say what went wrong.
        if let Some(loading) = document.get_element_by_id("loading") {
            match started {
                Ok(()) => loading.remove(),
                Err(_) => loading.set_text_content(Some("Sorry, PuppetMotion couldn't start in this browser.")),
            }
        }
    });
}

struct App {
    state: AppState,
    /// Window title last sent, so it's only updated when it changes.
    title: String,
}

impl App {
    fn new() -> Self {
        App { state: AppState::default(), title: String::new() }
    }

    /// Global keys: undo/redo, save, open. Skipped while a text box has focus
    /// (it has its own undo) and mid-drag (the drag's undo step isn't recorded yet).
    fn shortcuts(&mut self, ctx: &egui::Context) {
        // Nothing behind a question box ("Save changes?", "Recover?") reacts to keys.
        if ctx.text_edit_focused() || self.state.unsaved.prompt.is_some() || self.state.unsaved.recovery.is_some() {
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
            self.state.request_open_dialog(ctx);
        }
        if pressed(ctx, Modifiers::COMMAND, Key::N) {
            self.state.request_new();
        }
        if self.state.step == state::Step::Animate {
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
        // Space plays/pauses in Animate and Export, unless it was held to pan (Space+drag).
        if ctx.input(|i| i.key_released(Key::Space)) {
            if self.state.step.shows_pose() && !self.state.space_panned {
                self.state.toggle_play();
            }
            self.state.space_panned = false;
        }
    }
}

impl App {
    /// Timeline keys in the Animate step: Ctrl+C copies the selected keys, Ctrl+X cuts
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
        // A dropped file: on desktop it's read from its path; in the browser
        // it's read asynchronously and arrives with the picked files below.
        if let Some(file) = ctx.input(|i| i.raw.dropped_files.first().cloned()) {
            #[cfg(not(target_arch = "wasm32"))]
            self.state.request_open(&ctx, file.path());
            #[cfg(target_arch = "wasm32")]
            platform::read_dropped(&ctx, file);
        }
        for picked in platform::take_picked() {
            self.state.request_open_bytes(&ctx, picked.name, picked.bytes);
        }
        self.shortcuts(&ctx);
        self.state.tick(&ctx);

        export::poll(&ctx, &mut self.state);
        unsaved::handle_close(&ctx, &mut self.state);
        unsaved::autosave(&mut self.state);

        egui::Panel::top("steps").show(ui, |ui| {
            ui.add_space(4.0);
            // Scrolls sideways when the window is too narrow for every button.
            egui::ScrollArea::horizontal().id_salt("top bar").show(ui, |ui| self.state.top_bar(ui));
            ui.add_space(2.0);
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            self.state.status_bar(ui);
        });
        let side = egui::Panel::left("side").resizable(true).default_size(360.0).show(ui, |ui| {
            self.state.side_panel(ui);
        });
        // The top bar lines the steps up with the canvas's left edge (next frame).
        self.state.canvas_left = side.response.rect.right();
        if self.state.step == state::Step::Animate && self.state.doc.is_some() {
            egui::Panel::bottom("timeline").resizable(true).default_size(160.0).min_size(70.0).max_size(600.0).show(ui, |ui| {
                timeline::show(ui, &mut self.state);
            });
        }
        egui::CentralPanel::default().show(ui, |ui| {
            canvas::show(ui, &mut self.state);
        });
        unsaved::show_prompt(&ctx, &mut self.state);
        unsaved::show_recovery(&ctx, &mut self.state);

        let title = self.state.title();
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}
