/// Axis-aligned rectangle in image pixel coordinates. `w`/`h` may be zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct RectI {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl RectI {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w: w.max(0), h: h.max(0) }
    }

    /// Rectangle spanning two arbitrary corner points (inclusive-exclusive).
    pub fn from_corners(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        let (ax, bx) = (x0.min(x1), x0.max(x1));
        let (ay, by) = (y0.min(y1), y0.max(y1));
        Self::new(ax, ay, bx - ax, by - ay)
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    pub fn area(&self) -> i64 {
        self.w as i64 * self.h as i64
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    pub fn intersect(&self, other: &RectI) -> RectI {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = self.right().min(other.right());
        let y1 = self.bottom().min(other.bottom());
        if x1 <= x0 || y1 <= y0 {
            RectI::new(x0, y0, 0, 0)
        } else {
            RectI::new(x0, y0, x1 - x0, y1 - y0)
        }
    }

    pub fn clamp_to(&self, w: u32, h: u32) -> RectI {
        self.intersect(&RectI::new(0, 0, w as i32, h as i32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_normalize() {
        assert_eq!(RectI::from_corners(10, 8, 2, 3), RectI::new(2, 3, 8, 5));
    }

    #[test]
    fn intersect_and_clamp() {
        let a = RectI::new(-5, -5, 20, 20);
        assert_eq!(a.clamp_to(10, 12), RectI::new(0, 0, 10, 12));
        assert!(RectI::new(0, 0, 5, 5).intersect(&RectI::new(6, 6, 2, 2)).is_empty());
    }
}
