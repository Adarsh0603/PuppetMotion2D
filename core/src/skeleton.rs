//! Joint-based part assignment: the user places joints, each part is the bone
//! between two of them, and every foreground pixel goes to the nearest part.
//!
//! "Nearest" is measured to the part's capsule (bone line grown by the part's
//! thickness), not to the bare bone. Thickness is measured from the drawing,
//! so a wide torso keeps its own corners instead of losing them to the thin
//! arms attached there.

use crate::geom::RectI;
use crate::mask::Mask;

#[derive(Clone, Debug, PartialEq)]
pub struct Joint {
    pub name: String,
    pub pos: [f32; 2],
}

/// A body part spanning two joints. `pivot` is the end attached to the parent
/// part (the rotation center later on); `tip` is the free end.
#[derive(Clone, Debug, PartialEq)]
pub struct SkelPart {
    pub name: String,
    pub pivot: usize,
    pub tip: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Skeleton {
    pub joints: Vec<Joint>,
    pub parts: Vec<SkelPart>,
}

impl Skeleton {
    /// Front-facing stick person scaled into `bbox` (usually the foreground's
    /// bounding box). Left/right are as seen on screen. Stand-in until Step 4
    /// moves templates into JSON.
    pub fn person_front(bbox: RectI) -> Skeleton {
        let at = |u: f32, v: f32| [bbox.x as f32 + u * bbox.w as f32, bbox.y as f32 + v * bbox.h as f32];
        let joints = [
            ("head top", at(0.5, 0.03)),
            ("neck", at(0.5, 0.24)),
            ("pelvis", at(0.5, 0.62)),
            ("left shoulder", at(0.36, 0.30)),
            ("left hand", at(0.12, 0.56)),
            ("right shoulder", at(0.64, 0.30)),
            ("right hand", at(0.88, 0.56)),
            ("left hip", at(0.42, 0.66)),
            ("left foot", at(0.38, 0.97)),
            ("right hip", at(0.58, 0.66)),
            ("right foot", at(0.62, 0.97)),
        ];
        let parts = [
            ("head", 1, 0),
            ("torso", 2, 1),
            ("left arm", 3, 4),
            ("right arm", 5, 6),
            ("left leg", 7, 8),
            ("right leg", 9, 10),
        ];
        Skeleton {
            joints: joints.iter().map(|&(n, pos)| Joint { name: n.into(), pos }).collect(),
            parts: parts.iter().map(|&(n, pivot, tip)| SkelPart { name: n.into(), pivot, tip }).collect(),
        }
    }

    pub fn empty() -> Skeleton {
        Skeleton { joints: Vec::new(), parts: Vec::new() }
    }

    pub fn bone(&self, part: usize) -> ([f32; 2], [f32; 2]) {
        let p = &self.parts[part];
        (self.joints[p.pivot].pos, self.joints[p.tip].pos)
    }

    /// Add a joint with an automatic name ("joint 12"); returns its index.
    pub fn add_joint(&mut self, pos: [f32; 2]) -> usize {
        let name = unique_name("joint", self.joints.iter().map(|j| j.name.as_str()));
        self.joints.push(Joint { name, pos });
        self.joints.len() - 1
    }

    /// Add a bone from `pivot` (attachment end) to `tip`; returns the part index.
    pub fn add_part(&mut self, pivot: usize, tip: usize) -> usize {
        let name = unique_name("bone", self.parts.iter().map(|p| p.name.as_str()));
        self.parts.push(SkelPart { name, pivot, tip });
        self.parts.len() - 1
    }

    /// Remove a joint and every part attached to it.
    pub fn remove_joint(&mut self, joint: usize) {
        self.parts.retain(|p| p.pivot != joint && p.tip != joint);
        self.drop_joint(joint);
        self.prune_orphans();
    }

    /// Remove a part, and any joint no other part uses.
    pub fn remove_part(&mut self, part: usize) {
        self.parts.remove(part);
        self.prune_orphans();
    }

    /// Delete one joint and shift the joint indices stored in parts.
    fn drop_joint(&mut self, joint: usize) {
        self.joints.remove(joint);
        for p in &mut self.parts {
            if p.pivot > joint {
                p.pivot -= 1;
            }
            if p.tip > joint {
                p.tip -= 1;
            }
        }
    }

