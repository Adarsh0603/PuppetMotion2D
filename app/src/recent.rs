//! Recently opened or saved projects, newest first, for the Recent menu. Kept
//! as one path per line in `recent.txt` in the app-data folder. The browser
//! has no file paths to go back to, so it keeps no list.

use std::path::{Path, PathBuf};

/// How many projects the list remembers.
const KEEP: usize = 50;

#[cfg(not(target_arch = "wasm32"))]
fn file() -> PathBuf {
    crate::paths::app_data_dir().join("recent.txt")
}

/// The list as last saved (empty if there's none yet).
pub fn load() -> Vec<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::read_to_string(file())
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(PathBuf::from)
            .take(KEEP)
            .collect()
    }
    #[cfg(target_arch = "wasm32")]
    Vec::new()
}

/// Put `path` at the top of `list` (moving it if it's already there) and save.
pub fn add(list: &mut Vec<PathBuf>, path: &Path) {
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    list.retain(|p| !same(p, &path));
    list.insert(0, path);
    list.truncate(KEEP);
    save(list);
}

/// Forget every project in the list.
pub fn clear(list: &mut Vec<PathBuf>) {
    list.clear();
    save(list);
}

/// Whether two paths name the same file (Windows paths ignore case).
fn same(a: &Path, b: &Path) -> bool {
    if cfg!(windows) { a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy()) } else { a == b }
}

fn save(list: &[PathBuf]) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let text: String = list.iter().map(|p| format!("{}\n", p.display())).collect();
        let _ = std::fs::create_dir_all(crate::paths::app_data_dir());
        // A list that can't be saved is only a convenience lost; never interrupt work for it.
        let _ = std::fs::write(file(), text);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = list;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_without_repeats() {
        let mut list = Vec::new();
        let (a, b) = (std::env::temp_dir().join("a.pm2d"), std::env::temp_dir().join("b.pm2d"));
        add(&mut list, &a);
        add(&mut list, &b);
        add(&mut list, &a);
        assert_eq!(list, vec![a.clone(), b.clone()]);
        // (Reading it back isn't checked here: tests share one app-data folder and run at once.)
        clear(&mut list);
        assert!(list.is_empty());
    }
}
