//! Connected-component utilities on binary masks (4-connectivity, iterative).

use crate::mask::Mask;

/// Component labeling of the "on" pixels. Label 0 = off; component `i`
/// has label `i + 1` and size `sizes[i]`.
pub struct Labels {
    pub labels: Vec<u32>,
    pub sizes: Vec<usize>,
}

fn neighbors(i: usize, w: usize, h: usize) -> impl Iterator<Item = usize> {
    let (x, y) = (i % w, i / w);
    [
        (x > 0).then(|| i - 1),
        (x + 1 < w).then(|| i + 1),
        (y > 0).then(|| i - w),
        (y + 1 < h).then(|| i + w),
    ]
    .into_iter()
    .flatten()
}

pub fn label(mask: &Mask) -> Labels {
    let (w, h) = (mask.w as usize, mask.h as usize);
    let mut labels = vec![0u32; w * h];
    let mut sizes = Vec::new();
    let mut stack = Vec::new();
    for start in 0..w * h {
        if mask.data[start] == 0 || labels[start] != 0 {
            continue;
        }
        let id = sizes.len() as u32 + 1;
        let mut size = 0;
        labels[start] = id;
        stack.push(start);
        while let Some(i) = stack.pop() {
            size += 1;
            for n in neighbors(i, w, h) {
                if mask.data[n] != 0 && labels[n] == 0 {
                    labels[n] = id;
                    stack.push(n);
                }
            }
        }
        sizes.push(size);
    }
    Labels { labels, sizes }
}

/// Keep only components for which `keep(size)` returns true.
pub fn retain_components(mask: &Mask, keep: impl Fn(usize) -> bool) -> Mask {
    let l = label(mask);
    let keep: Vec<bool> = l.sizes.iter().map(|&s| keep(s)).collect();
    let mut out = Mask::new(mask.w, mask.h);
    for (o, &lab) in out.data.iter_mut().zip(&l.labels) {
        if lab != 0 && keep[lab as usize - 1] {
            *o = 255;
        }
    }
    out
}

pub fn remove_small(mask: &Mask, min_area: usize) -> Mask {
    retain_components(mask, |s| s >= min_area)
}

/// Pixels of `passable` reachable from the image border through `passable`.
pub fn flood_from_border(passable: &Mask) -> Mask {
    let (w, h) = (passable.w as usize, passable.h as usize);
    let mut out = Mask::new(passable.w, passable.h);
    let mut stack = Vec::new();
    let border = (0..w)
        .flat_map(|x| [x, (h - 1) * w + x])
        .chain((0..h).flat_map(|y| [y * w, y * w + w - 1]));
    for i in border {
        if passable.data[i] != 0 && out.data[i] == 0 {
            out.data[i] = 255;
            stack.push(i);
        }
    }
    while let Some(i) = stack.pop() {
        for n in neighbors(i, w, h) {
            if passable.data[n] != 0 && out.data[n] == 0 {
                out.data[n] = 255;
                stack.push(n);
            }
        }
    }
    out
}

/// Fill "off" regions that don't touch the border and are at most `max_area` pixels.
pub fn fill_holes(mask: &Mask, max_area: usize) -> Mask {
    let mut inverse = mask.clone();
    for v in &mut inverse.data {
        *v = 255 - *v;
    }
    let outside = flood_from_border(&inverse);
    let l = label(&inverse);
    let mut out = mask.clone();
    for (i, o) in out.data.iter_mut().enumerate() {
        let lab = l.labels[i];
        if lab != 0 && outside.data[i] == 0 && l.sizes[lab as usize - 1] <= max_area {
            *o = 255;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask_from(rows: &[&str]) -> Mask {
        let mut m = Mask::new(rows[0].len() as u32, rows.len() as u32);
        for (y, r) in rows.iter().enumerate() {
            for (x, c) in r.chars().enumerate() {
                m.set(x as u32, y as u32, c == '#');
            }
        }
        m
    }

    #[test]
    fn labels_and_removal() {
        let m = mask_from(&["##..#", "##...", ".....", "....#"]);
        let l = label(&m);
        assert_eq!(l.sizes, vec![4, 1, 1]);
        assert_eq!(remove_small(&m, 2).count(), 4);
    }

    #[test]
    fn holes_fill_but_outside_stays() {
        let m = mask_from(&[".....", ".###.", ".#.#.", ".###.", "....."]);
        let f = fill_holes(&m, 10);
        assert!(f.get(2, 2));
        assert!(!f.get(0, 0));
        assert!(!fill_holes(&m, 0).get(2, 2));
    }
}
