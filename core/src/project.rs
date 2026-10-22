//! Project files (`.pm2d`): a zip holding `project.json` and the original
//! image, byte for byte. Masks and sprites aren't stored; they're recomputed
//! from the saved joints and settings when the project is opened.
//!
//! `format_version` starts at 1. A newer app can read older files (new fields
//! get defaults); an older app refuses files newer than it understands.
//! Format 2 adds drawing layers (`layers/NNN.png`, listed in `layers`); a
//! project without layers is still written as format 1.

use crate::cutout::Params;
use crate::motion::Motion;
use crate::rig::Pose;
use crate::skeleton::Skeleton;
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
use std::path::Path;

/// The newest project format this build reads (and writes, for layered projects).
pub const FORMAT_VERSION: u32 = 2;

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
    /// Hand-painted bone weights (none in older projects).
    #[serde(default)]
    pub weight_paint: crate::weight_paint::WeightPaint,
    /// Guess what's hidden behind parts, shown when they move away. Off unless
    /// switched on (also for older projects): its guesses aren't reliable yet.
    #[serde(default)]
    pub fill_behind: bool,
    /// Drawing layers, bottom to top (none for a single drawing).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<LayerInfo>,
}

/// One saved drawing layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerInfo {
    pub name: String,
    /// Its PNG inside the project file.
    pub file: String,
    /// Bones chosen by hand for it to move with; empty = guessed.
    #[serde(default)]
    pub bones: Vec<String>,
}

fn default_overlap() -> f32 {
    1.0
}

/// A loaded project: its data plus the original image file's bytes and
/// extension, and each layer's PNG bytes (in `data.layers` order).
pub struct LoadedProject {
    pub data: ProjectData,
    pub image_bytes: Vec<u8>,
    pub image_ext: String,
    pub layer_pngs: Vec<Vec<u8>>,
}

/// Write a project. Goes to a temporary file first and is then renamed into
/// place, so a crash mid-save never leaves a half-written project behind.
/// `layer_pngs` holds each of `data.layers`' PNG bytes, in order.
pub fn save_project(path: &Path, data: &ProjectData, image_bytes: &[u8], image_ext: &str, layer_pngs: &[Vec<u8>]) -> io::Result<()> {
    let tmp = path.with_extension(format!("{EXTENSION}.tmp"));
    write_project(std::fs::File::create(&tmp)?, data, image_bytes, image_ext, layer_pngs)?;
    std::fs::rename(&tmp, path)
}

/// A project as the bytes of a `.pm2d` file (for the browser, where it's
/// downloaded instead of written to a path).
pub fn project_to_bytes(data: &ProjectData, image_bytes: &[u8], image_ext: &str, layer_pngs: &[Vec<u8>]) -> io::Result<Vec<u8>> {
    let mut out = io::Cursor::new(Vec::new());
    write_project(&mut out, data, image_bytes, image_ext, layer_pngs)?;
    Ok(out.into_inner())
}

/// The format to save a project in: 2 with layers, else 1, so projects
/// without layers still open in older builds.
pub fn format_for(has_layers: bool) -> u32 {
    if has_layers { 2 } else { 1 }
}

/// The file name a layer is saved under inside the project.
pub fn layer_file(index: usize) -> String {
    format!("layers/{index:03}.png")
}

fn write_project<W: Write + io::Seek>(
    to: W,
    data: &ProjectData,
    image_bytes: &[u8],
    image_ext: &str,
    layer_pngs: &[Vec<u8>],
) -> io::Result<()> {
    use zip::write::SimpleFileOptions;
    if layer_pngs.len() != data.layers.len() {
        return Err(io::Error::other("every layer needs its image"));
    }
    let mut zip = zip::ZipWriter::new(to);
    let deflate = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zip.start_file(JSON_NAME, deflate).map_err(io::Error::other)?;
    // Layers need format 2 (see `format_for`).
    let version = if data.layers.is_empty() { data.format_version } else { data.format_version.max(2) };
    let json = serde_json::to_vec_pretty(&ProjectData { format_version: version, ..data.clone() }).map_err(io::Error::other)?;
    zip.write_all(&json)?;
    // Images are already compressed; store them as-is.
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file(format!("{IMAGE_STEM}{image_ext}"), stored).map_err(io::Error::other)?;
    zip.write_all(image_bytes)?;
    for (info, png) in data.layers.iter().zip(layer_pngs) {
        zip.start_file(&info.file, stored).map_err(io::Error::other)?;
        zip.write_all(png)?;
    }
    zip.finish().map_err(io::Error::other)?;
    Ok(())
}

