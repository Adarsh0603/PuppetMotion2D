//! Skeleton templates on disk: the user's own, saved from the Rig step into
//! the app-data folder as one JSON file each, listed after the built-in ones.

use anim_core::Template;
use std::path::PathBuf;

fn dir() -> PathBuf {
    crate::paths::app_data_dir().join("templates")
}

/// Built-in templates, then the user's (sorted by name). Files that can't be
/// read are skipped, so one bad file never hides the rest.
pub fn all() -> Vec<Template> {
    let mut user: Vec<Template> = std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter_map(|p| Template::from_json(&std::fs::read_to_string(p).ok()?).ok())
        .collect();
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

/// Save `t` as `<name>.json`, replacing a saved template with the same name.
pub fn save(t: &Template) -> std::io::Result<PathBuf> {
    let dir = dir();
    std::fs::create_dir_all(&dir)?;
    let stem: String = t.name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
    let path = dir.join(format!("{}.json", stem.trim()));
    std::fs::write(&path, t.to_json())?;
    Ok(path)
}
