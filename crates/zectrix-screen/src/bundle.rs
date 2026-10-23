//! The `application/x-note4-screen` bundle and the [`Screen`] that works on it in place.
//!
//! ```text
//! offset  size
//! 0       4     "N4S1"
//! 4       1     version = 1
//! 5       1     flags: bit 0 = row 0 is a non-selectable header; other bits must be 0
//! 6       1     n_rows (at most MAX_ROWS)
//! 7       1     selected: index of the selected row (0 when no row is selectable)
//! 8       15000 base frame: 1 bpp, 1 = white, MSB = left, 50 B per line, top line first,
//!               every row drawn unselected
//! 15008   …     n_rows × { u16 x, u16 y, u16 w, u16 h   big-endian
//!                          w/8 × h B highlight strip: the row drawn selected, same
//!                          layout as a frame but w/8 B per line }
//! ```
//!
//! * `x` and `w` are multiples of 8 and the rectangle lies inside 400×300.
//! * Every selectable row has a non-empty rectangle. A header row (flags bit 0) may have
//!   an empty one (`w` or `h` = 0, no strip bytes); its strip is never shown.
//! * The body ends right after the last strip: no trailing bytes.
//!
//! Rows are not required to be disjoint or ordered, but the cursor moves in index order.

use ssd2683::{MonoFrame, Rect, framebuffer::MONO_FRAME_BYTES};

use crate::blit::{blit, intersect};
use crate::patch::{Patch, RectError, read_rect};

pub const MAGIC: &[u8; 4] = b"N4S1";
pub const VERSION: u8 = 1;
/// Flags bit 0: row 0 is a header and cannot be selected.
pub const FLAG_HEADER_ROW: u8 = 0x01;
/// Bytes before the base frame.
pub const HEADER_LEN: usize = 8;
/// Bytes of the per-row rectangle before its strip.
pub const ROW_HEADER_LEN: usize = 8;
/// Most rows a bundle may have (the row table is a fixed array).
pub const MAX_ROWS: usize = 32;

const BASE: core::ops::Range<usize> = HEADER_LEN..HEADER_LEN + MONO_FRAME_BYTES;
const FULL: Rect = Rect::full();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BundleError {
    /// Shorter than the header and the base frame.
    TooShort,
    /// The first four bytes are not `N4S1`.
    BadMagic,
    UnsupportedVersion(u8),
    /// Flag bits other than bit 0 are set.
    UnknownFlags(u8),
    /// More than [`MAX_ROWS`] rows.
    TooManyRows(u8),
    /// `selected` is not a selectable row (or not 0 when there is none).
    BadSelected(u8),
    /// Row `n`'s rectangle or strip goes past the end of the body.
    RowTruncated(u8),
    /// Row `n` has `x` or `w` not a multiple of 8.
    RowMisaligned(u8),
    /// Row `n` is not inside the panel, or is empty but selectable.
    RowOutOfBounds(u8),
    /// Bytes after the last strip; the total length is given.
    TrailingBytes(usize),
}

/// One row: its rectangle on the panel and where its strip is in the bundle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Row {
    rect: Rect,
    strip: usize,
}

const NO_ROW: Row = Row {
    rect: Rect::new(0, 0, 0, 0),
    strip: 0,
};

/// The validated structure of a bundle: flags and the row table, without the bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    flags: u8,
    n_rows: u8,
    selected: u8,
    len: usize,
    rows: [Row; MAX_ROWS],
}

