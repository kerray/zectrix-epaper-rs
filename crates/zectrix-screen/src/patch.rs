//! The `application/x-note4-patch` body: a rectangle of new pixels for the current screen.
//!
//! ```text
//! u16 x, u16 y, u16 w, u16 h        big-endian; x and w multiples of 8, inside 400x300
//! w/8 × h B  pixels                 1 bpp, 1 = white, MSB = left, top line first
//! [w/8 × h B highlighted pixels]    optional: the same area as drawn when selected
//! ```
//!
//! The body is exactly 8 + n or 8 + 2n bytes (n = w/8 × h); its length says whether the
//! highlighted plane is there. [`Screen::apply_patch`](crate::Screen::apply_patch) writes
//! the first plane into the base frame and, for every row strip the rectangle overlaps,
//! the highlighted plane (or the first plane when there is none) into that strip.

use ssd2683::{HEIGHT, Rect, WIDTH};

/// Bytes before the pixels.
pub const PATCH_HEADER_LEN: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatchError {
    /// Shorter than the 8-byte header.
    TooShort,
    /// `x` or `w` is not a multiple of 8.
    Misaligned,
    /// Empty, or not inside the 400×300 panel.
    OutOfBounds,
    /// The body is neither 8 + n nor 8 + 2n bytes (n = w/8 × h); the length is given.
    BadLength(usize),
}

/// A parsed patch, borrowing the body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Patch<'a> {
    rect: Rect,
    pixels: &'a [u8],
    highlighted: Option<&'a [u8]>,
}

pub(crate) fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

/// Read and check a rectangle at `at` (8 bytes, big-endian x, y, w, h).
/// Returns the rectangle and its strip size in bytes. An empty rectangle is allowed here;
/// the callers decide whether it is.
pub(crate) fn read_rect(b: &[u8], at: usize) -> Result<(Rect, usize), RectError> {
    let r = Rect::new(
        be16(b, at),
        be16(b, at + 2),
        be16(b, at + 4),
        be16(b, at + 6),
    );
    if !r.x.is_multiple_of(8) || !r.w.is_multiple_of(8) {
        return Err(RectError::Misaligned);
    }
    if r.right() > WIDTH || r.bottom() > HEIGHT {
        return Err(RectError::OutOfBounds);
    }
    Ok((r, r.w as usize / 8 * r.h as usize))
}

pub(crate) enum RectError {
    Misaligned,
    OutOfBounds,
}

impl<'a> Patch<'a> {
    pub fn parse(body: &'a [u8]) -> Result<Self, PatchError> {
        if body.len() < PATCH_HEADER_LEN {
            return Err(PatchError::TooShort);
        }
        let (rect, n) = read_rect(body, 0).map_err(|e| match e {
            RectError::Misaligned => PatchError::Misaligned,
            RectError::OutOfBounds => PatchError::OutOfBounds,
        })?;
        if rect.is_empty() {
            return Err(PatchError::OutOfBounds);
        }
        let px = &body[PATCH_HEADER_LEN..];
        let (pixels, highlighted) = if px.len() == n {
            (px, None)
        } else if px.len() == 2 * n {
            (&px[..n], Some(&px[n..]))
        } else {
            return Err(PatchError::BadLength(body.len()));
        };
        Ok(Self {
            rect,
            pixels,
            highlighted,
        })
    }

    /// The area the patch replaces.
    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// The unselected pixels (for the base frame), `w/8` bytes per line.
    pub fn pixels(&self) -> &'a [u8] {
        self.pixels
    }

    /// The pixels for row strips: the highlighted plane if the patch has one, otherwise the
    /// same as [`pixels`](Self::pixels).
    pub fn highlighted(&self) -> &'a [u8] {
        self.highlighted.unwrap_or(self.pixels)
    }

    /// Whether the body carried a separate highlighted plane.
    pub fn has_highlighted(&self) -> bool {
        self.highlighted.is_some()
    }
}
