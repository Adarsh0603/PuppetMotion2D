//! Joint-based part assignment: the user places joints, each part is the bone
//! between two of them, and every foreground pixel goes to the nearest part.
//!
//! "Nearest" is measured to the part's capsule (bone line grown by the part's
//! thickness), not to the bare bone. Thickness is measured from the drawing,
//! so a wide torso keeps its own corners instead of losing them to the thin
//! arms attached there.

use crate::geom::RectI;
use crate::mask::Mask;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Joint {
    pub name: String,
    pub pos: [f32; 2],
}

/// A body part spanning two joints. `pivot` is the end attached to the parent
/// part and the point the part rotates around; `tip` is the free end.
/// `parent` is the part it hangs from (None for a root such as the torso).
/// `limit` is the allowed rotation range `[min, max]` in radians relative to
/// the parent (0 = as drawn); None means it can turn freely.
/// `layer` is the part's place in the painting order, lowest at the back;
/// see `Skeleton::paint_order`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkelPart {
    pub name: String,
    pub pivot: usize,
    pub tip: usize,
    pub parent: Option<usize>,
    #[serde(default)]
    pub limit: Option<[f32; 2]>,
    #[serde(default)]
    pub layer: i32,
}

/// Where `Skeleton::move_layer` moves a part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toward {
    /// One place nearer the front.
    Front,
    /// One place nearer the back.
    Back,
    /// In front of everything.
    FrontMost,
    /// Behind everything.
    BackMost,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Skeleton {
    pub joints: Vec<Joint>,
    pub parts: Vec<SkelPart>,
}

impl Skeleton {
    /// Front-facing stick person (the built-in "Person (front)" template)
    /// scaled into `bbox`, usually the foreground's bounding box. Left/right
    /// are as seen on screen. The torso is the root; a forearm's pivot is its
    /// upper arm's tip (the elbow), which is what lets IK move the two together.
    pub fn person_front(bbox: RectI) -> Skeleton {
        crate::template::builtin()[0].fit(bbox)
    }

    pub fn empty() -> Skeleton {
        Skeleton { joints: Vec::new(), parts: Vec::new() }
    }

