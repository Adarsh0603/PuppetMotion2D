//! Posing: each part rotates around its pivot, and children follow their
//! parent (forward kinematics). Everything is expressed in rest-image
//! coordinates: a part's transform maps its rest-pose pixels to their posed
//! positions, so drawing a sprite is just transforming its corners.

use crate::skeleton::Skeleton;
use serde::{Deserialize, Serialize};

/// 2D affine transform: `x' = a·x + b·y + tx`, `y' = c·x + d·y + ty`,
/// stored as `[a, b, c, d, tx, ty]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine2 {
    pub m: [f32; 6],
}

impl Affine2 {
    pub const IDENTITY: Affine2 = Affine2 { m: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] };

    pub fn translate(t: [f32; 2]) -> Affine2 {
        Affine2 { m: [1.0, 0.0, 0.0, 1.0, t[0], t[1]] }
    }

    /// Rotation by `angle` radians around point `p`. Image y points down, so a
    /// positive angle turns clockwise on screen.
    pub fn rotate_about(p: [f32; 2], angle: f32) -> Affine2 {
        let (s, c) = angle.sin_cos();
        Affine2 { m: [c, -s, s, c, p[0] - c * p[0] + s * p[1], p[1] - s * p[0] - c * p[1]] }
    }

    /// `self ∘ other`: apply `other` first, then `self`.
    pub fn mul(&self, other: &Affine2) -> Affine2 {
        let [a, b, c, d, tx, ty] = self.m;
        let [e, f, g, h, ux, uy] = other.m;
        Affine2 { m: [a * e + b * g, a * f + b * h, c * e + d * g, c * f + d * h, a * ux + b * uy + tx, c * ux + d * uy + ty] }
    }

    pub fn apply(&self, p: [f32; 2]) -> [f32; 2] {
        let [a, b, c, d, tx, ty] = self.m;
        [a * p[0] + b * p[1] + tx, c * p[0] + d * p[1] + ty]
    }

    /// Transform a direction (ignores translation).
    pub fn apply_vec(&self, v: [f32; 2]) -> [f32; 2] {
        let [a, b, c, d, ..] = self.m;
        [a * v[0] + b * v[1], c * v[0] + d * v[1]]
    }
}

/// A pose: one rotation per part (radians, relative to its parent; 0 = as
/// drawn) plus an offset that moves the whole figure.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Pose {
    pub angles: Vec<f32>,
    pub offset: [f32; 2],
}

impl Pose {
    pub fn rest(parts: usize) -> Pose {
        Pose { angles: vec![0.0; parts], offset: [0.0, 0.0] }
    }

    /// Angle of `part`, 0 if the pose has none for it.
    pub fn angle(&self, part: usize) -> f32 {
        self.angles.get(part).copied().unwrap_or(0.0)
    }

    pub fn set_angle(&mut self, part: usize, angle: f32) {
        if self.angles.len() <= part {
            self.angles.resize(part + 1, 0.0);
        }
        self.angles[part] = angle;
    }
}

/// Transform of each part's rest pixels to posed positions, in part order.
pub fn world_transforms(skel: &Skeleton, pose: &Pose) -> Vec<Affine2> {
    let mut t = vec![Affine2::IDENTITY; skel.parts.len()];
    for p in skel.draw_order() {
        t[p] = parent_transform(skel, pose, &t, p).mul(&local_transform(skel, pose, p));
    }
    t
}

fn local_transform(skel: &Skeleton, pose: &Pose, part: usize) -> Affine2 {
    Affine2::rotate_about(skel.joints[skel.parts[part].pivot].pos, pose.angle(part))
}

/// The transform a part inherits: its parent's, or the figure offset for a root.
fn parent_transform(skel: &Skeleton, pose: &Pose, transforms: &[Affine2], part: usize) -> Affine2 {
    match skel.parts[part].parent {
        Some(q) if q < transforms.len() => transforms[q],
        _ => Affine2::translate(pose.offset),
    }
}