    fn prune_orphans(&mut self) {
        for j in (0..self.joints.len()).rev() {
            if !self.parts.iter().any(|p| p.pivot == j || p.tip == j) {
                self.drop_joint(j);
            }
        }
    }
}

/// "prefix N" with the smallest N (from 1) not already in `taken`.
fn unique_name<'a>(prefix: &str, taken: impl Iterator<Item = &'a str>) -> String {
    let taken: Vec<&str> = taken.collect();
    (1..).map(|n| format!("{prefix} {n}")).find(|name| !taken.contains(&name.as_str())).unwrap()
}

/// Approximate Euclidean distance from each pixel to the nearest pixel outside
/// `mask` (two-pass chamfer, weights 1 and √2). Off pixels are 0.
pub fn distance_transform(mask: &Mask) -> Vec<f32> {
    let (w, h) = (mask.w as usize, mask.h as usize);
    const D: f32 = std::f32::consts::SQRT_2;
    let mut d: Vec<f32> = mask.data.iter().map(|&v| if v != 0 { f32::INFINITY } else { 0.0 }).collect();
    // Pixels outside the image count as background.
    let get = |d: &[f32], x: isize, y: isize| {
        if x < 0 || y < 0 || x >= w as isize || y >= h as isize { 0.0 } else { d[y as usize * w + x as usize] }
    };
    for y in 0..h as isize {
        for x in 0..w as isize {
            let i = y as usize * w + x as usize;
            if d[i] == 0.0 {
                continue;
            }
            let best = (get(&d, x - 1, y) + 1.0)
                .min(get(&d, x, y - 1) + 1.0)
                .min(get(&d, x - 1, y - 1) + D)
                .min(get(&d, x + 1, y - 1) + D);
            d[i] = d[i].min(best);
        }
    }
    for y in (0..h as isize).rev() {
        for x in (0..w as isize).rev() {
            let i = y as usize * w + x as usize;
            if d[i] == 0.0 {
                continue;
            }
            let best = (get(&d, x + 1, y) + 1.0)
                .min(get(&d, x, y + 1) + 1.0)
                .min(get(&d, x + 1, y + 1) + D)
                .min(get(&d, x - 1, y + 1) + D);
            d[i] = d[i].min(best);
        }
    }
    d
}

pub fn dist_to_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ap = [p[0] - a[0], p[1] - a[1]];
    let len2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if len2 > 0.0 { ((ap[0] * ab[0] + ap[1] * ab[1]) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let d = [ap[0] - ab[0] * t, ap[1] - ab[1] * t];
    (d[0] * d[0] + d[1] * d[1]).sqrt()
}

/// Part thickness: median distance-to-edge sampled along the bone. The median
/// ignores the ends, where the bone runs into a neighbouring (thicker) part.
fn bone_radius(dt: &[f32], w: u32, h: u32, a: [f32; 2], b: [f32; 2]) -> f32 {
    let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    let n = (len.ceil() as usize).max(1);
    let mut samples: Vec<f32> = (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let (x, y) = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
            if x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 {
                0.0
            } else {
                dt[y as usize * w as usize + x as usize]
            }
        })
        .collect();
    samples.sort_by(f32::total_cmp);
    samples[samples.len() / 2]
}

pub struct SkeletonMasks {
    /// One mask per part, in `Skeleton::parts` order. Exclusive and covering
    /// every foreground pixel.
    pub masks: Vec<Mask>,
    /// Measured thickness (half-width, px) of each part.
    pub radii: Vec<f32>,
}

