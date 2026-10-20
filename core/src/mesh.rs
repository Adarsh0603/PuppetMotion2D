//! Part meshes: each part's cut-out image covered by small triangles, so the
//! drawing can bend instead of only turning as rigid pieces.
//!
//! A part's mesh is a grid over its sprite, keeping only the cells that hold
//! some of its pixels. The grid lines come from one lattice shared by the
//! whole drawing, so where two parts meet their mesh points sit at the same
//! spots; when both follow the same blend of bones there, the seam can't open.
//!
//! Every mesh point has bone weights. Posing moves a point by the weighted
//! mix of its bones' transforms (linear blend skinning). For now each point
//! follows only its own part's bone, which looks exactly like moving the
//! sprite as one piece; smooth weights come next.

use crate::geom::RectI;
use crate::rig::Affine2;
use crate::sprites::Sprite;

/// Grid cells are about this fraction of the character's longer side…
const CELLS_PER_FIGURE: f32 = 160.0;
/// …but never smaller than this (px), so huge drawings don't get huge meshes.
const MIN_CELL: f32 = 4.0;

#[derive(Clone, Debug, PartialEq)]
pub struct PartMesh {
    pub part: usize,
    /// Points in the drawing's coordinates, as drawn (rest pose).
    pub points: Vec<[f32; 2]>,
    /// Where each point is on the part's sprite image, 0..1 across and down.
    pub uvs: Vec<[f32; 2]>,
    pub triangles: Vec<[u32; 3]>,
    /// Each point's bones and how much each one moves it (adding up to 1).
    pub weights: Vec<Vec<(usize, f32)>>,
}

/// Grid cell size (px) for a character whose bounding box is `figure`.
pub fn cell_size(figure: RectI) -> f32 {
    (figure.w.max(figure.h) as f32 / CELLS_PER_FIGURE).max(MIN_CELL)
}

/// One mesh per sprite (in the same order), on a grid of `cell` px. A part
/// with no pixels gets an empty mesh.
pub fn build_meshes(sprites: &[Sprite], cell: f32) -> Vec<PartMesh> {
    sprites.iter().map(|s| part_mesh(s, cell)).collect()
}

fn part_mesh(s: &Sprite, cell: f32) -> PartMesh {
    let (w, h) = (s.image.width(), s.image.height());
    let mut mesh = PartMesh { part: s.part, points: Vec::new(), uvs: Vec::new(), triangles: Vec::new(), weights: Vec::new() };
    if w == 0 || h == 0 {
        return mesh;
    }
    // Grid lines across the sprite (its own pixels), on the shared lattice,
    // plus the sprite's edges so the mesh never reaches past its image.
    let lines = |origin: i32, len: u32| -> Vec<f32> {
        let (start, end) = (origin as f32, origin as f32 + len as f32);
        let mut v = vec![0.0];
        let mut k = (start / cell).floor() + 1.0;
        while k * cell < end {
            let local = k * cell - start;
            if local > 0.5 && local < len as f32 - 0.5 {
                v.push(local);
            }
            k += 1.0;
        }
        v.push(len as f32);
        v
    };
    let (xs, ys) = (lines(s.origin[0], w), lines(s.origin[1], h));
    // A cell is kept if any of its pixels is visible.
    let filled = |x0: f32, x1: f32, y0: f32, y1: f32| {
        let (px0, px1) = (x0.floor() as u32, (x1.ceil() as u32).min(w));
        let (py0, py1) = (y0.floor() as u32, (y1.ceil() as u32).min(h));
        (py0..py1).any(|y| (px0..px1).any(|x| s.image.get_pixel(x, y).0[3] > 0))
    };
    // Points are shared between neighbouring cells: one per grid crossing used.
    let mut index = vec![u32::MAX; xs.len() * ys.len()];
    let mut point = |i: usize, j: usize, mesh: &mut PartMesh| -> u32 {
        let k = j * xs.len() + i;
        if index[k] == u32::MAX {
            index[k] = mesh.points.len() as u32;
            mesh.points.push([s.origin[0] as f32 + xs[i], s.origin[1] as f32 + ys[j]]);
            mesh.uvs.push([xs[i] / w as f32, ys[j] / h as f32]);
            mesh.weights.push(vec![(s.part, 1.0)]);
        }
        index[k]
    };
    for j in 0..ys.len() - 1 {
        for i in 0..xs.len() - 1 {
            if !filled(xs[i], xs[i + 1], ys[j], ys[j + 1]) {
                continue;
            }
            let (a, b) = (point(i, j, &mut mesh), point(i + 1, j, &mut mesh));
            let (c, d) = (point(i + 1, j + 1, &mut mesh), point(i, j + 1, &mut mesh));
            // Split cells along alternating diagonals (by their place on the
            // shared lattice), so blends don't line up into one long zigzag.
            let gx = ((s.origin[0] as f32 + xs[i] + 0.5) / cell).floor() as i64;
            let gy = ((s.origin[1] as f32 + ys[j] + 0.5) / cell).floor() as i64;
            if (gx + gy).rem_euclid(2) == 0 {
                mesh.triangles.push([a, b, c]);
                mesh.triangles.push([a, c, d]);
            } else {
                mesh.triangles.push([a, b, d]);
                mesh.triangles.push([b, c, d]);
            }
        }
    }
    mesh
}

