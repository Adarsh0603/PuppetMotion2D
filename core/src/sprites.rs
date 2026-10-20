//! Cut each part out as its own image ("sprite") so it can be moved on its own.
//!
//! A parent's sprite also keeps a disc of pixels around each child's pivot
//! (the joint overlap). In the rest pose the child is drawn on top so nothing
//! changes; when the child rotates, the disc fills the gap it would leave,
//! like the round joints of a paper cut-out puppet.

use crate::mask::Mask;
use crate::skeleton::{Skeleton, distance_transform};
use image::RgbaImage;
use serde::Serialize;
use std::path::Path;

/// Disc radius never drops below this (px), so thin limbs still get a joint.
const MIN_DISC_RADIUS: f32 = 3.0;

#[derive(Clone)]
pub struct Sprite {
    pub part: usize,
    /// Cropped pixels; transparent outside the part. Empty (0×0) if the part has no pixels.
    pub image: RgbaImage,
    /// Top-left of `image` in the original drawing's coordinates.
    pub origin: [i32; 2],
    /// Rotation center (the part's pivot joint) in the original drawing's coordinates.
    pub pivot: [f32; 2],
}

/// Each part's mask plus the joint-overlap discs it keeps for its children.
/// A disc's radius is the drawing's half-thickness right at the joint (so a
/// big head on a thin neck gets a neck-sized disc, not a head-sized one),
/// scaled by `overlap` (0 = no discs).
pub fn sprite_masks(fg: &Mask, skel: &Skeleton, masks: &[Mask], overlap: f32) -> Vec<Mask> {
    let mut out = masks.to_vec();
    if overlap <= 0.0 {
        return out;
    }
    let dt = distance_transform(fg);
    let thickness_at = |p: [f32; 2]| {
        let (x, y) = (p[0].clamp(0.0, (fg.w - 1) as f32) as usize, p[1].clamp(0.0, (fg.h - 1) as f32) as usize);
        dt[y * fg.w as usize + x]
    };
    for part in &skel.parts {
        let Some(parent) = part.parent.filter(|&p| p < out.len()) else { continue };
        let c = skel.joints[part.pivot].pos;
        let r = (thickness_at(c) * overlap).max(MIN_DISC_RADIUS);
        let (x0, x1) = ((c[0] - r).floor().max(0.0) as u32, ((c[0] + r).ceil() as u32).min(fg.w));
        let (y0, y1) = ((c[1] - r).floor().max(0.0) as u32, ((c[1] + r).ceil() as u32).min(fg.h));
        for y in y0..y1 {
            for x in x0..x1 {
                let (dx, dy) = (x as f32 + 0.5 - c[0], y as f32 + 0.5 - c[1]);
                if dx * dx + dy * dy <= r * r && fg.get(x, y) {
                    out[parent].set(x, y, true);
                }
            }
        }
    }
    out
}

/// Cut `img` into one sprite per part. `masks` come from `skeleton_masks`;
/// `fg` is the whole-figure mask.
pub fn build_sprites(
    img: &RgbaImage,
    fg: &Mask,
    skel: &Skeleton,
    masks: &[Mask],
    overlap: f32,
) -> Vec<Sprite> {
    sprite_masks(fg, skel, masks, overlap)
        .iter()
        .enumerate()
        .map(|(part, m)| {
            let pivot = skel.joints[skel.parts[part].pivot].pos;
            let Some(bb) = m.bbox() else {
                return Sprite { part, image: RgbaImage::new(0, 0), origin: [pivot[0] as i32, pivot[1] as i32], pivot };
            };
            let mut image = RgbaImage::new(bb.w as u32, bb.h as u32);
            for (x, y, px) in image.enumerate_pixels_mut() {
                let (sx, sy) = (x + bb.x as u32, y + bb.y as u32);
                if m.get(sx, sy) {
                    *px = *img.get_pixel(sx, sy);
                }
            }
            Sprite { part, image, origin: [bb.x, bb.y], pivot }
        })
        .collect()
}

#[derive(Serialize)]
struct RigFile {
    parts: Vec<RigPart>,
}

#[derive(Serialize)]
struct RigPart {
    name: String,
    file: String,
    parent: Option<String>,
    /// Top-left of the PNG in the original drawing.
    origin: [i32; 2],
    size: [u32; 2],
    /// Rotation center in the original drawing.
    pivot: [f32; 2],
    /// Rotation center relative to the PNG's top-left.
    pivot_in_image: [f32; 2],
}

/// Write one PNG per part plus `rig.json` describing names, parents and pivots.
pub fn write_parts(dir: &Path, sprites: &[Sprite], skel: &Skeleton) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut files: Vec<String> = Vec::new();
    for s in sprites {
        let base = file_stem(&skel.parts[s.part].name);
        let mut file = format!("{base}.png");
        let mut n = 2;
        while files.contains(&file) {
            file = format!("{base}_{n}.png");
            n += 1;
        }
        if s.image.width() > 0 {
            s.image.save(dir.join(&file)).map_err(std::io::Error::other)?;
        }
        files.push(file);
    }
    let parts = sprites
        .iter()
        .zip(&files)
        .map(|(s, file)| {
            let part = &skel.parts[s.part];
            RigPart {
                name: part.name.clone(),
                file: file.clone(),
                parent: part.parent.map(|p| skel.parts[p].name.clone()),
                origin: s.origin,
                size: [s.image.width(), s.image.height()],
                pivot: s.pivot,
                pivot_in_image: [s.pivot[0] - s.origin[0] as f32, s.pivot[1] - s.origin[1] as f32],
            }
        })
        .collect();
    let json = serde_json::to_string_pretty(&RigFile { parts }).map_err(std::io::Error::other)?;
    std::fs::write(dir.join("rig.json"), json)
}