pub fn skeleton_masks(fg: &Mask, skel: &Skeleton) -> SkeletonMasks {
    let dt = distance_transform(fg);
    let bones: Vec<_> = (0..skel.parts.len()).map(|i| skel.bone(i)).collect();
    let radii: Vec<f32> = bones.iter().map(|&(a, b)| bone_radius(&dt, fg.w, fg.h, a, b)).collect();
    let mut masks = vec![Mask::new(fg.w, fg.h); skel.parts.len()];
    if bones.is_empty() {
        return SkeletonMasks { masks, radii };
    }
    for y in 0..fg.h {
        for x in 0..fg.w {
            if !fg.get(x, y) {
                continue;
            }
            let p = [x as f32 + 0.5, y as f32 + 0.5];
            let cost = |i: usize| dist_to_segment(p, bones[i].0, bones[i].1) - radii[i];
            let best = (0..bones.len()).min_by(|&a, &b| cost(a).total_cmp(&cost(b))).unwrap();
            masks[best].set(x, y, true);
        }
    }
    SkeletonMasks { masks, radii }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(m: &mut Mask, x0: u32, y0: u32, x1: u32, y1: u32) {
        for y in y0..y1 {
            for x in x0..x1 {
                m.set(x, y, true);
            }
        }
    }

    #[test]
    fn distance_transform_of_block() {
        let mut m = Mask::new(21, 21);
        fill(&mut m, 5, 5, 16, 16);
        let d = distance_transform(&m);
        assert_eq!(d[0], 0.0);
        assert_eq!(d[5 * 21 + 5], 1.0, "edge pixel is 1 from outside");
        assert!((d[10 * 21 + 10] - 6.0).abs() < 0.5, "center is ~6 from the edge");
    }

    /// Wide torso (x 40..80, y 20..100) with a thin arm out of its top-left corner.
    fn torso_with_arm() -> (Mask, Skeleton) {
        let mut fg = Mask::new(120, 120);
        fill(&mut fg, 40, 20, 80, 100);
        fill(&mut fg, 5, 22, 40, 30);
        let j = |name: &str, x: f32, y: f32| Joint { name: name.into(), pos: [x, y] };
        let skel = Skeleton {
            joints: vec![j("pelvis", 60.0, 95.0), j("neck", 60.0, 22.0), j("shoulder", 42.0, 26.0), j("hand", 6.0, 26.0)],
            parts: vec![
                SkelPart { name: "torso".into(), pivot: 0, tip: 1 },
                SkelPart { name: "arm".into(), pivot: 2, tip: 3 },
            ],
        };
        (fg, skel)
    }

    #[test]
    fn every_pixel_assigned_once() {
        let (fg, skel) = torso_with_arm();
        let sm = skeleton_masks(&fg, &skel);
        let total: usize = sm.masks.iter().map(Mask::count).sum();
        assert_eq!(total, fg.count());
        assert_eq!(sm.masks[0].and(&sm.masks[1]).count(), 0);
    }

    #[test]
    fn thick_torso_keeps_its_corner() {
        let (fg, skel) = torso_with_arm();
        let sm = skeleton_masks(&fg, &skel);
        assert!(sm.radii[0] > 15.0 && sm.radii[1] < 6.0, "radii {:?}", sm.radii);
        assert!(sm.masks[1].get(20, 26), "arm pixels go to the arm");
        // Inside the torso just below the shoulder: nearer the arm bone than the
        // torso's center line, but well inside the torso's thickness.
        assert!(sm.masks[0].get(44, 40), "torso corner stays torso");
        assert!(sm.masks[0].get(50, 25));
    }

    #[test]
    fn grow_and_remove_bones() {
        let mut s = Skeleton::empty();
        let a = s.add_joint([0.0, 0.0]);
        let b = s.add_joint([10.0, 0.0]);
        s.add_part(a, b);
        let c = s.add_joint([20.0, 0.0]);
        s.add_part(b, c); // grown from b
        assert_eq!(s.parts[1].name, "bone 2");
        assert_eq!(s.joints[2].name, "joint 3");

        // Removing the shared joint takes both bones and leaves nothing behind.
        let mut t = s.clone();
        t.remove_joint(b);
        assert!(t.parts.is_empty() && t.joints.is_empty());

        // Removing the first bone drops joint a, and the second bone's indices shift.
        s.remove_part(0);
        assert_eq!(s.joints.len(), 2);
        assert_eq!(s.bone(0), ([10.0, 0.0], [20.0, 0.0]));
        assert_eq!(s.add_part(0, 1), 1);
        assert_eq!(s.parts[1].name, "bone 1", "freed names are reused");
    }

    #[test]
    fn template_fits_bbox() {
        let s = Skeleton::person_front(RectI::new(100, 50, 200, 400));
        assert_eq!(s.parts.len(), 6);
        assert_eq!(s.joints[1].pos, [200.0, 146.0]);
    }
}