/// Posed position of a joint: it moves with the part it is the tip of, else
/// the part it is the pivot of; a joint no part uses stays put.
pub fn posed_joint(skel: &Skeleton, transforms: &[Affine2], joint: usize) -> [f32; 2] {
    let pos = skel.joints[joint].pos;
    let owner = skel.parts.iter().position(|p| p.tip == joint).or_else(|| skel.parts.iter().position(|p| p.pivot == joint));
    match owner {
        Some(i) => transforms[i].apply(pos),
        None => pos,
    }
}

/// The angle for `part` that points its bone from its posed pivot toward
/// `target`, keeping its parent where it is. Used when dragging a bone tip.
pub fn angle_to_point(skel: &Skeleton, pose: &Pose, transforms: &[Affine2], part: usize, target: [f32; 2]) -> f32 {
    let parent = parent_transform(skel, pose, transforms, part);
    let (pivot, tip) = skel.bone(part);
    let pivot_w = parent.apply(pivot);
    let rest_dir = parent.apply_vec([tip[0] - pivot[0], tip[1] - pivot[1]]);
    let want = (target[1] - pivot_w[1]).atan2(target[0] - pivot_w[0]);
    let have = rest_dir[1].atan2(rest_dir[0]);
    wrap_angle(want - have)
}

/// Angle in (-π, π].
fn wrap_angle(a: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let a = a.rem_euclid(TAU);
    if a > PI { a - TAU } else { a }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::RectI;
    use std::f32::consts::FRAC_PI_2;

    fn close(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
    }

    #[test]
    fn rotate_about_keeps_pivot_and_turns_clockwise() {
        let r = Affine2::rotate_about([10.0, 10.0], FRAC_PI_2);
        assert!(close(r.apply([10.0, 10.0]), [10.0, 10.0]));
        // A point to the right of the pivot ends up below it (y down = clockwise).
        assert!(close(r.apply([20.0, 10.0]), [10.0, 20.0]));
        let back = r.mul(&Affine2::rotate_about([10.0, 10.0], -FRAC_PI_2));
        assert!(close(back.apply([3.0, 7.0]), [3.0, 7.0]));
    }

    #[test]
    fn children_follow_parent() {
        let s = Skeleton::person_front(RectI::new(0, 0, 100, 200));
        let torso = 1;
        let pelvis = s.joints[s.parts[torso].pivot].pos;
        let hand = s.parts[2].tip;
        let rest_hand = s.joints[hand].pos;

        let mut pose = Pose::rest(s.parts.len());
        pose.set_angle(torso, FRAC_PI_2);
        let t = world_transforms(&s, &pose);
        let expected = Affine2::rotate_about(pelvis, FRAC_PI_2).apply(rest_hand);
        assert!(close(posed_joint(&s, &t, hand), expected), "hand swings with the torso");
        assert!(close(posed_joint(&s, &t, s.parts[torso].pivot), pelvis), "pivot stays put");

        // Offset moves everything.
        pose.offset = [5.0, -3.0];
        let t2 = world_transforms(&s, &pose);
        let moved = posed_joint(&s, &t2, hand);
        assert!(close(moved, [expected[0] + 5.0, expected[1] - 3.0]));
    }

    #[test]
    fn angle_to_point_aims_the_bone() {
        let s = Skeleton::person_front(RectI::new(0, 0, 100, 200));
        let mut pose = Pose::rest(s.parts.len());
        pose.set_angle(1, 0.4); // tilt the torso so the arm's parent is rotated too
        let arm = 3;
        let t = world_transforms(&s, &pose);
        let pivot_w = posed_joint(&s, &t, s.parts[arm].pivot);
        let target = [pivot_w[0] + 30.0, pivot_w[1] - 40.0];
        let angle = angle_to_point(&s, &pose, &t, arm, target);
        pose.set_angle(arm, angle);
        let t = world_transforms(&s, &pose);
        let tip = posed_joint(&s, &t, s.parts[arm].tip);
        let (d1, d2) = ([tip[0] - pivot_w[0], tip[1] - pivot_w[1]], [30.0f32, -40.0f32]);
        let cross = d1[0] * d2[1] - d1[1] * d2[0];
        let dot = d1[0] * d2[0] + d1[1] * d2[1];
        assert!(cross.abs() < 1e-2 * dot.abs() && dot > 0.0, "tip points at the target");
    }
}
