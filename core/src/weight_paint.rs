//! Hand-painted weights: changes the user brushes on top of the automatic
//! bone weights (`mesh::smooth_weights`), e.g. "this hair follows the head".
//!
//! Painting is stored per bone name as amounts (-1..1) at points of a grid
//! laid over the drawing, not on any particular mesh, so it survives moving
//! joints and rebuilding meshes, and reads the same on both sides of a seam.

use crate::mesh::PartMesh;
use crate::skeleton::Skeleton;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WeightPaint {
    /// Spacing (px) of the grid the painting is stored on; set by the first stroke.
    pub cell: f32,
    /// Bone name → painted amounts at grid points `(column, row, amount)`.
    /// Positive pulls the point toward the bone, negative pushes it away.
    pub bones: BTreeMap<String, Vec<(i32, i32, f32)>>,
}

impl WeightPaint {
    pub fn is_empty(&self) -> bool {
        self.bones.values().all(Vec::is_empty)
    }

    /// One dab of the brush for `bone`, centred on `at` (drawing px) with
    /// radius `radius`: adds `amount` (negative to remove) at the grid points
    /// inside, fading to nothing at the edge. `cell` is used if nothing has
    /// been painted yet.
    pub fn dab(&mut self, bone: &str, at: [f32; 2], radius: f32, amount: f32, cell: f32) {
        if self.cell <= 0.0 {
            self.cell = cell.max(1.0);
        }
        let c = self.cell;
        let points = self.bones.entry(bone.to_string()).or_default();
        let (i0, i1) = (((at[0] - radius) / c).floor() as i32, ((at[0] + radius) / c).ceil() as i32);
        let (j0, j1) = (((at[1] - radius) / c).floor() as i32, ((at[1] + radius) / c).ceil() as i32);
        for j in j0..=j1 {
            for i in i0..=i1 {
                let d = ((i as f32 * c - at[0]).powi(2) + (j as f32 * c - at[1]).powi(2)).sqrt();
                if d >= radius {
                    continue;
                }
                let fall = 1.0 - d / radius;
                let add = amount * fall * fall;
                match points.iter_mut().find(|(pi, pj, _)| *pi == i && *pj == j) {
                    Some(p) => p.2 = (p.2 + add).clamp(-1.0, 1.0),
                    None => points.push((i, j, add.clamp(-1.0, 1.0))),
                }
            }
        }
        points.retain(|p| p.2.abs() > 1e-4);
    }

    /// Blur what's painted for `bone` inside the brush (centre `at`, radius
    /// `radius`), so hard stroke edges fade out. `strength` (0..1) is how far
    /// each grid point moves toward the average of itself and its neighbours.
    pub fn smooth(&mut self, bone: &str, at: [f32; 2], radius: f32, strength: f32) {
        let c = self.cell;
        let Some(points) = self.bones.get_mut(bone).filter(|_| c > 0.0) else { return };
        let before: Grid = points.iter().map(|&(i, j, v)| ((i, j), v)).collect();
        let get = |i: i32, j: i32| before.get(&(i, j)).copied().unwrap_or(0.0);
        let (i0, i1) = (((at[0] - radius) / c).floor() as i32, ((at[0] + radius) / c).ceil() as i32);
        let (j0, j1) = (((at[1] - radius) / c).floor() as i32, ((at[1] + radius) / c).ceil() as i32);
        for j in j0..=j1 {
            for i in i0..=i1 {
                let d = ((i as f32 * c - at[0]).powi(2) + (j as f32 * c - at[1]).powi(2)).sqrt();
                if d >= radius {
                    continue;
                }
                let fall = 1.0 - d / radius;
                let v = get(i, j);
                let avg = (v + get(i - 1, j) + get(i + 1, j) + get(i, j - 1) + get(i, j + 1)) / 5.0;
                let new = v + (avg - v) * (strength * fall * fall).min(1.0);
                match points.iter_mut().find(|(pi, pj, _)| *pi == i && *pj == j) {
                    Some(p) => p.2 = new,
                    None if new.abs() > 1e-4 => points.push((i, j, new)),
                    None => {}
                }
            }
        }
        points.retain(|p| p.2.abs() > 1e-4);
    }

