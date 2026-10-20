//! Spreading per-row image work over the CPU cores (standard library only).

use std::ops::Range;

/// Run `f(y, row)` for each row `y` in `rows` of an image `w` pixels wide
/// stored row by row in `out`, splitting the rows between the CPU cores.
/// `row` is that row's slice of `out`.
pub fn for_rows<T: Send>(out: &mut [T], w: usize, rows: Range<usize>, f: impl Fn(usize, &mut [T]) + Sync) {
    let n = rows.len();
    if n == 0 || w == 0 {
        return;
    }
    let threads = std::thread::available_parallelism().map_or(1, |t| t.get()).min(n);
    let per = n.div_ceil(threads);
    let slice = &mut out[rows.start * w..rows.end * w];
    std::thread::scope(|s| {
        for (k, chunk) in slice.chunks_mut(per * w).enumerate() {
            let f = &f;
            let first = rows.start + k * per;
            s.spawn(move || {
                for (i, row) in chunk.chunks_mut(w).enumerate() {
                    f(first + i, row);
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_in_range_is_visited_once_with_its_own_slice() {
        let (w, h) = (7, 50);
        let mut out = vec![0usize; w * h];
        for_rows(&mut out, w, 3..41, |y, row| {
            for (x, v) in row.iter_mut().enumerate() {
                *v += y * 100 + x + 1;
            }
        });
        for y in 0..h {
            for x in 0..w {
                let want = if (3..41).contains(&y) { y * 100 + x + 1 } else { 0 };
                assert_eq!(out[y * w + x], want, "({x},{y})");
            }
        }
    }
}
