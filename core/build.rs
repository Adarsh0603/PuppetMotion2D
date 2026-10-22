//! Finds every ready-made motion in `motions/` (any folder depth) and lists
//! them for `cycles.rs`, so adding or deleting a motion is just adding or
//! deleting its JSON file.

use std::path::{Path, PathBuf};

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("motions");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = Vec::new();
    collect(&root, &mut files);
    files.sort();
    let list: String = files.iter().map(|f| format!("    include_str!({:?}),\n", f.display().to_string())).collect();
    let code = format!("/// Every JSON file under `motions/`, found when building.\nconst BUILTIN: &[&str] = &[\n{list}];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("motions.rs");
    std::fs::write(out, code).unwrap();
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            println!("cargo:rerun-if-changed={}", path.display());
            collect(&path, files);
        } else if path.extension().is_some_and(|x| x == "json") {
            files.push(path);
        }
    }
}
