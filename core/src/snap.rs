//! Moving joints onto the drawing: a fitted template is only stretched over
//! the character's box, so elbows land on a limb's edge and hands in the
//! background. Snapping puts each joint in the middle of the limb it's on.

use crate::mask::Mask;
use crate::skeleton::Skeleton;

/// Move each joint to the middle of the drawing across its bone, going no
/// further than `reach` pixels. A joint in the background first moves to the
/// nearest drawn pixel. Joints stay put where three or more bones meet (hips,
/// shoulders: "across the bone" means nothing there), and where the drawing
/// across them is wider than `reach` both ways (the body, not a limb).
/// Returns how many joints moved.
pub fn snap_joints(skel: &mut Skeleton, fg: &Mask, reach: f32) -> usize {
    let mut moved = 0;
    for j in 0..skel.joints.len() {
        let Some(dir) = bone_direction(skel, j) else { continue };
        let start = skel.joints[j].pos;
        let Some(on) = (if inside(fg, start) { Some(start) } else { nearest_inside(fg, start, reach) }) else { continue };
        let across = [-dir[1], dir[0]];
        // Wider than `reach` across means the body, not a limb: leave the joint
        // alone rather than park it on the body's edge.
        let (Some(a), Some(b)) = (edge(fg, on, across, reach), edge(fg, on, [-across[0], -across[1]], reach)) else { continue };
        let p = [on[0] + across[0] * (a - b) / 2.0, on[1] + across[1] * (a - b) / 2.0];
        let d = ((p[0] - start[0]).powi(2) + (p[1] - start[1]).powi(2)).sqrt();
        if d > 0.5 && d <= reach {
            skel.joints[j].pos = p;
            moved += 1;
        }
    }
    moved
}

/// The unit direction of the one or two bones at joint `j` (two are averaged,
/// whichever way they point), or None for a lone joint or a branching one.
fn bone_direction(skel: &Skeleton, j: usize) -> Option<[f32; 2]> {
    let mut sum = [0.0f32; 2];
    let mut count = 0;
    for part in skel.parts.iter().filter(|p| p.pivot == j || p.tip == j) {
        let (a, b) = (skel.joints.get(part.pivot)?.pos, skel.joints.get(part.tip)?.pos);
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        if len < 1e-3 {
            continue;
        }
        let mut d = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
        if d[0] * sum[0] + d[1] * sum[1] < 0.0 {
            d = [-d[0], -d[1]];
        }
        sum = [sum[0] + d[0], sum[1] + d[1]];
        count += 1;
    }
    let len = (sum[0] * sum[0] + sum[1] * sum[1]).sqrt();
    (matches!(count, 1 | 2) && len > 1e-3).then(|| [sum[0] / len, sum[1] / len])
}

fn inside(fg: &Mask, p: [f32; 2]) -> bool {
    let (x, y) = (p[0].floor(), p[1].floor());
    x >= 0.0 && y >= 0.0 && (x as u32) < fg.w && (y as u32) < fg.h && fg.get(x as u32, y as u32)
}

/// The middle of the drawn pixel nearest to `p`, within `reach`.
fn nearest_inside(fg: &Mask, p: [f32; 2], reach: f32) -> Option<[f32; 2]> {
    let r = reach.ceil() as i64;
    let (cx, cy) = (p[0].floor() as i64, p[1].floor() as i64);
    let mut best: Option<([f32; 2], f32)> = None;
    for y in (cy - r).max(0)..=(cy + r).min(fg.h as i64 - 1) {
        for x in (cx - r).max(0)..=(cx + r).min(fg.w as i64 - 1) {
            if !fg.get(x as u32, y as u32) {
                continue;
            }
            let q = [x as f32 + 0.5, y as f32 + 0.5];
            let d = (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2);
            if d <= reach * reach && best.is_none_or(|(_, bd)| d < bd) {
                best = Some((q, d));
            }
        }
    }
    best.map(|(q, _)| q)
}

/// How far from `p` along unit `dir` the drawing ends, if within `reach`.
fn edge(fg: &Mask, p: [f32; 2], dir: [f32; 2], reach: f32) -> Option<f32> {
    const STEP: f32 = 0.5;
    let mut t = 0.0;
    while t <= reach {
        if !inside(fg, [p[0] + dir[0] * t, p[1] + dir[1] * t]) {
            return Some(t);
        }
        t += STEP;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A horizontal bar of drawing, rows 40..60, on a 200 × 100 image.
    fn bar() -> Mask {
        let mut m = Mask::new(200, 100);
        for y in 40..60 {
            for x in 20..180 {
                m.set(x, y, true);
            }
        }
        m
    }

    /// Two bones along the bar, with their middle joint at `mid`.
    fn arm(mid: [f32; 2]) -> Skeleton {
        let mut s = Skeleton::empty();
        let a = s.add_joint([30.0, 50.0]);
        let b = s.add_joint(mid);
        let c = s.add_joint([170.0, 50.0]);
        s.add_part(a, b);
        s.add_part(b, c);
        s
    }

    #[test]
    fn a_joint_on_the_edge_moves_to_the_middle() {
        let mut s = arm([100.0, 42.0]);
        assert!(snap_joints(&mut s, &bar(), 20.0) >= 1);
        let p = s.joints[1].pos;
        assert!((p[1] - 50.0).abs() < 1.5 && (p[0] - 100.0).abs() < 2.0, "{p:?}");
    }

    #[test]
    fn a_joint_in_the_background_comes_onto_the_drawing() {
        let mut s = arm([100.0, 30.0]);
        snap_joints(&mut s, &bar(), 20.0);
        let p = s.joints[1].pos;
        assert!((p[1] - 50.0).abs() < 1.5, "{p:?}");
    }

    #[test]
    fn far_joints_and_branching_joints_stay() {
        let mut far = arm([100.0, 5.0]);
        snap_joints(&mut far, &bar(), 20.0);
        assert_eq!(far.joints[1].pos, [100.0, 5.0], "out of reach");

        let mut branch = arm([100.0, 42.0]);
        let d = branch.add_joint([100.0, 90.0]);
        branch.add_part(1, d);
        let before = branch.joints[1].pos;
        snap_joints(&mut branch, &bar(), 20.0);
        assert_eq!(branch.joints[1].pos, before, "three bones meet here");
    }
}
