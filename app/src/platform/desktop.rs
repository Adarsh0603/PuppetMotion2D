//! Desktop: files are opened and saved through native dialogs by path (see
//! `AppState::open_dialog` and friends), so most of the browser-only helpers
//! have nothing to do here.

use super::Picked;

/// Files picked asynchronously since the last call, one list per pick.
/// Desktop dialogs answer straight away, so there never are any.
pub fn take_picked() -> Vec<Vec<Picked>> {
    Vec::new()
}
