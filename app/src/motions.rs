//! Motions the user saved from the Animate step, listed with the built-in
//! ones. On desktop each is a JSON file in the app-data folder; in the
//! browser they're kept together in the browser's local storage.

use anim_core::Cycle;

/// Category offered first when saving your own motion.
pub const USER_CATEGORY: &str = "My motions";

/// Built-in motions and the user's, in menu order. Ones that can't be read
/// are skipped, so one bad entry never hides the rest.
pub fn all() -> Vec<Cycle> {
    let mut user = load_user();
    user.retain(|u| !is_builtin(&u.name));
    let mut out = anim_core::builtin_cycles();
    out.extend(user);
    anim_core::cycles::sort(&mut out);
    out
}

/// Whether `name` is taken by a motion that comes with the app.
pub fn is_builtin(name: &str) -> bool {
    anim_core::builtin_cycles().iter().any(|b| b.name.eq_ignore_ascii_case(name.trim()))
}

/// Whether `c` is one of the user's own (so it can be deleted).
pub fn is_user(c: &Cycle) -> bool {
    !anim_core::builtin_cycles().iter().any(|b| b.name == c.name && b.template == c.template)
}

#[cfg(not(target_arch = "wasm32"))]
fn dir() -> std::path::PathBuf {
    crate::paths::app_data_dir().join("motions")
}

#[cfg(not(target_arch = "wasm32"))]
fn file(name: &str) -> std::path::PathBuf {
    let stem: String = name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
    dir().join(format!("{}.json", stem.trim()))
}

#[cfg(not(target_arch = "wasm32"))]
fn load_user() -> Vec<Cycle> {
    std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter_map(|p| Cycle::from_json(&std::fs::read_to_string(p).ok()?).ok())
        .collect()
}

/// Save `c`, replacing a saved motion with the same name. Returns where it
/// went (for the status line).
#[cfg(not(target_arch = "wasm32"))]
pub fn save(c: &Cycle) -> std::io::Result<String> {
    std::fs::create_dir_all(dir())?;
    let path = file(&c.name);
    std::fs::write(&path, c.to_json())?;
    Ok(path.display().to_string())
}

/// Delete the saved motion called `name`.
#[cfg(not(target_arch = "wasm32"))]
pub fn delete(name: &str) -> std::io::Result<()> {
    std::fs::remove_file(file(name))
}

/// Browser storage key holding the user's motions as one JSON list.
#[cfg(target_arch = "wasm32")]
const STORE_KEY: &str = "puppetmotion.motions";

#[cfg(target_arch = "wasm32")]
fn load_user() -> Vec<Cycle> {
    let Some(text) = crate::platform::load_text(STORE_KEY) else { return Vec::new() };
    let list: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap_or_default();
    list.iter().filter_map(|v| Cycle::from_json(&v.to_string()).ok()).collect()
}

#[cfg(target_arch = "wasm32")]
fn store(user: &[Cycle]) -> std::io::Result<()> {
    let list: Vec<serde_json::Value> = user.iter().filter_map(|u| serde_json::from_str(&u.to_json()).ok()).collect();
    let text = serde_json::to_string(&list).map_err(std::io::Error::other)?;
    crate::platform::save_text(STORE_KEY, &text).map_err(std::io::Error::other)
}

#[cfg(target_arch = "wasm32")]
pub fn save(c: &Cycle) -> std::io::Result<String> {
    let mut user = load_user();
    user.retain(|u| !u.name.eq_ignore_ascii_case(&c.name));
    user.push(c.clone());
    store(&user)?;
    Ok("this browser".into())
}

#[cfg(target_arch = "wasm32")]
pub fn delete(name: &str) -> std::io::Result<()> {
    let mut user = load_user();
    user.retain(|u| u.name != name);
    store(&user)
}
