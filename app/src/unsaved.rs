//! Keeping work safe: the "Save changes?" prompt before closing or opening
//! another file, a recovery copy autosaved every minute, and the offer to
//! recover it after a crash.
//!
//! The recovery copy is an ordinary project file (`autosave.pm2d`) in the
//! user's app-data folder, next to `autosave.txt`, which records where the
//! real project lives (if it was ever saved). Both are deleted after a real
//! save, a clean exit, or choosing "Don't save".

use crate::state::AppState;
use anim_core::{ProjectData, load_project, save_project};
use eframe::egui;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// How often unsaved changes are copied to the recovery file.
const AUTOSAVE_EVERY: Duration = Duration::from_secs(60);

const RECOVERY_FILE: &str = "autosave.pm2d";
const RECOVERY_INFO: &str = "autosave.txt";

/// What to do once the user has answered "Save changes?".
#[derive(Clone, Debug, PartialEq)]
pub enum Pending {
    Close,
    OpenDialog,
    Open(PathBuf),
}

/// A recovery copy found at startup.
pub struct Recovery {
    /// The drawing's name.
    name: String,
    /// The project it came from, if it had been saved before.
    original: Option<PathBuf>,
    saved_at: SystemTime,
}

pub struct Unsaved {
    /// The "Save changes?" prompt is showing, for this action.
    pub prompt: Option<Pending>,
    /// The user agreed to close; let the next close request through.
    allow_close: bool,
    /// Where the recovery copy lives.
    dir: PathBuf,
    last_check: Instant,
    /// What was last autosaved, so an unchanged drawing isn't written again.
    last_saved: Option<ProjectData>,
    /// A recovery copy left by a previous run, offered until accepted or discarded.
    pub recovery: Option<Recovery>,
}

impl Unsaved {
    pub fn new(dir: PathBuf) -> Self {
        Unsaved { prompt: None, allow_close: false, dir, last_check: Instant::now(), last_saved: None, recovery: None }
    }

    fn file(&self) -> PathBuf {
        self.dir.join(RECOVERY_FILE)
    }

    /// Delete the recovery copy (after a real save, or when changes are thrown away).
    pub fn clear_recovery(&mut self) {
        let _ = std::fs::remove_file(self.file());
        let _ = std::fs::remove_file(self.dir.join(RECOVERY_INFO));
        self.last_saved = None;
    }

    /// Look for a recovery copy left behind by a crash.
    pub fn find_recovery(&mut self) {
        let file = self.file();
        let Ok(saved_at) = std::fs::metadata(&file).and_then(|m| m.modified()) else { return };
        let info = std::fs::read_to_string(self.dir.join(RECOVERY_INFO)).unwrap_or_default();
        let mut lines = info.lines();
        let name = lines.next().filter(|s| !s.is_empty()).unwrap_or("drawing").to_string();
        let original = lines.next().filter(|s| !s.is_empty()).map(PathBuf::from);
        self.recovery = Some(Recovery { name, original, saved_at });
    }
}