impl Layout {
    /// Check a whole bundle. `body` must be exactly the bundle (e.g. the first
    /// `Content-Length` bytes of the download buffer).
    pub fn parse(body: &[u8]) -> Result<Self, BundleError> {
        if body.len() < BASE.end {
            return Err(BundleError::TooShort);
        }
        if &body[..4] != MAGIC {
            return Err(BundleError::BadMagic);
        }
        if body[4] != VERSION {
            return Err(BundleError::UnsupportedVersion(body[4]));
        }
        let (flags, n_rows, selected) = (body[5], body[6], body[7]);
        if flags & !FLAG_HEADER_ROW != 0 {
            return Err(BundleError::UnknownFlags(flags));
        }
        if n_rows as usize > MAX_ROWS {
            return Err(BundleError::TooManyRows(n_rows));
        }
        let first = flags & FLAG_HEADER_ROW;
        let mut rows = [NO_ROW; MAX_ROWS];
        let mut at = BASE.end;
        for i in 0..n_rows {
            if body.len() < at + ROW_HEADER_LEN {
                return Err(BundleError::RowTruncated(i));
            }
            let (rect, n) = read_rect(body, at).map_err(|e| match e {
                RectError::Misaligned => BundleError::RowMisaligned(i),
                RectError::OutOfBounds => BundleError::RowOutOfBounds(i),
            })?;
            if rect.is_empty() && i >= first {
                return Err(BundleError::RowOutOfBounds(i));
            }
            at += ROW_HEADER_LEN;
            if body.len() < at + n {
                return Err(BundleError::RowTruncated(i));
            }
            rows[i as usize] = Row { rect, strip: at };
            at += n;
        }
        if at != body.len() {
            return Err(BundleError::TrailingBytes(body.len()));
        }
        let selectable = n_rows > first;
        if (selectable && !(first..n_rows).contains(&selected)) || (!selectable && selected != 0) {
            return Err(BundleError::BadSelected(selected));
        }
        Ok(Self {
            flags,
            n_rows,
            selected,
            len: at,
            rows,
        })
    }

    pub fn flags(&self) -> u8 {
        self.flags
    }

    /// Row 0 is a non-selectable header.
    pub fn has_header(&self) -> bool {
        self.flags & FLAG_HEADER_ROW != 0
    }

    pub fn n_rows(&self) -> usize {
        self.n_rows as usize
    }

    /// The row selected when the bundle arrived.
    pub fn initial_selected(&self) -> usize {
        self.selected as usize
    }

    /// Total length of the bundle in bytes.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Always false: a bundle holds at least the header and the base frame.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Rectangle of row `i`.
    pub fn row_rect(&self, i: usize) -> Option<Rect> {
        (i < self.n_rows()).then(|| self.rows[i].rect)
    }

    /// Indices of the rows the cursor can be on.
    pub fn selectable(&self) -> core::ops::Range<usize> {
        let first = (self.flags & FLAG_HEADER_ROW) as usize;
        first.min(self.n_rows())..self.n_rows()
    }

    fn strip_range(&self, i: usize) -> core::ops::Range<usize> {
        let r = &self.rows[i];
        r.strip..r.strip + r.rect.w as usize / 8 * r.rect.h as usize
    }
}

/// Direction of a cursor move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Up,
    Down,
}

/// How the two rows of a cursor move are refreshed. Which is faster is one of the
/// open measurements in `docs/INTERACTIVE.md`, so the firmware chooses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Merge {
    /// Two partial refreshes, one per row.
    Never,
    /// One partial refresh of the smallest rectangle that holds both rows.
    Always,
    /// One refresh if the rows are at most this many pixels apart (vertically or
    /// horizontally), otherwise two. `IfGapAtMost(0)` merges touching rows only.
    IfGapAtMost(u16),
}

/// What to refresh after a change, in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dirty {
    None,
    One(Rect),
    Two(Rect, Rect),
}

impl Dirty {
    pub fn rects(&self) -> impl Iterator<Item = Rect> {
        let (a, b) = match *self {
            Dirty::None => (None, None),
            Dirty::One(a) => (Some(a), None),
            Dirty::Two(a, b) => (Some(a), Some(b)),
        };
        a.into_iter().chain(b)
    }

    /// The smallest rectangle that holds everything, for a single refresh.
    pub fn bounds(&self) -> Option<Rect> {
        self.rects().reduce(|a, b| a.union(&b))
    }

    pub fn is_none(&self) -> bool {
        matches!(self, Dirty::None)
    }
}

/// Gap in pixels between two rectangles (0 if they touch or overlap).
fn gap(a: &Rect, b: &Rect) -> u32 {
    let dy = (b.y as u32)
        .saturating_sub(a.bottom())
        .max((a.y as u32).saturating_sub(b.bottom()));
    let dx = (b.x as u32)
        .saturating_sub(a.right())
        .max((a.x as u32).saturating_sub(b.right()));
    dx.max(dy)
}

/// A screen bundle in a mutable buffer, plus the current selection.
///
/// The bundle stays where it was downloaded. Patches are written into it in place (base
/// frame and strips), so the bundle always describes the current screen and a later
/// cursor move cannot bring back stale pixels. The methods that change the screen also
/// write the new pixels into the caller's target frame and return the rectangles to
/// refresh; the caller passes those to the driver (or does a full refresh, see
/// [`RefreshPolicy`](crate::RefreshPolicy)).
pub struct Screen<'a> {
    buf: &'a mut [u8],
    layout: Layout,
    selected: usize,
}

