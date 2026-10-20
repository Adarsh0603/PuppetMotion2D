//! Skeleton templates the user saved from the Rig step, listed after the
//! built-in ones. On desktop each is a JSON file in the app-data folder; in
//! the browser they're kept together in the browser's local storage.

use anim_core::Template;

/// Built-in templates, then the user's (sorted by name). Ones that can't be
/// read are skipped, so one bad entry never hides the rest.
pub fn all() -> Vec<Template> {
    let mut user = load_user();
    user.sort_by_key(|t| t.name.to_lowercase());
    let mut out = anim_core::template::builtin();
    // A user template can't take a built-in's name (it would be ambiguous in the menu).
    user.retain(|u| !out.iter().any(|b| b.name.eq_ignore_ascii_case(&u.name)));
    out.extend(user);
    out
}

/// Whether `name` is taken by a template that comes with the app.
pub fn is_builtin(name: &str) -> bool {
    anim_core::template::builtin().iter().any(|b| b.name.eq_ignore_ascii_case(name.trim()))
}

#[cfg(not(target_arch = "wasm32"))]
fn dir() -> std::path::PathBuf {
    crate::paths::app_data_dir().join("templates")
}

#[cfg(not(target_arch = "wasm32"))]
fn load_user() -> Vec<Template> {
    std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter_map(|p| Template::from_json(&std::fs::read_to_string(p).ok()?).ok())
        .collect()
}

/// Save `t`, replacing a saved template with the same name. Returns where
/// it went (for the status line).
#[cfg(not(target_arch = "wasm32"))]
pub fn save(t: &Template) -> std::io::Result<String> {
    let dir = dir();
    std::fs::create_dir_all(&dir)?;
    let stem: String = t.name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
    let path = dir.join(format!("{}.json", stem.trim()));
    std::fs::write(&path, t.to_json())?;
    Ok(path.display().to_string())
}

/// Browser storage key holding the user's templates as one JSON list.
#[cfg(target_arch = "wasm32")]
const STORE_KEY: &str = "puppetmotion.templates";

#[cfg(target_arch = "wasm32")]
fn load_user() -> Vec<Template> {
    let Some(text) = crate::platform::load_text(STORE_KEY) else { return Vec::new() };
    let list: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap_or_default();
    list.iter().filter_map(|v| Template::from_json(&v.to_string()).ok()).collect()
}

#[cfg(target_arch = "wasm32")]
pub fn save(t: &Template) -> std::io::Result<String> {
    let mut user = load_user();
    user.retain(|u| !u.name.eq_ignore_ascii_case(&t.name));
    user.push(t.clone());
    let list: Vec<serde_json::Value> =
        user.iter().filter_map(|u| serde_json::from_str(&u.to_json()).ok()).collect();
    let text = serde_json::to_string(&list).map_err(std::io::Error::other)?;
    crate::platform::save_text(STORE_KEY, &text).map_err(std::io::Error::other)?;
    Ok("this browser".into())
}
