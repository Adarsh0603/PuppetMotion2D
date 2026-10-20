//! Project files (`.pm2d`): a zip holding `project.json` and the original
//! image, byte for byte. Masks and sprites aren't stored; they're recomputed
//! from the saved joints and settings when the project is opened.
//!
//! `format_version` starts at 1. A newer app can read older files (new fields
//! get defaults); an older app refuses files newer than it understands.

use crate::cutout::Params;
use crate::motion::Motion;
use crate::rig::Pose;
use crate::skeleton::Skeleton;
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
use std::path::Path;

/// The newest project format this build reads and the one it writes.
pub const FORMAT_VERSION: u32 = 1;

pub const EXTENSION: &str = "pm2d";

const JSON_NAME: &str = "project.json";
const IMAGE_STEM: &str = "image.";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectData {
    pub format_version: u32,
    /// File name of the drawing when it was first opened (for display).
    pub image_name: String,
    #[serde(default)]
    pub params: Params,
    pub skeleton: Skeleton,
    #[serde(default)]
    pub pose: Pose,
    /// Joint overlap (disc size) used for the cut-out parts.
    #[serde(default = "default_overlap")]
    pub overlap: f32,
    /// Keyed animation (empty in projects saved before motion existed).
    #[serde(default)]
    pub motion: Motion,
    /// How smoothly the drawing bends at the joints (0 = rigid pieces).
    /// Projects saved before bending existed open rigid, so they look the same.
    #[serde(default)]
    pub softness: f32,
}

fn default_overlap() -> f32 {
    1.0
}

/// A loaded project: its data plus the original image file's bytes and extension.
pub struct LoadedProject {
    pub data: ProjectData,
    pub image_bytes: Vec<u8>,
    pub image_ext: String,
}

/// Write a project. Goes to a temporary file first and is then renamed into
/// place, so a crash mid-save never leaves a half-written project behind.
pub fn save_project(path: &Path, data: &ProjectData, image_bytes: &[u8], image_ext: &str) -> io::Result<()> {
    use zip::write::SimpleFileOptions;
    let tmp = path.with_extension(format!("{EXTENSION}.tmp"));
    {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&tmp)?);
        let deflate = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file(JSON_NAME, deflate).map_err(io::Error::other)?;
        let json = serde_json::to_vec_pretty(data).map_err(io::Error::other)?;
        zip.write_all(&json)?;
        // Images are already compressed; store them as-is.
        let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file(format!("{IMAGE_STEM}{image_ext}"), stored).map_err(io::Error::other)?;
        zip.write_all(image_bytes)?;
        zip.finish().map_err(io::Error::other)?;
    }
    std::fs::rename(&tmp, path)
}

pub fn load_project(path: &Path) -> io::Result<LoadedProject> {
    let bad = |msg: String| io::Error::new(io::ErrorKind::InvalidData, msg);
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path)?)
        .map_err(|e| bad(format!("not a project file ({e})")))?;

    let mut json = Vec::new();
    zip.by_name(JSON_NAME).map_err(|_| bad("project.json is missing".into()))?.read_to_end(&mut json)?;
    // Check the version before parsing everything, so a newer file gets a clear message.
    let version: serde_json::Value = serde_json::from_slice(&json).map_err(|e| bad(format!("project.json: {e}")))?;
    let found = version.get("format_version").and_then(|v| v.as_u64()).unwrap_or(0);
    if found > FORMAT_VERSION as u64 {
        return Err(bad(format!(
            "this project was saved by a newer version of the app (format {found}); this version reads up to format {FORMAT_VERSION}"
        )));
    }
    let data: ProjectData = serde_json::from_slice(&json).map_err(|e| bad(format!("project.json: {e}")))?;

    let image_name = zip
        .file_names()
        .filter_map(Result::ok)
        .find(|n| n.starts_with(IMAGE_STEM))
        .map(|n| n.into_owned())
        .ok_or_else(|| bad("the image is missing from the project".into()))?;
    let mut image_bytes = Vec::new();
    zip.by_name(&image_name).map_err(io::Error::other)?.read_to_end(&mut image_bytes)?;
    let image_ext = image_name[IMAGE_STEM.len()..].to_string();
    Ok(LoadedProject { data, image_bytes, image_ext })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::RectI;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("pm2d_test_{}_{name}.{EXTENSION}", std::process::id()))
    }

    fn sample() -> ProjectData {
        let mut pose = Pose::rest(10);
        pose.set_angle(3, 0.7);
        pose.offset = [4.0, -2.0];
        ProjectData {
            format_version: FORMAT_VERSION,
            image_name: "kid_dragon.jpg".into(),
            params: Params { gap_close: 6, ..Params::default() },
            skeleton: Skeleton::person_front(RectI::new(0, 0, 100, 200)),
            pose,
            overlap: 0.5,
            softness: 0.8,
            motion: {
                let mut m = Motion::default();
                m.offset_x.push(crate::motion::Key { t: 0.5, value: 2.0, ease: crate::motion::Ease::Hold });
                m
            },
        }
    }

    #[test]
    fn round_trip() {
        let path = temp_path("roundtrip");
        let data = sample();
        let image = vec![1u8, 2, 3, 250, 0, 7];
        save_project(&path, &data, &image, "jpg").unwrap();
        let loaded = load_project(&path).unwrap();
        assert_eq!(loaded.data, data);
        assert_eq!(loaded.image_bytes, image);
        assert_eq!(loaded.image_ext, "jpg");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn newer_format_is_refused() {
        let path = temp_path("future");
        let data = ProjectData { format_version: FORMAT_VERSION + 1, ..sample() };
        save_project(&path, &data, &[0], "png").unwrap();
        let err = load_project(&path).err().unwrap().to_string();
        assert!(err.contains("newer version"), "{err}");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn older_data_without_new_fields_loads() {
        // A part saved before rotation limits existed, a project without
        // pose/overlap, and fields from removed features (boxes, keep_ratio).
        let json = r#"{
            "format_version": 1,
            "image_name": "a.png",
            "params": {"threshold": 30.0, "keep_ratio": 0.2},
            "boxes": [{"rect": {"x": 1, "y": 2, "w": 3, "h": 4}, "axis": "DiagUp"}],
            "skeleton": {
                "joints": [{"name": "a", "pos": [0, 0]}, {"name": "b", "pos": [10, 0]}],
                "parts": [{"name": "bone 1", "pivot": 0, "tip": 1, "parent": null}]
            }
        }"#;
        let data: ProjectData = serde_json::from_str(json).unwrap();
        assert_eq!(data.skeleton.parts[0].limit, None);
        assert_eq!(data.params, Params { threshold: 30.0, ..Params::default() });
        assert_eq!(data.overlap, 1.0);
        assert!(data.motion.is_empty(), "no motion in an older project");
    }

    #[test]
    fn not_a_project_gives_a_clear_error() {
        let path = temp_path("garbage");
        std::fs::write(&path, b"hello").unwrap();
        let err = load_project(&path).err().unwrap().to_string();
        assert!(err.contains("not a project file"), "{err}");
        std::fs::remove_file(&path).unwrap();
    }
}
