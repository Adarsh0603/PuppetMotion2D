//! Posing: each part rotates around its pivot, and children follow their
//! parent (forward kinematics). Everything is expressed in rest-image
//! coordinates: a part's transform maps its rest-pose pixels to their posed
//! positions, so drawing a sprite is just transforming its corners.

use crate::skeleton::{Skeleton, dist_to_segment};
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

    /// The transform that undoes this one (identity if it can't be undone,
    /// which rotations and translations never are).
    pub fn inverse(&self) -> Affine2 {
        let [a, b, c, d, tx, ty] = self.m;
        let det = a * d - b * c;
        if det.abs() < 1e-12 {
            return Affine2::IDENTITY;
        }
        let (ia, ib, ic, id) = (d / det, -b / det, -c / det, a / det);
        Affine2 { m: [ia, ib, ic, id, -(ia * tx + ib * ty), -(ic * tx + id * ty)] }
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

/// `angle` kept inside the part's rotation limit, if it has one.
pub fn clamp_to_limit(skel: &Skeleton, part: usize, angle: f32) -> f32 {
    match skel.parts[part].limit {
        Some([a, b]) => angle.clamp(a.min(b), a.max(b)),
        None => angle,
    }
}

/// How far `angle` is outside the part's limit (0 if inside).
fn limit_violation(skel: &Skeleton, part: usize, angle: f32) -> f32 {
    (angle - clamp_to_limit(skel, part, angle)).abs()
}

/// Rotate `part` so its bone points at `target`, within its limit.
pub fn aim_part(skel: &Skeleton, pose: &mut Pose, part: usize, target: [f32; 2]) {
    let t = world_transforms(skel, pose);
    let angle = angle_to_point(skel, pose, &t, part, target);
    pose.set_angle(part, clamp_to_limit(skel, part, angle));
}

/// The part above `part` in a two-bone chain: its parent, if `part` starts
/// exactly where the parent ends (a forearm and its upper arm share the
/// elbow) and the parent is not the root. The root is the body itself, so a
/// head on top of the torso is just rotated at the neck rather than bending
/// the whole body to reach the mouse.
pub fn ik_parent(skel: &Skeleton, part: usize) -> Option<usize> {
    let p = skel.parts[part].parent?;
    skel.parts[p].parent?;
    (skel.parts[p].tip == skel.parts[part].pivot).then_some(p)
}

/// Move the end of `part` (a hand, a foot) toward `target` by bending the
/// two-bone chain it ends (upper arm + forearm). Out-of-reach targets leave
/// the limb straight, pointing at them. Of the two ways the middle joint can
/// bend, the one that breaks the rotation limits less wins; if both are
/// equally fine the current bend direction is kept so the elbow doesn't flip.
/// A part with no chain above it is simply aimed at the target.
pub fn solve_two_bone(skel: &Skeleton, pose: &mut Pose, part: usize, target: [f32; 2]) {
    let Some(upper) = ik_parent(skel, part) else {
        aim_part(skel, pose, part, target);
        return;
    };
    let t = world_transforms(skel, pose);
    let (u_pivot, u_tip) = skel.bone(upper);
    let (_, l_tip) = skel.bone(part);
    let base = parent_transform(skel, pose, &t, upper).apply(u_pivot);
    let l1 = dist(u_pivot, u_tip);
    let l2 = dist(u_tip, l_tip);
    let to_target = [target[0] - base[0], target[1] - base[1]];
    let reach = (to_target[0].hypot(to_target[1])).max(1e-6);
    let dir = [to_target[0] / reach, to_target[1] / reach];
    let d = reach.clamp((l1 - l2).abs() + 1e-3, l1 + l2 - 1e-3);
    let bend = ((l1 * l1 + d * d - l2 * l2) / (2.0 * l1 * d)).clamp(-1.0, 1.0).acos();

    let elbow_for = |side: f32| add(base, scale(rotate(dir, side * bend), l1));

    // Preferred bend side. If the limb is already bent, keep that side (which
    // side of the base→hand line the elbow is on). If it is straight, as
    // drawn limbs usually are, bend away from the body: pick the elbow
    // farther from the bone this limb hangs from.
    let elbow_now = t[upper].apply(u_tip);
    let hand_now = t[part].apply(l_tip);
    let (a, b) = (sub(hand_now, base), sub(elbow_now, base));
    let sin_now = cross(a, b) / (a[0].hypot(a[1]) * b[0].hypot(b[1])).max(1e-6);
    let preferred = if sin_now.abs() > 0.05 {
        sin_now.signum()
    } else if let Some(body) = skel.parts[upper].parent {
        let (p, q) = skel.bone(body);
        let (p, q) = (t[body].apply(p), t[body].apply(q));
        let away = |side: f32| dist_to_segment(elbow_for(side), p, q);
        if away(1.0) >= away(-1.0) { 1.0 } else { -1.0 }
    } else {
        1.0
    };

    let mut best: Option<(f32, Pose)> = None;
    for side in [preferred, -preferred] {
        let elbow = elbow_for(side);
        let mut p = pose.clone();
        let t1 = world_transforms(skel, &p);
        let a_upper = angle_to_point(skel, &p, &t1, upper, elbow);
        p.set_angle(upper, a_upper);
        let t2 = world_transforms(skel, &p);
        let a_lower = angle_to_point(skel, &p, &t2, part, target);
        let violation = limit_violation(skel, upper, a_upper) + limit_violation(skel, part, a_lower);
        // Apply the limits: clamp the upper part, then re-aim the lower one.
        p.set_angle(upper, clamp_to_limit(skel, upper, a_upper));
        aim_part(skel, &mut p, part, target);
        if best.as_ref().is_none_or(|(v, _)| violation < *v - 1e-4) {
            best = Some((violation, p));
        }
    }
    *pose = best.unwrap().1;
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

fn sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn add(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] + b[0], a[1] + b[1]]
}

