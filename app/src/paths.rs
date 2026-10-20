//! Where the app keeps its own files (recovery copies, saved templates).

use std::path::PathBuf;

/// The per-user app-data folder: `%LOCALAPPDATA%\PuppetMotion2D` on Windows,
/// `$XDG_DATA_HOME/PuppetMotion2D` or `~/.local/share/PuppetMotion2D` elsewhere.
/// Tests get a temporary folder, so they never touch real files.
pub fn app_data_dir() -> PathBuf {
    // The browser has no files (asking for a temp folder would even crash);
    // it keeps things in its own storage instead (see `platform`).
    if cfg!(target_arch = "wasm32") {
        return PathBuf::new();
    }
    if cfg!(test) {
        return std::env::temp_dir().join(format!("pm2d_test_appdata_{}", std::process::id()));
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("XDG_DATA_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("PuppetMotion2D")
}
