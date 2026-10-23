//! Rectangles in panel pixels, used for partial refresh windows.

use crate::{HEIGHT, WIDTH};

/// A rectangle in panel pixels: top-left corner `(x, y)`, size `w × h`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Rect {
    pub const fn new(x: u16, y: u16, w: u16, h: u16) -> Self {
        Self { x, y, w, h }
    }

    /// The whole 400×300 panel.
    pub const fn full() -> Self {
        Self::new(0, 0, WIDTH as u16, HEIGHT as u16)
    }

    /// One past the right edge.
    pub const fn right(&self) -> u32 {
        self.x as u32 + self.w as u32
    }

    /// One past the bottom edge.
    pub const fn bottom(&self) -> u32 {
        self.y as u32 + self.h as u32
    }

    pub const fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// Not empty and entirely on the panel.
    pub const fn is_valid(&self) -> bool {
        !self.is_empty() && self.right() <= WIDTH && self.bottom() <= HEIGHT
    }

    /// The window the controller actually refreshes: x rounded down to a multiple of 8,
    /// the right edge rounded up to a multiple of 8 (at most 400). y is unchanged.
    /// Only meaningful for a valid rectangle.
    pub const fn aligned(&self) -> Self {
        let x0 = self.x & !7;
        let mut end = (self.right() + 7) & !7;
        if end > WIDTH {
            end = WIDTH;
        }
        Self::new(x0, self.y, (end as u16).saturating_sub(x0), self.h)
    }

    /// The smallest rectangle that contains both. An empty rectangle is ignored.
    pub fn union(&self, other: &Self) -> Self {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let r = self.right().max(other.right());
        let b = self.bottom().max(other.bottom());
        Self::new(x, y, (r - x as u32) as u16, (b - y as u32) as u16)
    }
}