fn scale(v: [f32; 2], s: f32) -> [f32; 2] {
    [v[0] * s, v[1] * s]
}

fn cross(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[1] - a[1] * b[0]
}

fn rotate(v: [f32; 2], angle: f32) -> [f32; 2] {
    let (s, c) = angle.sin_cos();
    [v[0] * c - v[1] * s, v[0] * s + v[1] * c]
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
    fn inverse_undoes_the_transform() {
        let t = Affine2::translate([5.0, -3.0]).mul(&Affine2::rotate_about([10.0, 10.0], 0.7));
        let p = [12.0, 34.0];
        assert!(close(t.inverse().apply(t.apply(p)), p));
        assert!(close(t.apply(t.inverse().apply(p)), p));
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

    fn part(s: &Skeleton, name: &str) -> usize {
        s.parts.iter().position(|p| p.name == name).unwrap()
    }

    fn hand_and_elbow(s: &Skeleton, pose: &Pose) -> ([f32; 2], [f32; 2]) {
        let t = world_transforms(s, pose);
        let f = part(s, "left forearm");
        (posed_joint(s, &t, s.parts[f].tip), posed_joint(s, &t, s.parts[f].pivot))
    }

    #[test]
    fn ik_reaches_and_keeps_bend_side() {
        let s = Skeleton::person_front(RectI::new(0, 0, 200, 400));
        let forearm = part(&s, "left forearm");
        let shoulder = s.joints[s.parts[part(&s, "left upper arm")].pivot].pos;
        let mut pose = Pose::rest(s.parts.len());

        let target = [shoulder[0] - 40.0, shoulder[1] - 60.0]; // hand up and in
        solve_two_bone(&s, &mut pose, forearm, target);
        let (hand, elbow) = hand_and_elbow(&s, &pose);
        assert!(dist(hand, target) < 0.5, "hand at {hand:?}, wanted {target:?}");
        let side = cross(sub(hand, shoulder), sub(elbow, shoulder)).signum();

        // A nearby target keeps the elbow on the same side.
        let target2 = [target[0] + 5.0, target[1] + 3.0];
        solve_two_bone(&s, &mut pose, forearm, target2);
        let (hand2, elbow2) = hand_and_elbow(&s, &pose);
        assert!(dist(hand2, target2) < 0.5);
        assert_eq!(cross(sub(hand2, shoulder), sub(elbow2, shoulder)).signum(), side, "elbow didn't flip");
    }

    #[test]
    fn head_drag_turns_only_the_head() {
        let s = Skeleton::person_front(RectI::new(0, 0, 200, 400));
        let (head, torso) = (part(&s, "head"), part(&s, "torso"));
        assert_eq!(ik_parent(&s, head), None, "head + torso is not an IK limb");
        assert_eq!(ik_parent(&s, part(&s, "left forearm")), Some(part(&s, "left upper arm")));
        let neck = s.joints[s.parts[head].pivot].pos;
        let mut pose = Pose::rest(s.parts.len());
        solve_two_bone(&s, &mut pose, head, [neck[0] + 40.0, neck[1] - 40.0]);
        assert_eq!(pose.angle(torso), 0.0, "body stays put");
        assert!(pose.angle(head) > 0.1, "head turned toward the mouse");
    }

    #[test]
    fn straight_limb_bends_away_from_the_body() {
        // The template's arms are drawn straight; pulling the left hand up
        // toward the face should swing the elbow out to the left, not across the chest.
        let s = Skeleton::person_front(RectI::new(0, 0, 200, 400));
        let shoulder = s.joints[s.parts[part(&s, "left upper arm")].pivot].pos;
        let mut pose = Pose::rest(s.parts.len());
        solve_two_bone(&s, &mut pose, part(&s, "left forearm"), [shoulder[0] + 15.0, shoulder[1] - 50.0]);
        let (_, elbow) = hand_and_elbow(&s, &pose);
        assert!(elbow[0] < shoulder[0], "elbow at {elbow:?} should be left of the shoulder {shoulder:?}");
    }

    #[test]
    fn ik_out_of_reach_stretches_straight() {
        let s = Skeleton::person_front(RectI::new(0, 0, 200, 400));
        let forearm = part(&s, "left forearm");
        let upper = part(&s, "left upper arm");
        let (sh, el) = s.bone(upper);
        let (_, hd) = s.bone(forearm);
        let full = dist(sh, el) + dist(el, hd);
        let mut pose = Pose::rest(s.parts.len());
        let target = [sh[0] - 500.0, sh[1]];
        solve_two_bone(&s, &mut pose, forearm, target);
        let (hand, elbow) = hand_and_elbow(&s, &pose);
        assert!((dist(sh, hand) - full).abs() < 0.5, "arm fully extended");
        let (a, b) = (sub(hand, sh), sub(elbow, sh));
        let sin = cross(a, b) / (dist(hand, sh) * dist(elbow, sh));
        assert!(sin.abs() < 0.01, "and straight (within half a degree)");
        assert!((hand[1] - sh[1]).abs() < 0.5 && hand[0] < sh[0], "pointing at the target");
    }

    #[test]
    fn limits_clamp_and_steer_the_bend() {
        let mut s = Skeleton::person_front(RectI::new(0, 0, 200, 400));
        let forearm = part(&s, "left forearm");
        let head = part(&s, "head");
        assert!(clamp_to_limit(&s, head, 2.0) < 0.71, "head limited to ±40°");

        // Let the forearm bend only one way, then ask for a bent arm.
        s.parts[forearm].limit = Some([0.0, 2.6]);
        let shoulder = s.joints[s.parts[part(&s, "left upper arm")].pivot].pos;
        let mut pose = Pose::rest(s.parts.len());
        for target in [[shoulder[0] - 30.0, shoulder[1] + 30.0], [shoulder[0] + 10.0, shoulder[1] - 50.0]] {
            solve_two_bone(&s, &mut pose, forearm, target);
            let a = pose.angle(forearm);
            assert!((-1e-4..=2.6001).contains(&a), "forearm angle {a} within its limit");
            let (hand, _) = hand_and_elbow(&s, &pose);
            assert!(dist(hand, target) < 0.5, "still reaches by bending the allowed way");
        }
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