/// Where recovery copies go: the per-user app-data folder.
pub fn recovery_dir() -> PathBuf {
    // Tests must never touch (or delete) a real recovery copy.
    if cfg!(test) {
        return std::env::temp_dir().join(format!("pm2d_test_recovery_{}", std::process::id()));
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("XDG_DATA_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("PuppetMotion2D").join("recovery")
}

/// Copy unsaved changes to the recovery file, at most once a minute. Call once per frame.
pub fn autosave(state: &mut AppState) {
    if state.unsaved.last_check.elapsed() < AUTOSAVE_EVERY {
        return;
    }
    autosave_now(state);
}

/// Write the recovery copy if there are unsaved changes it doesn't have yet.
pub fn autosave_now(state: &mut AppState) {
    state.unsaved.last_check = Instant::now();
    let Some(doc) = &state.doc else { return };
    if !doc.dirty {
        return;
    }
    let Some(data) = state.project_data() else { return };
    if state.unsaved.last_saved.as_ref() == Some(&data) {
        return;
    }
    let u = &mut state.unsaved;
    let written = std::fs::create_dir_all(&u.dir)
        .and_then(|()| save_project(&u.file(), &data, &doc.image_bytes, &doc.image_ext))
        .and_then(|()| {
            let original = doc.project_path.as_deref().map(Path::to_string_lossy).unwrap_or_default();
            std::fs::write(u.dir.join(RECOVERY_INFO), format!("{}\n{original}\n", doc.name))
        });
    // A failed autosave is retried next minute; it never interrupts work.
    if written.is_ok() {
        u.last_saved = Some(data);
    }
}

/// Stop the window closing while there are unsaved changes, and ask instead.
/// Call once per frame.
pub fn handle_close(ctx: &egui::Context, state: &mut AppState) {
    if !ctx.input(|i| i.viewport().close_requested()) {
        return;
    }
    let dirty = state.doc.as_ref().is_some_and(|d| d.dirty);
    if dirty && !state.unsaved.allow_close {
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        state.unsaved.prompt = Some(Pending::Close);
    } else if !dirty {
        // A clean exit: nothing to recover next time.
        state.unsaved.clear_recovery();
    }
}

/// The "Save changes?" box, while one is pending.
pub fn show_prompt(ctx: &egui::Context, state: &mut AppState) {
    let Some(pending) = state.unsaved.prompt.clone() else { return };
    let name = state.doc.as_ref().map_or_else(String::new, |d| d.name.clone());
    let action = match pending {
        Pending::Close => "before closing",
        Pending::OpenDialog | Pending::Open(_) => "before opening another file",
    };
    let mut choice = None;
    let modal = egui::Modal::new(egui::Id::unique("save prompt")).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.heading(format!("Save changes to {name} {action}?"));
        ui.add_space(4.0);
        ui.label("If you don't save, your changes since the last save are lost.");
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button(egui::RichText::new("Save").strong()).clicked() {
                choice = Some(Choice::Save);
            }
            if ui.button("Don't save").clicked() {
                choice = Some(Choice::Discard);
            }
            if ui.button("Cancel").clicked() {
                choice = Some(Choice::Cancel);
            }
        });
    });
    if modal.should_close() && choice.is_none() {
        choice = Some(Choice::Cancel);
    }
    match choice {
        Some(Choice::Save) => {
            state.unsaved.prompt = None;
            state.save();
            // Only carry on if it really saved (Save As can be cancelled, or fail).
            if !state.doc.as_ref().is_some_and(|d| d.dirty) {
                proceed(ctx, state, pending);
            }
        }
        Some(Choice::Discard) => {
            state.unsaved.prompt = None;
            state.unsaved.clear_recovery();
            proceed(ctx, state, pending);
        }
        Some(Choice::Cancel) => state.unsaved.prompt = None,
        None => {}
    }
}

enum Choice {
    Save,
    Discard,
    Cancel,
}

/// Do what was waiting on the prompt.
fn proceed(ctx: &egui::Context, state: &mut AppState, pending: Pending) {
    match pending {
        Pending::Close => {
            state.unsaved.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        Pending::OpenDialog => state.open_dialog(ctx),
        Pending::Open(path) => state.open(ctx, &path),
    }
}

/// The "Recover unsaved work?" box, shown once at startup if a recovery copy exists.
pub fn show_recovery(ctx: &egui::Context, state: &mut AppState) {
    let Some(r) = &state.unsaved.recovery else { return };
    let ago = r.saved_at.elapsed().map_or(String::new(), |d| format!(" ({})", ago(d)));
    let (name, original) = (r.name.clone(), r.original.clone());
    let mut choice = None;
    egui::Modal::new(egui::Id::unique("recovery")).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.heading("Recover unsaved work?");
        ui.add_space(4.0);
        ui.label(format!("PuppetMotion closed before {name} was saved. An autosaved copy was kept{ago}."));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button(egui::RichText::new("Recover").strong()).clicked() {
                choice = Some(true);
            }
            if ui.button("Discard").clicked() {
                choice = Some(false);
            }
        });
    });
    match choice {
        Some(true) => {
            state.unsaved.recovery = None;
            recover(ctx, state, original);
        }
        Some(false) => {
            state.unsaved.recovery = None;
            state.unsaved.clear_recovery();
        }
        None => {}
    }
}

