mod canvas;
mod state;

use eframe::egui;
use state::AppState;
use std::path::PathBuf;

fn main() -> eframe::Result {
    let initial = std::env::args_os().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Animator")
            .with_inner_size([1280.0, 800.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "Animator",
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

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let dropped = ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()));
        if let Some(path) = dropped {
            self.state.open(&ctx, &path);
        }

        egui::Panel::left("side").resizable(true).default_size(260.0).show(ui, |ui| {
            self.state.side_panel(ui);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            canvas::show(ui, &mut self.state);
        });
    }
}
