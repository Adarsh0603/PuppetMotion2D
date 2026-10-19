use crate::geom::RectI;
use image::RgbaImage;

/// Binary mask, one byte per pixel: 0 = off, 255 = on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mask {
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
}

impl Mask {
    pub fn new(w: u32, h: u32) -> Self {
        Self { w, h, data: vec![0; (w * h) as usize] }
    }

    #[inline]
    pub fn idx(&self, x: u32, y: u32) -> usize {
        (y * self.w + x) as usize
    }

    #[inline]
    pub fn get(&self, x: u32, y: u32) -> bool {
        self.data[self.idx(x, y)] != 0
    }

    #[inline]
    pub fn set(&mut self, x: u32, y: u32, on: bool) {
        let i = self.idx(x, y);
        self.data[i] = if on { 255 } else { 0 };
    }

    pub fn count(&self) -> usize {
        self.data.iter().filter(|&&v| v != 0).count()
    }

    /// Copy of this mask with everything outside `rect` cleared.
    pub fn and_rect(&self, rect: &RectI) -> Mask {
        let r = rect.clamp_to(self.w, self.h);
        let mut out = Mask::new(self.w, self.h);
        for y in r.y..r.bottom() {
            let row = (y as u32 * self.w) as usize;
            let (a, b) = (row + r.x as usize, row + r.right() as usize);
            out.data[a..b].copy_from_slice(&self.data[a..b]);
        }
        out
    }

    pub fn inverted(&self) -> Mask {
        Mask { w: self.w, h: self.h, data: self.data.iter().map(|v| 255 - v).collect() }
    }

    /// Pixels on in both masks.
    pub fn and(&self, other: &Mask) -> Mask {
        let data = self.data.iter().zip(&other.data).map(|(&a, &b)| a & b).collect();
        Mask { w: self.w, h: self.h, data }
    }

    /// Pixels on in `self` and off in `other`.
    pub fn and_not(&self, other: &Mask) -> Mask {
        let data = self.data.iter().zip(&other.data).map(|(&a, &b)| a & !b).collect();
        Mask { w: self.w, h: self.h, data }
    }

    /// Grow "on" regions by `r` pixels (square structuring element, separable, O(n)).
    pub fn dilate(&self, r: u32) -> Mask {
        if r == 0 {
            return self.clone();
        }
        let (w, h, r) = (self.w as usize, self.h as usize, r as usize);
        // Count of "on" pixels in a sliding window along one axis.
        let pass = |src: &[u8], len: usize, lines: usize, at: &dyn Fn(usize, usize) -> usize| {
            let mut out = vec![0u8; src.len()];
            for line in 0..lines {
                let mut count = 0usize;
                for i in 0..r.min(len) {
                    count += (src[at(line, i)] != 0) as usize;
                }
                for i in 0..len {
                    if i + r < len {
                        count += (src[at(line, i + r)] != 0) as usize;
                    }
                    if i > r {
                        count -= (src[at(line, i - r - 1)] != 0) as usize;
                    }
                    out[at(line, i)] = if count > 0 { 255 } else { 0 };
                }
            }
            out
        };
        let horiz = pass(&self.data, w, h, &|y, x| y * w + x);
        let data = pass(&horiz, h, w, &|x, y| y * w + x);
        Mask { w: self.w, h: self.h, data }
    }

    /// Tight bounding box of the "on" pixels, or `None` if the mask is empty.
    pub fn bbox(&self) -> Option<RectI> {
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
        for y in 0..self.h {
            for x in 0..self.w {
                if self.get(x, y) {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
        }
        (x0 != u32::MAX).then(|| RectI::from_corners(x0 as i32, y0 as i32, x1 as i32 + 1, y1 as i32 + 1))
    }

    /// Paint "on" pixels with `color` onto `dst` (which must match the mask size).
    pub fn paint_onto(&self, dst: &mut RgbaImage, color: [u8; 4]) {
        assert_eq!((dst.width(), dst.height()), (self.w, self.h));
        for (px, &m) in dst.pixels_mut().zip(&self.data) {
            if m != 0 {
                px.0 = color;
            }
        }
    }

    /// RGBA image where "on" pixels have `color` and the rest are transparent.
    pub fn to_rgba_tint(&self, color: [u8; 4]) -> RgbaImage {
        let mut img = RgbaImage::new(self.w, self.h);
        self.paint_onto(&mut img, color);
        img
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn and_rect_and_bbox() {
        let mut m = Mask::new(10, 10);
        m.data.fill(255);
        let r = m.and_rect(&RectI::new(2, 3, 4, 5));
        assert_eq!(r.count(), 20);
        assert_eq!(r.bbox(), Some(RectI::new(2, 3, 4, 5)));
        assert_eq!(Mask::new(4, 4).bbox(), None);
    }

    #[test]
    fn dilate_square() {
        let mut m = Mask::new(9, 9);
        m.set(4, 4, true);
        let d = m.dilate(2);
        assert_eq!(d.count(), 25);
        assert_eq!(d.bbox(), Some(RectI::new(2, 2, 5, 5)));
        let mut edge = Mask::new(5, 5);
        edge.set(0, 0, true);
        assert_eq!(edge.dilate(1).count(), 4);
    }
}