    /// Forget everything painted for `bone`.
    pub fn clear(&mut self, bone: &str) {
        self.bones.remove(bone);
    }

    /// Keep painting with a bone when it's renamed.
    pub fn rename(&mut self, old: &str, new: &str) {
        if old != new
            && let Some(points) = self.bones.remove(old)
        {
            self.bones.insert(new.to_string(), points);
        }
    }
}

/// One bone's painted amounts by grid point, for quick lookup.
type Grid = HashMap<(i32, i32), f32>;

/// Add the painting to the meshes' (automatic) weights: each painted bone's
/// amount at a point, read smoothly between grid points, is added to that
/// bone's weight there (kept within 0..1), then the weights are scaled to add
/// up to 1. A point whose weights all end up at 0 follows its own part.
pub fn apply_paint(meshes: &mut [PartMesh], skel: &Skeleton, paint: &WeightPaint) {
    if paint.is_empty() || paint.cell <= 0.0 {
        return;
    }
    let c = paint.cell;
    // Painted bones that exist, with their grid points ready for quick lookup.
    let painted: Vec<(usize, Grid)> = paint
        .bones
        .iter()
        .filter_map(|(name, pts)| {
            let bone = skel.parts.iter().position(|p| &p.name == name)?;
            Some((bone, pts.iter().map(|&(i, j, v)| ((i, j), v)).collect()))
        })
        .filter(|(_, m): &(usize, Grid)| !m.is_empty())
        .collect();
    if painted.is_empty() {
        return;
    }
    let amount_at = |grid: &Grid, p: [f32; 2]| {
        let (fx, fy) = (p[0] / c, p[1] / c);
        let (i, j) = (fx.floor() as i32, fy.floor() as i32);
        let (u, v) = (fx - i as f32, fy - j as f32);
        let g = |di: i32, dj: i32| grid.get(&(i + di, j + dj)).copied().unwrap_or(0.0);
        let top = g(0, 0) + (g(1, 0) - g(0, 0)) * u;
        let bottom = g(0, 1) + (g(1, 1) - g(0, 1)) * u;
        top + (bottom - top) * v
    };
    for mesh in meshes.iter_mut() {
        for (p, weights) in mesh.points.iter().zip(mesh.weights.iter_mut()) {
            for (bone, grid) in &painted {
                let a = amount_at(grid, *p);
                if a.abs() < 1e-4 {
                    continue;
                }
                set_weight(weights, *bone, a, skel.parts[*bone].parent);
            }
        }
    }
}

