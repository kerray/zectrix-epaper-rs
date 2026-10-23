//! Reference bundle builder, the same layout as `tools/screen-bundle.py` without fonts:
//! an optional title as a header row (flags bit 0, empty rectangle), then full-width
//! items of `ROW_H` px. A row's highlight strip is the row inverted. Instead of text, item
//! `i` gets a black bar `16 × (i + 1)` px wide, so rows differ.

#![allow(dead_code)]

use embedded_graphics_core::pixelcolor::BinaryColor;
use zectrix_screen::{MonoFrame, Rect};

pub const ROW_H: u16 = 40;

/// Encode a bundle (the format in `docs/INTERACTIVE.md`).
pub fn encode(flags: u8, selected: u8, base: &MonoFrame, rows: &[(Rect, Vec<u8>)]) -> Vec<u8> {
    let mut v = b"N4S1".to_vec();
    v.extend([1, flags, rows.len() as u8, selected]);
    v.extend_from_slice(base.as_bytes());
    for (r, strip) in rows {
        for n in [r.x, r.y, r.w, r.h] {
            v.extend(n.to_be_bytes());
        }
        v.extend_from_slice(strip);
    }
    v
}

/// The pixels of `r` (byte aligned) cut out of `f`, optionally inverted.
pub fn cut(f: &MonoFrame, r: Rect, invert: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for y in r.y as usize..(r.y + r.h) as usize {
        let line = &f.as_bytes()[y * 50 + r.x as usize / 8..y * 50 + (r.x + r.w) as usize / 8];
        out.extend(line.iter().map(|b| if invert { !b } else { *b }));
    }
    out
}

pub fn fill(f: &mut MonoFrame, r: Rect, c: BinaryColor) {
    for y in r.y as u32..r.bottom() {
        for x in r.x as u32..r.right() {
            f.set(x, y, c);
        }
    }
}

pub fn item_rect(title: bool, i: usize) -> Rect {
    let top = if title { ROW_H } else { 0 };
    Rect::new(0, top + i as u16 * ROW_H, 400, ROW_H)
}

/// Base frame and rows of the reference layout.
pub fn layout(title: bool, items: usize) -> (MonoFrame, Vec<(Rect, Vec<u8>)>) {
    let mut base = MonoFrame::new();
    let mut rows = Vec::new();
    if title {
        fill(&mut base, Rect::new(8, 12, 120, 16), BinaryColor::On);
        fill(&mut base, Rect::new(0, ROW_H - 2, 400, 2), BinaryColor::On);
        rows.push((Rect::new(0, 0, 0, 0), Vec::new()));
    }
    for i in 0..items {
        let r = item_rect(title, i);
        fill(
            &mut base,
            Rect::new(8, r.y + 12, 16 * (i as u16 + 1), 16),
            BinaryColor::On,
        );
    }
    for i in 0..items {
        let r = item_rect(title, i);
        rows.push((r, cut(&base, r, true)));
    }
    (base, rows)
}

/// A whole reference bundle. `selected` is a row index (the header counts as row 0).
pub fn bundle(title: bool, items: usize, selected: u8) -> Vec<u8> {
    let (base, rows) = layout(title, items);
    encode(title as u8, selected, &base, &rows)
}

/// What the panel should show: the base with row `sel` inverted.
pub fn expected(title: bool, items: usize, sel: Option<usize>) -> MonoFrame {
    let (mut f, rows) = layout(title, items);
    if let Some(s) = sel {
        let r = rows[s].0;
        for y in r.y as usize..r.bottom() as usize {
            for b in
                &mut f.as_bytes_mut()[y * 50 + r.x as usize / 8..y * 50 + r.right() as usize / 8]
            {
                *b = !*b;
            }
        }
    }
    f
}

/// Encode a patch.
pub fn patch(r: Rect, planes: &[&[u8]]) -> Vec<u8> {
    let mut v = Vec::new();
    for n in [r.x, r.y, r.w, r.h] {
        v.extend(n.to_be_bytes());
    }
    for p in planes {
        v.extend_from_slice(p);
    }
    v
}