    /// Swap the positions of every "near …" joint with its "far …" partner
    /// (e.g. "near knee" ↔ "far knee"), for a side-view drawing posed with
    /// the other limb in front. Names and layers stay. Returns whether
    /// anything was swapped.
    pub fn swap_near_far(&mut self) -> bool {
        let mut swapped = false;
        for i in 0..self.joints.len() {
            let Some(rest) = self.joints[i].name.strip_prefix("near ") else { continue };
            let far = format!("far {rest}");
            if let Some(k) = self.joints.iter().position(|j| j.name == far) {
                let (a, b) = (self.joints[i].pos, self.joints[k].pos);
                self.joints[i].pos = b;
                self.joints[k].pos = a;
                swapped = true;
            }
        }
        swapped
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
    /// Its parent is guessed: the bone ending at `pivot`, else a bone starting
    /// there, else the bone nearest to it (none if this is the first bone).
    pub fn add_part(&mut self, pivot: usize, tip: usize) -> usize {
        let name = unique_name("bone", self.parts.iter().map(|p| p.name.as_str()));
        let parent = self.guess_parent(pivot);
        // A new bone goes in front of everything, where it can be seen.
        let layer = self.parts.iter().map(|p| p.layer + 1).max().unwrap_or(0);
        self.parts.push(SkelPart { name, pivot, tip, parent, limit: None, layer });
        self.parts.len() - 1
    }

    fn guess_parent(&self, pivot: usize) -> Option<usize> {
        if let Some(i) = self.parts.iter().position(|p| p.tip == pivot) {
            return Some(i);
        }
        if let Some(i) = self.parts.iter().position(|p| p.pivot == pivot) {
            return Some(i);
        }
        let at = self.joints[pivot].pos;
        (0..self.parts.len())
            .map(|i| {
                let (a, b) = self.bone(i);
                (i, dist_to_segment(at, a, b))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// Remove a joint and every part attached to it.
    pub fn remove_joint(&mut self, joint: usize) {
        for i in (0..self.parts.len()).rev() {
            if self.parts[i].pivot == joint || self.parts[i].tip == joint {
                self.drop_part(i);
            }
        }
        self.drop_joint(joint);
        self.prune_orphans();
    }

    /// Remove a part, and any joint no other part uses. Its children are
    /// re-attached to its parent.
    pub fn remove_part(&mut self, part: usize) {
        self.drop_part(part);
        self.prune_orphans();
    }

    fn drop_part(&mut self, part: usize) {
        let grandparent = self.parts[part].parent;
        self.parts.remove(part);
        for p in &mut self.parts {
            p.parent = match p.parent {
                Some(q) if q == part => grandparent.map(|g| if g > part { g - 1 } else { g }),
                Some(q) if q > part => Some(q - 1),
                other => other,
            };
        }
    }

    /// Parts directly attached to `part`.
    pub fn children(&self, part: usize) -> impl Iterator<Item = usize> + '_ {
        (0..self.parts.len()).filter(move |&i| self.parts[i].parent == Some(part))
    }

    /// Every part below `part` in the hierarchy (children, grandchildren, …).
    pub fn descendants(&self, part: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stack = vec![part];
        while let Some(p) = stack.pop() {
            for c in self.children(p) {
                if !out.contains(&c) && c != part {
                    out.push(c);
                    stack.push(c);
                }
            }
        }
        out
    }

    /// Re-attach `part` under `parent`. Refused (returns false) if that would
    /// create a loop, i.e. `parent` is the part itself or one of its descendants.
    pub fn set_parent(&mut self, part: usize, parent: Option<usize>) -> bool {
        if let Some(p) = parent
            && (p == part || p >= self.parts.len() || self.descendants(part).contains(&p))
        {
            return false;
        }
        self.parts[part].parent = parent;
        true
    }

    /// All parts with every parent before its children: the order to compute
    /// transforms in and to draw in (children on top).
    pub fn draw_order(&self) -> Vec<usize> {
        let mut order = Vec::with_capacity(self.parts.len());
        let mut seen = vec![false; self.parts.len()];
        let roots: Vec<usize> = (0..self.parts.len())
            .filter(|&i| self.parts[i].parent.is_none_or(|p| p >= self.parts.len()))
            .collect();
        let mut queue: std::collections::VecDeque<usize> = roots.into();
        while let Some(p) = queue.pop_front() {
            if std::mem::replace(&mut seen[p], true) {
                continue;
            }
            order.push(p);
            queue.extend(self.children(p));
        }
        // Anything unreachable (only possible with a corrupted loop) goes last.
        order.extend((0..self.parts.len()).filter(|&i| !seen[i]));
        order
    }

    /// The order to paint parts in, back to front: by `layer`, and for equal
    /// layers (a fresh template has all 0) parents before children, as
    /// `draw_order`. Use this for drawing; use `draw_order` for transforms.
    pub fn paint_order(&self) -> Vec<usize> {
        let mut order = self.draw_order();
        order.sort_by_key(|&p| self.parts[p].layer); // stable: ties keep hierarchy order
        order
    }

    /// Move `part` in the painting order: one place toward the front
    /// (`Toward::Front`), one back, or all the way to either end. The parts
    /// hanging from it come along and sit just in front of it, so a limb
    /// stays together. Returns whether anything moved.
    pub fn move_layer(&mut self, part: usize, to: Toward) -> bool {
        if part >= self.parts.len() {
            return false;
        }
        let old = self.paint_order();
        // The part, then its descendants in their current order.
        let family: Vec<usize> = std::iter::once(part)
            .chain(old.iter().copied().filter(|&p| p != part && self.descendants(part).contains(&p)))
            .collect();
        let rest: Vec<usize> = old.iter().copied().filter(|p| !family.contains(p)).collect();
        // Where the part sits among the others now, then where it goes.
        let here = old.iter().take_while(|&&p| p != part).filter(|p| !family.contains(p)).count();
        let to = match to {
            Toward::Front => (here + 1).min(rest.len()),
            Toward::Back => here.saturating_sub(1),
            Toward::FrontMost => rest.len(),
            Toward::BackMost => 0,
        };
        let mut new = rest;
        new.splice(to..to, family);
        if new == old {
            return false;
        }
        for (i, &p) in new.iter().enumerate() {
            self.parts[p].layer = i as i32;
        }
        true
    }

    /// All parts depth-first as `(part, depth)`, each child listed right under
    /// its parent: the order for showing the hierarchy as an indented tree.
    pub fn tree_order(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::with_capacity(self.parts.len());
        let mut seen = vec![false; self.parts.len()];
        let mut stack: Vec<(usize, usize)> = (0..self.parts.len())
            .rev()
            .filter(|&i| self.parts[i].parent.is_none_or(|p| p >= self.parts.len()))
            .map(|i| (i, 0))
            .collect();
        while let Some((p, depth)) = stack.pop() {
            if std::mem::replace(&mut seen[p], true) {
                continue;
            }
            out.push((p, depth));
            let children: Vec<usize> = self.children(p).collect();
            stack.extend(children.into_iter().rev().map(|c| (c, depth + 1)));
        }
        // Anything unreachable (only possible with a corrupted loop) goes last.
        out.extend((0..self.parts.len()).filter(|&i| !seen[i]).map(|i| (i, 0)));
        out
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

/// "No part here" in `SkeletonMasks::labels` (a background pixel).
pub const NO_PART: u16 = u16::MAX;

pub struct SkeletonMasks {
    /// One mask per part, in `Skeleton::parts` order. Exclusive and covering
    /// every foreground pixel.
    pub masks: Vec<Mask>,
    /// Measured thickness (half-width, px) of each part.
    pub radii: Vec<f32>,
    /// The same split as one map: each pixel's part number, or `NO_PART`.
    pub labels: Vec<u16>,
}

/// Split the character into parts. Works out the edge-distance map itself;
/// use `skeleton_masks_with_dt` when it's already known (it only changes with `fg`).
pub fn skeleton_masks(fg: &Mask, skel: &Skeleton) -> SkeletonMasks {
    skeleton_masks_with_dt(fg, &distance_transform(fg), skel)
}

/// `skeleton_masks` with the character's edge-distance map (`distance_transform(fg)`)
/// passed in.
pub fn skeleton_masks_with_dt(fg: &Mask, dt: &[f32], skel: &Skeleton) -> SkeletonMasks {
    let mut labels = Vec::new();
    let radii = part_labels(fg, dt, fg.bbox(), skel, &mut labels);
    let masks = masks_from_labels(&labels, fg.w, fg.h, skel.parts.len());
    SkeletonMasks { masks, radii, labels }
}

/// One mask per part from a part-per-pixel map (see `part_labels`).
pub fn masks_from_labels(labels: &[u16], w: u32, h: u32, parts: usize) -> Vec<Mask> {
    let mut masks = vec![Mask::new(w, h); parts];
    for (i, &l) in labels.iter().enumerate() {
        if let Some(m) = masks.get_mut(l as usize) {
            m.data[i] = 255;
        }
    }
    masks
}

/// The part split as a map: writes each foreground pixel's part number
/// (`NO_PART` elsewhere) into `labels`, reusing its memory, and returns each
/// part's measured thickness. This is the per-joint-move work, so it takes
/// what doesn't change with the joints from the caller: the edge-distance
/// map `dt` and the character's bounding box `bbox`. Only that box is
/// scanned, spread over the CPU cores.
pub fn part_labels(fg: &Mask, dt: &[f32], bbox: Option<RectI>, skel: &Skeleton, labels: &mut Vec<u16>) -> Vec<f32> {
    let bones: Vec<_> = (0..skel.parts.len()).map(|i| skel.bone(i)).collect();
    let radii: Vec<f32> = bones.iter().map(|&(a, b)| bone_radius(dt, fg.w, fg.h, a, b)).collect();
    let (w, h) = (fg.w as usize, fg.h as usize);
    labels.clear();
    labels.resize(w * h, NO_PART);
    let Some(bb) = bbox.filter(|_| !bones.is_empty()) else {
        return radii;
    };
    // A part stops dead at a joint where it meets another part, like a paper
    // cut-out cut straight across: at its pivot if it hangs from a parent, and
    // at its tip if other parts hang from that joint. Only free ends (a hand,
    // the top of the head) keep a rounded end. Without this, a fat part's
    // rounded end reaches past the joint into its neighbour (a big head
    // would take the top of the torso and swing it around when posed).
    let cut_at_pivot: Vec<bool> = skel.parts.iter().map(|p| p.parent.is_some()).collect();
    let cut_at_tip: Vec<bool> = skel.parts.iter().map(|p| skel.parts.iter().any(|q| q.pivot == p.tip)).collect();
    let (x0, x1) = (bb.x.max(0) as usize, (bb.right().max(0) as usize).min(w));
    let rows = bb.y.max(0) as usize..(bb.bottom().max(0) as usize).min(h);
    crate::par::for_rows(labels, w, rows.clone(), |y, row| {
        // Each bone's distance once per pixel: capsule edge, and the cost with the joint cuts.
        let mut capsule = vec![0.0f32; bones.len()];
        let mut cost = vec![0.0f32; bones.len()];
        for (x, out) in row.iter_mut().enumerate().take(x1).skip(x0) {
            if fg.data[y * w + x] == 0 {
                continue;
            }
            let p = [x as f32 + 0.5, y as f32 + 0.5];
            for (i, &(a, b)) in bones.iter().enumerate() {
                capsule[i] = dist_to_segment(p, a, b) - radii[i];
                let t = along(p, a, b);
                let cut = (t < 0.0 && cut_at_pivot[i]) || (t > 1.0 && cut_at_tip[i]);
                cost[i] = if cut { f32::INFINITY } else { capsule[i] };
            }
            let mut best = first_min(&cost);
            if cost[best].is_infinite() {
                // Outside every part's span (e.g. a stray blob): nearest capsule wins.
                best = first_min(&capsule);
            }
            *out = best as u16;
        }
    });
    // A piece handed over can leave the receiver's own stray bit next to it; a few rounds settle it.
    for _ in 0..3 {
        if !absorb_strays(labels, w, x0..x1, rows.clone(), skel.parts.len()) {
            break;
        }
    }
    radii
}

/// Give each stray piece of a part to the neighbour it borders most. Where a
/// limb bends, the wedge on the outside of the bend lies past both bones'
/// joint cuts, so it goes to some other bone (a tail's wedges to the body)
/// and is left behind, cut loose, when the limb moves. Pieces touching only
/// the background are kept (a separate blob of drawing), and so are big ones
/// (over a quarter of the part's main piece: a torso split by an arm drawn
/// across it). Only the box `xs` × `rows` is looked at. Works on runs of
/// equal labels along each row, so it stays quick on big drawings. Returns
/// whether anything changed.
fn absorb_strays(labels: &mut [u16], w: usize, xs: std::ops::Range<usize>, rows: std::ops::Range<usize>, parts: usize) -> bool {
    struct Run {
        y: usize,
        x0: usize,
        x1: usize,
        label: u16,
    }
    let mut runs: Vec<Run> = Vec::new();
    // Where each row's runs start in `runs` (one extra entry at the end).
    let mut row_start = Vec::with_capacity(rows.len() + 1);
    for y in rows.clone() {
        row_start.push(runs.len());
        let row = &labels[y * w..(y + 1) * w];
        let mut x = xs.start;
        while x < xs.end {
            let label = row[x];
            let x0 = x;
            while x < xs.end && row[x] == label {
                x += 1;
            }
            if label != NO_PART && (label as usize) < parts {
                runs.push(Run { y, x0, x1: x, label });
            }
        }
    }
    row_start.push(runs.len());

    // Join runs into pieces: touching runs on neighbouring rows with the same label.
    let mut parent: Vec<usize> = (0..runs.len()).collect();
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    // Each pair of overlapping runs on rows r and r + 1 (the overlap's length).
    let mut overlaps: Vec<(usize, usize, usize)> = Vec::new();
    for r in 0..rows.len().saturating_sub(1) {
        let (above, below) = (row_start[r]..row_start[r + 1], row_start[r + 1]..row_start[r + 2]);
        let (mut a, mut b) = (above.start, below.start);
        while a < above.end && b < below.end {
            let lo = runs[a].x0.max(runs[b].x0);
            let hi = runs[a].x1.min(runs[b].x1);
            if lo < hi {
                overlaps.push((a, b, hi - lo));
            }
            if runs[a].x1 < runs[b].x1 { a += 1 } else { b += 1 }
        }
    }
    for &(a, b, _) in &overlaps {
        if runs[a].label == runs[b].label {
            let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
            parent[ra] = rb;
        }
    }
    let mut size = vec![0usize; runs.len()];
    for (i, run) in runs.iter().enumerate() {
        let root = find(&mut parent, i);
        size[root] += run.x1 - run.x0;
    }
    let mut main = vec![0usize; parts];
    for i in 0..runs.len() {
        if parent[i] == i {
            let part = runs[i].label as usize;
            main[part] = main[part].max(size[i]);
        }
    }
    let stray = |root: usize| size[root] * 4 <= main[runs[root].label as usize];

    // Count each stray piece's border with every other part.
    let mut votes: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
    let mut vote = |parent: &mut [usize], from: usize, to: usize, n: usize| {
        let root = find(parent, from);
        if runs[from].label != runs[to].label && stray(root) {
            votes.entry(root).or_insert_with(|| vec![0; parts])[runs[to].label as usize] += n;
        }
    };
    for &(a, b, n) in &overlaps {
        vote(&mut parent, a, b, n);
        vote(&mut parent, b, a, n);
    }
    for r in 0..rows.len() {
        for i in row_start[r]..row_start[r + 1].saturating_sub(1) {
            if runs[i].x1 == runs[i + 1].x0 {
                vote(&mut parent, i, i + 1, 1);
                vote(&mut parent, i + 1, i, 1);
            }
        }
    }
    if votes.is_empty() {
        return false;
    }
    let winner: std::collections::HashMap<usize, u16> =
        votes.into_iter().map(|(root, v)| (root, (0..parts).max_by_key(|&p| v[p]).unwrap_or(0) as u16)).collect();
    for (i, &Run { y, x0, x1, .. }) in runs.iter().enumerate() {
        if let Some(&to) = winner.get(&find(&mut parent, i)) {
            labels[y * w + x0..y * w + x1].fill(to);
        }
    }
    true
}

/// Index of the smallest value (the first one, on a tie).
fn first_min(v: &[f32]) -> usize {
    let mut best = 0;
    for i in 1..v.len() {
        if v[i].total_cmp(&v[best]).is_lt() {
            best = i;
        }
    }
    best
}

/// Where `p` falls along the bone a→b: 0 at `a`, 1 at `b`, outside [0, 1]
/// beyond either end.
fn along(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let len2 = ab[0] * ab[0] + ab[1] * ab[1];
    if len2 == 0.0 { 0.5 } else { ((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len2 }
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
    fn stray_pieces_join_the_part_around_them() {
        // 20 × 10: part 0 on the left, part 1 on the right, background in the last row.
        let (w, h) = (20, 10);
        let mut l = vec![NO_PART; w * h];
        for y in 0..9 {
            for x in 0..w {
                l[y * w + x] = if x < 10 { 0 } else { 1 };
            }
        }
        l[2 * w + 15] = 0; // a speck of part 0 inside part 1: a wedge left behind
        l[9 * w + 3] = 1; // part 1 alone on the background: a separate blob, kept
        // A big piece of part 1 cut off inside part 0 stays too (over a quarter of its main piece).
        for y in 0..9 {
            for x in 0..4 {
                l[y * w + x] = 1;
            }
        }
        assert!(absorb_strays(&mut l, w, 0..w, 0..h, 2));
        assert_eq!(l[2 * w + 15], 1, "the speck joins the part around it");
        assert_eq!(l[9 * w + 3], 1, "a piece touching only background stays");
        assert_eq!(l[w + 1], 1, "a big piece stays");
        assert!(!absorb_strays(&mut l, w, 0..w, 0..h, 2), "nothing left to move");
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
                SkelPart { name: "torso".into(), pivot: 0, tip: 1, parent: None, limit: None, layer: 0 },
                SkelPart { name: "arm".into(), pivot: 2, tip: 3, parent: Some(0), limit: None, layer: 0 },
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
        // The part-per-pixel map says the same thing as the masks.
        for (i, &l) in sm.labels.iter().enumerate() {
            let owners: Vec<usize> = (0..2).filter(|&p| sm.masks[p].data[i] != 0).collect();
            assert_eq!(owners, if l == NO_PART { vec![] } else { vec![l as usize] });
        }
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
    fn big_head_stops_at_the_neck() {
        // Big round head (r 40) on a narrow torso (20 px wide).
        let mut fg = Mask::new(120, 200);
        for y in 0..200u32 {
            for x in 0..120u32 {
                let head = (x as f32 - 60.0).powi(2) + (y as f32 - 50.0).powi(2) <= 1600.0;
                let torso = (50..70).contains(&x) && (88..180).contains(&y);
                if head || torso {
                    fg.set(x, y, true);
                }
            }
        }
        let j = |name: &str, x: f32, y: f32| Joint { name: name.into(), pos: [x, y] };
        let skel = Skeleton {
            joints: vec![j("pelvis", 60.0, 175.0), j("neck", 60.0, 90.0), j("head top", 60.0, 12.0)],
            parts: vec![
                SkelPart { name: "torso".into(), pivot: 0, tip: 1, parent: None, limit: None, layer: 0 },
                SkelPart { name: "head".into(), pivot: 1, tip: 2, parent: Some(0), limit: None, layer: 0 },
            ],
        };
        let sm = skeleton_masks(&fg, &skel);
        assert!(sm.radii[1] > 2.0 * sm.radii[0], "head is much fatter: {:?}", sm.radii);
        for (x, y) in [(52, 92), (60, 95), (68, 100)] {
            assert!(sm.masks[0].get(x, y), "({x},{y}) below the neck belongs to the torso");
        }
        assert!(sm.masks[1].get(60, 50) && sm.masks[1].get(25, 50), "head keeps the whole head");
        assert!(sm.masks[1].get(60, 13), "free end (head top) is still covered");
    }

    #[test]
    fn layers_paint_parts_behind_with_their_children() {
        let mut s = Skeleton::person_front(RectI::new(0, 0, 100, 200));
        let name = |s: &Skeleton, n: &str| s.parts.iter().position(|p| p.name == n).unwrap();
        let (torso, arm, forearm) = (name(&s, "torso"), name(&s, "left upper arm"), name(&s, "left forearm"));
        assert_eq!(s.paint_order(), s.draw_order(), "all layers 0: same as the hierarchy");
        let at = |s: &Skeleton, p: usize| s.paint_order().iter().position(|&q| q == p).unwrap();

        // One step back moves the arm past exactly one other part, and its
        // forearm comes along, right in front of it.
        let before = s.paint_order();
        let others: Vec<usize> = before.iter().copied().filter(|&p| p != arm && p != forearm).collect();
        let arm_among = before.iter().take_while(|&&p| p != arm).filter(|&&p| p != forearm).count();
        assert!(s.move_layer(arm, Toward::Back));
        let order = s.paint_order();
        assert_eq!(order.iter().position(|&p| p == forearm), order.iter().position(|&p| p == arm).map(|i| i + 1));
        let now_among = order.iter().take_while(|&&p| p != arm).count();
        assert_eq!(now_among, arm_among - 1, "exactly one place back");
        assert_eq!(order.iter().copied().filter(|p| others.contains(p)).collect::<Vec<_>>(), others, "others keep their order");

        // To the back: behind the torso, still in one piece.
        assert!(s.move_layer(arm, Toward::BackMost));
        assert_eq!(&s.paint_order()[..2], &[arm, forearm]);
        assert!(at(&s, arm) < at(&s, torso));
        assert!(!s.move_layer(arm, Toward::BackMost), "already at the back");

        // A child alone can move forward one place.
        assert!(s.move_layer(forearm, Toward::Front));
        assert_eq!(at(&s, forearm), 2);

        // A new bone appears in front.
        let (a, b) = (s.add_joint([0.0, 0.0]), s.add_joint([5.0, 5.0]));
        let new = s.add_part(a, b);
        assert_eq!(*s.paint_order().last().unwrap(), new);

        // Older projects without layers load as 0.
        let old: SkelPart = serde_json::from_str(r#"{"name": "a", "pivot": 0, "tip": 1, "parent": null}"#).unwrap();
        assert_eq!(old.layer, 0);
    }

    #[test]
    fn tree_order_lists_children_under_their_parent() {
        let s = Skeleton::person_front(RectI::new(0, 0, 100, 200));
        let tree = s.tree_order();
        assert_eq!(tree.len(), s.parts.len());
        assert_eq!(tree[0], (1, 0), "torso is the root");
        for (k, &(p, depth)) in tree.iter().enumerate() {
            if let Some(parent) = s.parts[p].parent {
                let pk = tree.iter().position(|&(q, _)| q == parent).unwrap();
                assert!(pk < k && tree[pk].1 + 1 == depth);
                assert!(tree[pk + 1..k].iter().all(|&(_, d)| d >= depth), "child sits inside its parent's subtree");
            }
        }
    }

    #[test]
    fn parents_are_guessed_and_kept_consistent() {
        let mut s = Skeleton::person_front(RectI::new(0, 0, 100, 200));
        let torso = 1;
        assert_eq!(s.draw_order()[0], torso, "root first");
        assert!(s.children(torso).count() == 5);

        // Grow a forearm from the left hand: its parent is the left arm.
        let hand = s.parts[2].tip;
        let tip = s.add_joint([0.0, 150.0]);
        let forearm = s.add_part(hand, tip);
        assert_eq!(s.parts[forearm].parent, Some(2));
        // A bone started on empty space attaches to the nearest bone.
        let a = s.add_joint([52.0, 120.0]);
        let b = s.add_joint([70.0, 120.0]);
        let tail = s.add_part(a, b);
        assert_eq!(s.parts[tail].parent, Some(torso));

        // No loops: the torso can't hang from its own descendant.
        assert!(!s.set_parent(torso, Some(forearm)));
        assert!(!s.set_parent(2, Some(2)));
        assert!(s.set_parent(tail, Some(4)));

        // Removing the left arm re-attaches the forearm to the torso.
        s.remove_part(2);
        let forearm = s.parts.iter().position(|p| p.name == "bone 1").unwrap();
        assert_eq!(s.parts[forearm].parent, Some(torso));
        let order = s.draw_order();
        assert_eq!(order.len(), s.parts.len());
        for (i, &p) in order.iter().enumerate() {
            if let Some(parent) = s.parts[p].parent {
                assert!(order[..i].contains(&parent), "parent drawn before child");
            }
        }
    }

    #[test]
    fn template_fits_bbox() {
        let s = Skeleton::person_front(RectI::new(100, 50, 200, 400));
        assert_eq!((s.joints.len(), s.parts.len()), (15, 10));
        let forearm = s.parts.iter().position(|p| p.name == "left forearm").unwrap();
        let upper = s.parts[forearm].parent.unwrap();
        assert_eq!(s.parts[upper].name, "left upper arm");
        assert_eq!(s.parts[forearm].pivot, s.parts[upper].tip, "elbow joint is shared");
        assert_eq!(s.joints[1].pos, [200.0, 146.0]);
    }
}