/// Joints never get a thinner band than this (px) to bend over.
const MIN_JOINT_THICKNESS: f32 = 3.0;

/// Smooth bending at the joints. Mesh points near a joint where a bone hangs
/// from its parent follow a blend of both bones, fading from the parent to
/// the child across a band whose half-width is `softness` × the drawing's
/// half-thickness at that joint (`dt` is the edge-distance map of the
/// character, `w` pixels wide). 0 keeps every point on its own bone (rigid).
///
/// A point's weights depend only on where it is, not on which part's mesh it
/// belongs to, so where two parts meet (their grid points coincide) both
/// sides move together and the seam stays closed.
pub fn smooth_weights(meshes: &mut [PartMesh], skel: &crate::skeleton::Skeleton, dt: &[f32], w: usize, softness: f32) {
    // Every parent → child joint: where it is, how thick the drawing is there,
    // and which way the child points.
    struct Joint {
        at: [f32; 2],
        thickness: f32,
        dir: [f32; 2],
        parent: usize,
        child: usize,
    }
    let thickness_at = |p: [f32; 2]| {
        let h = dt.len() / w.max(1);
        let (x, y) = (p[0].max(0.0) as usize, p[1].max(0.0) as usize);
        if x < w && y < h { dt[y * w + x] } else { 0.0 }
    };
    let joints: Vec<Joint> = skel
        .parts
        .iter()
        .enumerate()
        .filter_map(|(child, part)| {
            let parent = part.parent?;
            let (a, b) = skel.bone(child);
            let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt().max(1e-3);
            Some(Joint {
                at: a,
                thickness: thickness_at(a).max(MIN_JOINT_THICKNESS),
                dir: [(b[0] - a[0]) / len, (b[1] - a[1]) / len],
                parent,
                child,
            })
        })
        .collect();
    for mesh in meshes.iter_mut() {
        for (p, weights) in mesh.points.iter().zip(mesh.weights.iter_mut()) {
            *weights = vec![(mesh.part, 1.0)];
            if softness <= 0.0 {
                continue;
            }
            // The nearest joint this part is on either side of, if the point is
            // close to it. Several children can share one joint (both thighs at
            // the hip); then the child bone nearest the point wins, so the
            // point agrees with that child's own mesh along their seam.
            let near = joints
                .iter()
                .filter(|j| j.parent == mesh.part || j.child == mesh.part)
                .map(|j| {
                    let d = ((p[0] - j.at[0]).powi(2) + (p[1] - j.at[1]).powi(2)).sqrt();
                    let (a, b) = skel.bone(j.child);
                    (j, d, crate::skeleton::dist_to_segment(*p, a, b))
                })
                .filter(|(j, d, _)| *d <= j.thickness * (1.5 + softness))
                .min_by(|a, b| a.1.total_cmp(&b.1).then(a.2.total_cmp(&b.2)));
            let Some((j, ..)) = near else { continue };
            // How far past the joint, along the child bone, this point is.
            let along = (p[0] - j.at[0]) * j.dir[0] + (p[1] - j.at[1]) * j.dir[1];
            let r = softness * j.thickness;
            let u = ((along + r) / (2.0 * r)).clamp(0.0, 1.0);
            let child = u * u * (3.0 - 2.0 * u);
            *weights = vec![(j.parent, 1.0 - child), (j.child, child)];
            weights.retain(|&(_, w)| w > 1e-4);
        }
    }
}