/// Open the recovery copy as if it were the original project, still unsaved.
fn recover(ctx: &egui::Context, state: &mut AppState, original: Option<PathBuf>) {
    let file = state.unsaved.file();
    if let Err(e) = load_project(&file) {
        state.status = format!("Could not recover: {e}");
        state.unsaved.clear_recovery();
        return;
    }
    state.open(ctx, &file);
    if let Some(doc) = &mut state.doc {
        // Saving goes to the real project (or asks where), never to the recovery file.
        doc.project_path = original;
        doc.dirty = true;
        state.status = "Recovered unsaved work. Save it to keep it.".into();
    }
}

/// "3 minutes ago" style, for the recovery message.
fn ago(d: Duration) -> String {
    let m = d.as_secs() / 60;
    match m {
        0 => "just now".into(),
        1 => "1 minute ago".into(),
        2..=59 => format!("{m} minutes ago"),
        _ if m < 120 => "1 hour ago".into(),
        _ if m < 60 * 48 => format!("{} hours ago", m / 60),
        _ => format!("{} days ago", m / (60 * 24)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pm2d_unsaved_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn open_sample(dir: &Path) -> AppState {
        let mut state = AppState { unsaved: Unsaved::new(dir.to_path_buf()), ..AppState::default() };
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../testdata/synthetic_sketch.png");
        state.open(&egui::Context::default(), &path);
        assert!(state.doc.is_some(), "{}", state.status);
        state
    }

    #[test]
    fn opening_with_unsaved_changes_asks_first() {
        let dir = temp_dir("prompt");
        let mut state = open_sample(&dir);
        let ctx = egui::Context::default();
        let other = Path::new("elsewhere.png");

        // Clean: opens straight away (here it fails, the file doesn't exist, but no prompt).
        state.request_open(&ctx, other);
        assert_eq!(state.unsaved.prompt, None);

        state.key_all();
        assert!(state.doc.as_ref().unwrap().dirty);
        state.request_open(&ctx, other);
        assert_eq!(state.unsaved.prompt, Some(Pending::Open(other.to_path_buf())));
        state.request_open_dialog(&ctx);
        assert_eq!(state.unsaved.prompt, Some(Pending::OpenDialog));
    }

    #[test]
    fn autosave_writes_a_recovery_copy_that_comes_back() {
        let dir = temp_dir("autosave");
        let mut state = open_sample(&dir);
        let ctx = egui::Context::default();

        // Nothing to save while clean.
        autosave_now(&mut state);
        assert!(!dir.join(RECOVERY_FILE).exists());

        // An unsaved change is copied, and "remembers" the drawing's name.
        state.key_all();
        autosave_now(&mut state);
        assert!(dir.join(RECOVERY_FILE).exists());
        let motion = state.doc.as_ref().unwrap().motion.clone();

        // Next run: the copy is found and recovered as an unsaved drawing.
        let mut next = AppState { unsaved: Unsaved::new(dir.clone()), ..AppState::default() };
        next.unsaved.find_recovery();
        let r = next.unsaved.recovery.as_ref().expect("recovery offered");
        assert_eq!(r.name, "synthetic_sketch");
        assert_eq!(r.original, None, "never saved as a project");
        recover(&ctx, &mut next, None);
        let doc = next.doc.as_ref().unwrap();
        assert_eq!(doc.motion, motion);
        assert!(doc.dirty && doc.project_path.is_none(), "saving asks where, not into the recovery folder");

        // A real save removes the copy.
        let project = dir.join("saved.pm2d");
        next.save_to(&project);
        assert!(!dir.join(RECOVERY_FILE).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ago_reads_naturally() {
        assert_eq!(ago(Duration::from_secs(20)), "just now");
        assert_eq!(ago(Duration::from_secs(60 * 5)), "5 minutes ago");
        assert_eq!(ago(Duration::from_secs(3600 * 3)), "3 hours ago");
    }
}
