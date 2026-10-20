//! What differs between the desktop app and the browser version, kept in one
//! place. The rest of the app calls these and doesn't need to know which one
//! it's running as.
//!
//! - Desktop: native file dialogs and paths, files in the app-data folder.
//! - Browser: the browser's file picker (it answers later, so picked files
//!   arrive through `take_picked`), downloads instead of saving to a path,
//!   and the browser's local storage instead of files.

#[cfg(not(target_arch = "wasm32"))]
mod desktop;
#[cfg(not(target_arch = "wasm32"))]
pub use desktop::*;

#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
pub use web::*;

/// Whether this is the browser version.
pub const IS_WEB: bool = cfg!(target_arch = "wasm32");

/// A file the user picked, by name and contents.
pub struct Picked {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// Image file types the app opens (lower case, no dot).
pub const IMAGE_TYPES: [&str; 6] = ["png", "jpg", "jpeg", "webp", "bmp", "gif"];