impl PartMesh {
    /// The points moved by the bones: each is the weighted mix of where its
    /// bones' transforms (`world_transforms`, one per part) would put it.
    pub fn deform(&self, transforms: &[Affine2]) -> Vec<[f32; 2]> {
        self.points
            .iter()
            .zip(&self.weights)
            .map(|(&p, ws)| {
                let mut out = [0.0, 0.0];
                let mut total = 0.0;
                for &(bone, w) in ws {
                    if let Some(t) = transforms.get(bone) {
                        let q = t.apply(p);
                        out[0] += q[0] * w;
                        out[1] += q[1] * w;
                        total += w;
                    }
                }
                if total > 0.0 { [out[0] / total, out[1] / total] } else { p }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    /// An L-shaped sprite at (13, 7): a 30×6 bar plus a 6×20 leg.
    fn l_sprite() -> Sprite {
        let mut image = RgbaImage::new(30, 26);
        for (x, y, px) in image.enumerate_pixels_mut() {
            if y < 6 || x < 6 {
                *px = Rgba([200, 100, 50, 255]);
            }
        }
        Sprite { part: 3, image, origin: [13, 7], pivot: [16.0, 10.0] }
    }

    fn inside(p: [f32; 2], t: [[f32; 2]; 3]) -> bool {
        let s = |a: [f32; 2], b: [f32; 2]| (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
        let (d1, d2, d3) = (s(t[0], t[1]), s(t[1], t[2]), s(t[2], t[0]));
        !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
    }

    #[test]
    fn every_visible_pixel_is_covered_and_empty_cells_are_left_out() {
        let s = l_sprite();
        let m = &build_meshes(std::slice::from_ref(&s), 5.0)[0];
        assert_eq!(m.part, 3);
        let tri = |t: &[u32; 3]| [m.points[t[0] as usize], m.points[t[1] as usize], m.points[t[2] as usize]];
        for (x, y, px) in s.image.enumerate_pixels() {
            let p = [s.origin[0] as f32 + x as f32 + 0.5, s.origin[1] as f32 + y as f32 + 0.5];
            let covered = m.triangles.iter().any(|t| inside(p, tri(t)));
            if px.0[3] > 0 {
                assert!(covered, "visible pixel ({x},{y}) is in the mesh");
            }
        }
        // The empty corner (bottom right) has no triangles.
        let empty = [s.origin[0] as f32 + 25.0, s.origin[1] as f32 + 20.0];
        assert!(!m.triangles.iter().any(|t| inside(empty, tri(t))));
        // Points stay on the sprite, with matching texture positions.
        for (p, uv) in m.points.iter().zip(&m.uvs) {
            assert!((0.0..=1.0).contains(&uv[0]) && (0.0..=1.0).contains(&uv[1]));
            assert!((p[0] - (13.0 + uv[0] * 30.0)).abs() < 1e-4 && (p[1] - (7.0 + uv[1] * 26.0)).abs() < 1e-4);
        }
    }

    #[test]
    fn grid_lines_are_shared_across_the_drawing() {
        // Two sprites at different places: their inner grid lines land on the same lattice.
        let (a, mut b) = (l_sprite(), l_sprite());
        b.origin = [41, 22];
        for m in build_meshes(&[a, b], 5.0) {
            let s = &m;
            let on_lattice = s.points.iter().filter(|p| (p[0] / 5.0).fract().abs() < 1e-4).count();
            assert!(on_lattice > 0);
        }
    }

    #[test]
    fn rigid_weights_move_the_mesh_like_the_part() {
        let m = &build_meshes(&[l_sprite()], 5.0)[0];
        let mut t = vec![Affine2::IDENTITY; 4];
        t[3] = Affine2::translate([3.0, -2.0]).mul(&Affine2::rotate_about([16.0, 10.0], 0.8));
        for (moved, p) in m.deform(&t).iter().zip(&m.points) {
            let want = t[3].apply(*p);
            assert!((moved[0] - want[0]).abs() < 1e-4 && (moved[1] - want[1]).abs() < 1e-4);
        }
    }

    #[test]
    fn blended_weights_mix_the_bones() {
        let mut m = build_meshes(&[l_sprite()], 5.0).remove(0);
        for w in &mut m.weights {
            *w = vec![(0, 0.5), (1, 0.5)];
        }
        let t = [Affine2::translate([10.0, 0.0]), Affine2::translate([0.0, 10.0])];
        for (moved, p) in m.deform(&t).iter().zip(&m.points) {
            assert!((moved[0] - (p[0] + 5.0)).abs() < 1e-4 && (moved[1] - (p[1] + 5.0)).abs() < 1e-4);
        }
    }

    /// A straight vertical "arm" 10 px wide: upper arm from (20,0) to (20,40),
    /// forearm from (20,40) to (20,80), as two sprites split at the elbow.
    fn straight_arm() -> (Vec<Sprite>, crate::skeleton::Skeleton, Vec<f32>, usize) {
        use crate::skeleton::{Joint, SkelPart, Skeleton, distance_transform};
        let (w, h) = (40u32, 80u32);
        let mut fg = crate::mask::Mask::new(w, h);
        for y in 0..h {
            for x in 15..25 {
                fg.set(x, y, true);
            }
        }
        let piece = |part: usize, y0: u32, y1: u32| {
            let mut image = RgbaImage::new(10, y1 - y0);
            for px in image.pixels_mut() {
                *px = Rgba([90, 90, 90, 255]);
            }
            Sprite { part, image, origin: [15, y0 as i32], pivot: [20.0, y0 as f32] }
        };
        let j = |n: &str, y: f32| Joint { name: n.into(), pos: [20.0, y] };
        let skel = Skeleton {
            joints: vec![j("shoulder", 0.0), j("elbow", 40.0), j("hand", 80.0)],
            parts: vec![
                SkelPart { name: "upper".into(), pivot: 0, tip: 1, parent: None, limit: None, layer: 0 },
                SkelPart { name: "fore".into(), pivot: 1, tip: 2, parent: Some(0), limit: None, layer: 0 },
            ],
        };
        (vec![piece(0, 0, 40), piece(1, 40, 80)], skel, distance_transform(&fg), w as usize)
    }

    #[test]
    fn softness_blends_bones_near_the_joint_and_keeps_the_seam_closed() {
        let (sprites, skel, dt, w) = straight_arm();
        let mut meshes = build_meshes(&sprites, 4.0);
        smooth_weights(&mut meshes, &skel, &dt, w, 1.0);
        let weight_of = |m: &PartMesh, i: usize, bone: usize| {
            m.weights[i].iter().find(|&&(b, _)| b == bone).map_or(0.0, |&(_, w)| w)
        };
        // Far from the elbow: rigid. At the elbow: half and half. Weights add to 1.
        let upper = &meshes[0];
        let top = upper.points.iter().position(|p| p[1] == 0.0).unwrap();
        assert_eq!(upper.weights[top], vec![(0, 1.0)]);
        for m in &meshes {
            for (i, p) in m.points.iter().enumerate() {
                let total: f32 = m.weights[i].iter().map(|w| w.1).sum();
                assert!((total - 1.0).abs() < 1e-4);
                if p[1] == 40.0 {
                    assert!((weight_of(m, i, 1) - 0.5).abs() < 1e-4, "half and half on the joint line");
                }
            }
        }
        // Bend the elbow 60°: points the two meshes share land in the same place.
        let mut t = vec![Affine2::IDENTITY; 2];
        t[1] = Affine2::rotate_about([20.0, 40.0], 1.0);
        let (a, b) = (meshes[0].deform(&t), meshes[1].deform(&t));
        let mut shared = 0;
        for (i, p) in meshes[0].points.iter().enumerate() {
            if let Some(k) = meshes[1].points.iter().position(|q| q == p) {
                assert!((a[i][0] - b[k][0]).abs() < 1e-3 && (a[i][1] - b[k][1]).abs() < 1e-3, "seam point {p:?} stays shut");
                shared += 1;
            }
        }
        assert!(shared >= 2, "the meshes meet along the elbow");
        // Softness 0 is rigid.
        smooth_weights(&mut meshes, &skel, &dt, w, 0.0);
        assert!(meshes.iter().all(|m| m.weights.iter().all(|w| w.len() == 1 && w[0].0 == m.part)));
    }

    #[test]
    fn cell_size_follows_the_figure_with_a_floor() {
        assert_eq!(cell_size(RectI::new(0, 0, 96, 1600)), 10.0);
        assert_eq!(cell_size(RectI::new(0, 0, 20, 20)), MIN_CELL);
    }
}