pub fn load_project(path: &Path) -> io::Result<LoadedProject> {
    read_project(std::fs::File::open(path)?)
}

/// Read a project from the bytes of a `.pm2d` file.
pub fn project_from_bytes(bytes: &[u8]) -> io::Result<LoadedProject> {
    read_project(io::Cursor::new(bytes))
}

fn read_project<R: Read + io::Seek>(from: R) -> io::Result<LoadedProject> {
    let bad = |msg: String| io::Error::new(io::ErrorKind::InvalidData, msg);
    let mut zip = zip::ZipArchive::new(from).map_err(|e| bad(format!("not a project file ({e})")))?;

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
    let mut layer_pngs = Vec::with_capacity(data.layers.len());
    for info in &data.layers {
        let mut png = Vec::new();
        zip.by_name(&info.file).map_err(|_| bad(format!("layer \"{}\" is missing from the project", info.name)))?.read_to_end(&mut png)?;
        layer_pngs.push(png);
    }
    Ok(LoadedProject { data, image_bytes, image_ext, layer_pngs })
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
            fill_behind: false,
            weight_paint: {
                let mut p = crate::weight_paint::WeightPaint::default();
                p.dab("head", [10.0, 12.0], 6.0, 0.4, 3.0);
                p
            },
            motion: {
                let mut m = Motion::default();
                m.offset_x.push(crate::motion::Key { t: 0.5, value: 2.0, ease: crate::motion::Ease::Hold });
                m
            },
            layers: Vec::new(),
        }
    }

    #[test]
    fn round_trip() {
        let path = temp_path("roundtrip");
        let data = ProjectData { format_version: format_for(false), ..sample() };
        let image = vec![1u8, 2, 3, 250, 0, 7];
        save_project(&path, &data, &image, "jpg", &[]).unwrap();
        let loaded = load_project(&path).unwrap();
        assert_eq!(loaded.data, data);
        assert_eq!(loaded.data.format_version, 1, "no layers: still format 1");
        assert_eq!(loaded.image_bytes, image);
        assert_eq!(loaded.image_ext, "jpg");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn round_trip_in_memory() {
        let data = sample();
        let bytes = project_to_bytes(&data, &[9, 8, 7], "png", &[]).unwrap();
        let loaded = project_from_bytes(&bytes).unwrap();
        assert_eq!(loaded.data, data);
        assert_eq!(loaded.image_bytes, vec![9, 8, 7]);
        assert_eq!(loaded.image_ext, "png");
    }

    #[test]
    fn layers_round_trip() {
        let layer = |i: usize, name: &str, bones: &[&str]| LayerInfo {
            name: name.into(),
            file: layer_file(i),
            bones: bones.iter().map(|b| b.to_string()).collect(),
        };
        let data = ProjectData {
            format_version: format_for(true),
            layers: vec![layer(0, "far leg", &[]), layer(1, "body", &["torso", "head"])],
            ..sample()
        };
        let pngs = vec![vec![1u8, 1], vec![2u8, 2, 2]];
        let loaded = project_from_bytes(&project_to_bytes(&data, &[0], "png", &pngs).unwrap()).unwrap();
        assert_eq!(loaded.data, data);
        assert_eq!(loaded.data.format_version, 2);
        assert_eq!(loaded.layer_pngs, pngs);
        assert!(project_to_bytes(&data, &[0], "png", &pngs[..1]).is_err(), "a layer without its image");
    }

    #[test]
    fn newer_format_is_refused() {
        let path = temp_path("future");
        let data = ProjectData { format_version: FORMAT_VERSION + 1, ..sample() };
        save_project(&path, &data, &[0], "png", &[]).unwrap();
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