/// A part name made safe for a file name ("left arm" → "left_arm").
fn file_stem(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    if s.is_empty() { "part".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skeleton::{Joint, SkelPart, skeleton_masks};
    use image::Rgba;

    /// Torso block with an arm sticking out of its top-left, in two colors.
    fn figure() -> (RgbaImage, Mask, Skeleton) {
        let mut img = RgbaImage::from_pixel(120, 120, Rgba([255, 255, 255, 255]));
        let mut fg = Mask::new(120, 120);
        for y in 0..120 {
            for x in 0..120 {
                let torso = (40..80).contains(&x) && (20..100).contains(&y);
                let arm = (5..40).contains(&x) && (22..30).contains(&y);
                if torso || arm {
                    fg.set(x, y, true);
                    img.put_pixel(x, y, Rgba([(x * 2) as u8, (y * 2) as u8, 90, 255]));
                }
            }
        }
        let j = |name: &str, x: f32, y: f32| Joint { name: name.into(), pos: [x, y] };
        let skel = Skeleton {
            joints: vec![j("pelvis", 60.0, 95.0), j("neck", 60.0, 22.0), j("shoulder", 42.0, 26.0), j("hand", 6.0, 26.0)],
            parts: vec![
                SkelPart { name: "torso".into(), pivot: 0, tip: 1, parent: None, limit: None },
                SkelPart { name: "left arm".into(), pivot: 2, tip: 3, parent: Some(0), limit: None },
            ],
        };
        (img, fg, skel)
    }

    #[test]
    fn rest_pose_reassembles_the_drawing() {
        let (img, fg, skel) = figure();
        let sm = skeleton_masks(&fg, &skel);
        let sprites = build_sprites(&img, &fg, &skel, &sm.masks, 1.0);
        let mut out = RgbaImage::new(120, 120);
        for p in skel.draw_order() {
            let s = &sprites[p];
            for (x, y, px) in s.image.enumerate_pixels() {
                if px.0[3] > 0 {
                    out.put_pixel(x + s.origin[0] as u32, y + s.origin[1] as u32, *px);
                }
            }
        }
        for (x, y, px) in out.enumerate_pixels() {
            if fg.get(x, y) {
                assert_eq!(px, img.get_pixel(x, y), "figure pixel ({x},{y}) reproduced");
            } else {
                assert_eq!(px.0[3], 0, "background stays empty");
            }
        }
        assert_eq!(sprites[0].pivot, [60.0, 95.0]);
    }

    #[test]
    fn joint_disc_goes_to_parent_only() {
        let (_, fg, skel) = figure();
        let sm = skeleton_masks(&fg, &skel);
        // An arm pixel just outside the torso, right at the shoulder joint.
        let (x, y) = (38u32, 26u32);
        assert!(sm.masks[1].get(x, y) && !sm.masks[0].get(x, y));
        let with = sprite_masks(&fg, &skel, &sm.masks, 1.0);
        assert!(with[0].get(x, y), "torso keeps a disc around the shoulder");
        assert!(with[1].get(x, y), "arm still has it");
        assert_eq!(with[1], sm.masks[1], "child mask unchanged");
        let without = sprite_masks(&fg, &skel, &sm.masks, 0.0);
        assert!(!without[0].get(x, y));
    }

    #[test]
    fn neck_disc_is_neck_sized_not_head_sized() {
        // Big round head (r 40) on a 20 px wide torso; the neck joint at the seam.
        let mut fg = Mask::new(120, 200);
        for y in 0..200u32 {
            for x in 0..120u32 {
                let head = (x as f32 - 60.0).powi(2) + (y as f32 - 50.0).powi(2) <= 1600.0;
                if head || ((50..70).contains(&x) && (88..180).contains(&y)) {
                    fg.set(x, y, true);
                }
            }
        }
        let j = |name: &str, x: f32, y: f32| Joint { name: name.into(), pos: [x, y] };
        let skel = Skeleton {
            joints: vec![j("pelvis", 60.0, 175.0), j("neck", 60.0, 90.0), j("head top", 60.0, 12.0)],
            parts: vec![
                SkelPart { name: "torso".into(), pivot: 0, tip: 1, parent: None, limit: None },
                SkelPart { name: "head".into(), pivot: 1, tip: 2, parent: Some(0), limit: None },
            ],
        };
        let sm = skeleton_masks(&fg, &skel);
        let with = sprite_masks(&fg, &skel, &sm.masks, 1.0);
        assert!(with[0].get(60, 85), "torso keeps a little of the head right above the neck");
        assert!(!with[0].get(40, 70) && !with[0].get(60, 60), "but not a head-sized chunk");
    }

    #[test]
    fn writes_pngs_and_rig_json() {
        let (img, fg, skel) = figure();
        let sm = skeleton_masks(&fg, &skel);
        let sprites = build_sprites(&img, &fg, &skel, &sm.masks, 1.0);
        let dir = std::env::temp_dir().join(format!("anim_parts_test_{}", std::process::id()));
        write_parts(&dir, &sprites, &skel).unwrap();
        assert!(dir.join("torso.png").exists() && dir.join("left_arm.png").exists());
        let json = std::fs::read_to_string(dir.join("rig.json")).unwrap();
        assert!(json.contains("\"parent\": \"torso\""));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