impl<'a> Screen<'a> {
    /// Validate `body` (exactly the bundle bytes) and take it over.
    pub fn new(body: &'a mut [u8]) -> Result<Self, BundleError> {
        let layout = Layout::parse(body)?;
        let selected = layout.initial_selected();
        Ok(Self {
            buf: body,
            layout,
            selected,
        })
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// The selected row, `None` on a screen without selectable rows.
    pub fn selected(&self) -> Option<usize> {
        (!self.layout.selectable().is_empty()).then_some(self.selected)
    }

    /// The base frame (15,000 B, every row unselected).
    pub fn base(&self) -> &[u8] {
        &self.buf[BASE]
    }

    /// The highlight strip of row `i` (`w/8` B per line).
    pub fn strip(&self, i: usize) -> Option<&[u8]> {
        (i < self.layout.n_rows()).then(|| &self.buf[self.layout.strip_range(i)])
    }

    /// Write the whole screen into `frame`: the base with the selected row highlighted.
    /// Use it for the first (full) refresh of a new screen.
    pub fn compose(&self, frame: &mut MonoFrame) {
        self.compose_rect(frame, FULL);
    }

    /// Write the part of the screen inside `area` into `frame`.
    pub fn compose_rect(&self, frame: &mut MonoFrame, area: Rect) {
        let dst = frame.as_bytes_mut();
        blit(dst, &FULL, self.base(), &FULL, &area);
        if let Some(i) = self.selected() {
            let rect = self.layout.rows[i].rect;
            blit(
                dst,
                &FULL,
                &self.buf[self.layout.strip_range(i)],
                &rect,
                &area,
            );
        }
    }

    /// Select row `i` and update `frame`. Returns what to refresh: [`Dirty::None`] if `i`
    /// is already selected, not selectable, or out of range.
    pub fn select(&mut self, i: usize, frame: &mut MonoFrame, merge: Merge) -> Dirty {
        let Some(old) = self.selected() else {
            return Dirty::None;
        };
        if i == old || !self.layout.selectable().contains(&i) {
            return Dirty::None;
        }
        let (a, b) = (self.layout.rows[old].rect, self.layout.rows[i].rect);
        self.selected = i;
        // The old row back from the base, then the new row from its strip; if the rows
        // overlap, the new highlight wins.
        self.compose_rect(frame, a);
        self.compose_rect(frame, b);
        let one = match merge {
            Merge::Never => false,
            Merge::Always => true,
            Merge::IfGapAtMost(px) => gap(&a, &b) <= px as u32,
        };
        if one || a == b {
            Dirty::One(a.union(&b))
        } else {
            Dirty::Two(a, b)
        }
    }

    /// Move the cursor one selectable row up or down, wrapping around and skipping the
    /// header. Returns [`Dirty::None`] if there is nothing to move to.
    pub fn move_cursor(&mut self, dir: Move, frame: &mut MonoFrame, merge: Merge) -> Dirty {
        let Some(cur) = self.selected() else {
            return Dirty::None;
        };
        let sel = self.layout.selectable();
        let n = sel.len();
        let k = cur - sel.start;
        let next = match dir {
            Move::Down => (k + 1) % n,
            Move::Up => (k + n - 1) % n,
        };
        self.select(sel.start + next, frame, merge)
    }

    /// Apply a patch: its pixels replace that area in the base frame, and its highlighted
    /// plane (or its pixels, if it has none) replaces that area in every strip it overlaps.
    /// `frame` gets the new pixels of the area. Returns the rectangle to refresh.
    pub fn apply_patch(&mut self, patch: &Patch, frame: &mut MonoFrame) -> Rect {
        let pr = patch.rect();
        blit(&mut self.buf[BASE], &FULL, patch.pixels(), &pr, &pr);
        for i in 0..self.layout.n_rows() {
            let rect = self.layout.rows[i].rect;
            if intersect(&rect, &pr).is_some() {
                let range = self.layout.strip_range(i);
                blit(&mut self.buf[range], &rect, patch.highlighted(), &pr, &pr);
            }
        }
        self.compose_rect(frame, pr);
        pr
    }
}