/// Change `bone`'s weight in `weights` (which add up to 1) by `amount`,
/// keeping the total at 1. Adding takes the share evenly (by proportion) from
/// the other bones; removing gives the freed share to the other bones, or to
/// `parent` if `bone` was the only one, so removing a bone's pull really
/// hands the point to a neighbour instead of being undone by rescaling.
fn set_weight(weights: &mut Vec<(usize, f32)>, bone: usize, amount: f32, parent: Option<usize>) {
    let old = weights.iter().find(|w| w.0 == bone).map_or(0.0, |w| w.1);
    let new = (old + amount).clamp(0.0, 1.0);
    if (new - old).abs() < 1e-6 {
        return;
    }
    let others: f32 = weights.iter().filter(|w| w.0 != bone).map(|w| w.1).sum();
    if new < old && others <= 1e-6 {
        // Nothing else holds this point: the parent takes what's removed.
        let Some(parent) = parent else { return };
        *weights = vec![(bone, new), (parent, 1.0 - new)];
    } else {
        // The other bones share the rest (1 - new) in their current proportions.
        let scale = if others > 1e-6 { (1.0 - new) / others } else { 0.0 };
        for w in weights.iter_mut() {
            w.1 = if w.0 == bone { new } else { w.1 * scale };
        }
        if !weights.iter().any(|w| w.0 == bone) {
            weights.push((bone, new));
        }
    }
    weights.retain(|&(_, w)| w > 1e-4);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::RectI;
    use crate::mesh::{build_meshes, smooth_weights};
    use crate::skeleton::distance_transform;
    use crate::sprites::Sprite;
    use image::{Rgba, RgbaImage};

    fn setup() -> (Vec<PartMesh>, Skeleton) {
        // Two side-by-side 20×20 parts on a 2-bone skeleton.
        let skel = crate::template::builtin()[0].fit(RectI::new(0, 0, 100, 200));
        let sprite = |part: usize, x: i32| Sprite {
            part,
            image: RgbaImage::from_pixel(20, 20, Rgba([1, 2, 3, 255])),
            origin: [x, 0],
            pivot: [0.0, 0.0],
        };
        let meshes = build_meshes(&[sprite(0, 0), sprite(1, 20)], 5.0);
        (meshes, skel)
    }

    fn weight(m: &PartMesh, i: usize, bone: usize) -> f32 {
        m.weights[i].iter().find(|w| w.0 == bone).map_or(0.0, |w| w.1)
    }

    #[test]
    fn painting_pulls_points_toward_a_bone_and_removing_undoes_it() {
        let (mut meshes, skel) = setup();
        let head = &skel.parts[0].name;
        let mut paint = WeightPaint::default();
        // Paint the head bone (part 0) strongly onto part 1's middle.
        for _ in 0..10 {
            paint.dab(head, [30.0, 10.0], 8.0, 0.5, 5.0);
        }
        assert_eq!(paint.cell, 5.0);
        apply_paint(&mut meshes, &skel, &paint);
        let m = &meshes[1];
        let mid = m.points.iter().position(|p| *p == [30.0, 10.0]).unwrap();
        assert!(weight(m, mid, 0) > 0.45, "pulled to the head: {:?}", m.weights[mid]);
        for w in &m.weights {
            assert!((w.iter().map(|x| x.1).sum::<f32>() - 1.0).abs() < 1e-4, "weights add to 1");
        }
        // Far from the brush: untouched.
        let corner = m.points.iter().position(|p| *p == [40.0, 20.0]).unwrap();
        assert_eq!(m.weights[corner], vec![(1, 1.0)]);

        // Removing it again brings the point back to its own bone.
        for _ in 0..10 {
            paint.dab(head, [30.0, 10.0], 8.0, -0.5, 5.0);
        }
        let (mut fresh, _) = setup();
        apply_paint(&mut fresh, &skel, &paint);
        assert_eq!(fresh[1].weights[mid], vec![(1, 1.0)]);
    }

    #[test]
    fn painting_survives_new_meshes_and_renames_and_reads_the_same_across_seams() {
        let (_, skel) = setup();
        let mut paint = WeightPaint::default();
        paint.dab(&skel.parts[0].name, [20.0, 10.0], 12.0, 0.6, 5.0);
        // Rebuilt meshes (e.g. after moving a joint) pick it up again.
        let (mut meshes, _) = setup();
        let dt = distance_transform(&crate::mask::Mask::new(40, 20));
        smooth_weights(&mut meshes, &skel, &dt, 40, 0.0);
        // Seam points (x = 20, in both meshes) with the same automatic weights,
        // as a soft joint gives them, end up the same after painting too.
        for m in meshes.iter_mut() {
            for (p, w) in m.points.iter().zip(m.weights.iter_mut()) {
                if p[0] == 20.0 {
                    *w = vec![(0, 0.5), (1, 0.5)];
                }
            }
        }
        apply_paint(&mut meshes, &skel, &paint);
        let mut shared = 0;
        for (i, p) in meshes[0].points.iter().enumerate() {
            if let Some(k) = meshes[1].points.iter().position(|q| q == p) {
                assert!((weight(&meshes[0], i, 0) - weight(&meshes[1], k, 0)).abs() < 1e-5, "seam point {p:?}");
                shared += 1;
            }
        }
        assert!(shared >= 2);
        // Renamed bone keeps its painting; unknown bones are ignored.
        paint.rename(&skel.parts[0].name, "big head");
        assert!(paint.bones.contains_key("big head"));
        let (mut meshes, _) = setup();
        apply_paint(&mut meshes, &skel, &paint);
        assert!(meshes.iter().all(|m| m.weights.iter().all(|w| w.len() == 1)), "no bone called that any more");
        paint.clear("big head");
        assert!(paint.is_empty());
    }

    #[test]
    fn removing_a_bones_pull_from_its_own_part_hands_it_to_the_parent() {
        let (mut meshes, skel) = setup();
        // Part 1 is the torso in the front template; its own points follow it fully.
        let torso = 1;
        assert_eq!(skel.parts[torso].parent, None);
        let head = 0; // the head hangs from the torso
        let mut paint = WeightPaint::default();
        // Remove the head's pull from the head's own part (mesh 0).
        for _ in 0..10 {
            paint.dab(&skel.parts[head].name, [10.0, 10.0], 8.0, -0.5, 5.0);
        }
        apply_paint(&mut meshes, &skel, &paint);
        let m = &meshes[0];
        let mid = m.points.iter().position(|p| *p == [10.0, 10.0]).unwrap();
        assert!(weight(m, mid, head) < 0.05, "the head lets go: {:?}", m.weights[mid]);
        assert!(weight(m, mid, torso) > 0.95, "its parent takes over");
        // Weights still add to 1 everywhere.
        for w in &m.weights {
            assert!((w.iter().map(|x| x.1).sum::<f32>() - 1.0).abs() < 1e-4);
        }
    }

    #[test]
    fn set_weight_keeps_the_total_at_one() {
        let mut w = vec![(0, 0.5), (1, 0.5)];
        set_weight(&mut w, 0, 0.3, None);
        assert!((w.iter().map(|x| x.1).sum::<f32>() - 1.0).abs() < 1e-5);
        assert!((w[0].1 - 0.8).abs() < 1e-5);
        set_weight(&mut w, 2, 0.5, None);
        assert!((w.iter().find(|x| x.0 == 2).unwrap().1 - 0.5).abs() < 1e-5);
        assert!((w.iter().map(|x| x.1).sum::<f32>() - 1.0).abs() < 1e-5);
        let mut alone = vec![(4, 1.0)];
        set_weight(&mut alone, 4, -1.0, None);
        assert_eq!(alone, vec![(4, 1.0)], "no parent to hand it to: stays");
    }

    #[test]
    fn smoothing_softens_a_hard_edge() {
        // A hard-edged block of full paint: columns 0..5 at 1.0, nothing beyond.
        let mut paint = WeightPaint { cell: 2.0, ..Default::default() };
        paint.bones.insert("arm".into(), (0..5).flat_map(|i| (0..5).map(move |j| (i, j, 1.0))).collect());
        let at = |p: &WeightPaint, i: i32| p.bones["arm"].iter().find(|q| q.0 == i && q.1 == 2).map_or(0.0, |q| q.2);
        let edge_before = at(&paint, 4) - at(&paint, 5);
        for _ in 0..5 {
            paint.smooth("arm", [9.0, 4.0], 6.0, 0.8);
        }
        let (inside, outside) = (at(&paint, 4), at(&paint, 5));
        assert!(inside < 1.0 && outside > 0.0, "the step spreads: {inside} / {outside}");
        assert!(inside - outside < edge_before, "and gets gentler");
        // Far from the brush nothing changes.
        assert_eq!(paint.bones["arm"].iter().find(|q| q.0 == 0 && q.1 == 0).unwrap().2, 1.0);
        // Nothing painted, nothing to smooth.
        paint.smooth("leg", [0.0, 0.0], 5.0, 1.0);
        assert!(!paint.bones.contains_key("leg"));
    }

    #[test]
    fn saves_and_loads() {
        let mut paint = WeightPaint::default();
        paint.dab("tail", [3.0, 4.0], 6.0, 0.3, 2.0);
        let back: WeightPaint = serde_json::from_str(&serde_json::to_string(&paint).unwrap()).unwrap();
        assert_eq!(back, paint);
    }
}
