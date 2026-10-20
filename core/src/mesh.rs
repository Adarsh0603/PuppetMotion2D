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
const CELLS_PER_FIGURE: f32 = 48.0;
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
            mesh.triangles.push([a, b, c]);
            mesh.triangles.push([a, c, d]);
        }
    }
    mesh
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

    #[test]
    fn cell_size_follows_the_figure_with_a_floor() {
        assert_eq!(cell_size(RectI::new(0, 0, 96, 480)), 10.0);
        assert_eq!(cell_size(RectI::new(0, 0, 20, 20)), MIN_CELL);
    }
}
