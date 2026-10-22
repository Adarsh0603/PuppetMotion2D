//! Browser: the file picker, downloads and local storage.

use super::{IMAGE_TYPES, Picked};
use eframe::egui;
use std::cell::RefCell;
use wasm_bindgen::JsCast;

thread_local! {
    /// Files the browser's picker has handed over, waiting for the app: one
    /// list per pick or drop.
    static PICKED: RefCell<Vec<Vec<Picked>>> = const { RefCell::new(Vec::new()) };
}

/// Show the browser's file picker for a drawing, a project, or several layer
/// images. The browser answers later; the files arrive through `take_picked`.
pub fn pick_file(ctx: &egui::Context) {
    let ctx = ctx.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let mut all: Vec<&str> = IMAGE_TYPES.to_vec();
        all.push(anim_core::project::EXTENSION);
        let Some(files) = rfd::AsyncFileDialog::new().add_filter("Drawings and projects", &all).pick_files().await else {
            return;
        };
        let mut batch = Vec::with_capacity(files.len());
        for file in files {
            batch.push(Picked { name: file.file_name(), bytes: file.read().await });
        }
        PICKED.with(|p| p.borrow_mut().push(batch));
        ctx.request_repaint();
    });
}

/// Files dropped on the page: the browser reads them later; they arrive
/// together through `take_picked`, like picked ones.
pub fn read_dropped(ctx: &egui::Context, files: Vec<egui::DroppedFileHandle>) {
    let ctx = ctx.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let mut batch = Vec::with_capacity(files.len());
        for file in files {
            let name = file.path().to_string_lossy().into_owned();
            if let Ok(bytes) = file.bytes_async().await {
                batch.push(Picked { name, bytes });
            }
        }
        PICKED.with(|p| p.borrow_mut().push(batch));
        ctx.request_repaint();
    });
}

/// Files picked or dropped since the last call, one list per pick or drop.
pub fn take_picked() -> Vec<Vec<Picked>> {
    PICKED.with(|p| std::mem::take(&mut *p.borrow_mut()))
}

/// Hand `bytes` to the browser as a download called `name`.
pub fn download(name: &str, bytes: &[u8], mime: &str) {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else { return };
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(bytes));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(mime);
    let Ok(blob) = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options) else { return };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else { return };
    if let Ok(link) = document.create_element("a") {
        let link: web_sys::HtmlAnchorElement = link.unchecked_into();
        link.set_href(&url);
        link.set_download(name);
        link.click();
    }
    // Let the browser start the download before the link is forgotten.
    let revoke = wasm_bindgen::closure::Closure::once_into_js(move || {
        let _ = web_sys::Url::revoke_object_url(&url);
    });
    if let Some(window) = web_sys::window() {
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 10_000);
    }
}

/// Text kept in the browser's local storage under `key`.
pub fn load_text(key: &str) -> Option<String> {
    web_sys::window()?.local_storage().ok()??.get_item(key).ok()?
}

/// Keep `text` in the browser's local storage under `key`.
pub fn save_text(key: &str, text: &str) -> Result<(), String> {
    let storage = web_sys::window().and_then(|w| w.local_storage().ok().flatten()).ok_or("no browser storage")?;
    storage.set_item(key, text).map_err(|_| "the browser's storage is full or blocked".to_string())
}
