//! Byte-aligned copies between 1 bpp buffers that each cover a rectangle of the panel.
//!
//! Every rectangle here has `x` and `w` multiples of 8 (the bundle and patch parsers
//! check that), so a copy is whole bytes per line. A buffer covering `r` has
//! `r.w / 8` bytes per line and `r.h` lines, top line first, MSB = leftmost pixel.

use ssd2683::Rect;

/// The overlap of two rectangles, `None` if they do not overlap.
pub(crate) fn intersect(a: &Rect, b: &Rect) -> Option<Rect> {
    let x0 = a.x.max(b.x) as u32;
    let y0 = a.y.max(b.y) as u32;
    let x1 = a.right().min(b.right());
    let y1 = a.bottom().min(b.bottom());
    (x0 < x1 && y0 < y1)
        .then(|| Rect::new(x0 as u16, y0 as u16, (x1 - x0) as u16, (y1 - y0) as u16))
}

/// Copy the pixels of `area` from `src` (covering `src_r`) to `dst` (covering `dst_r`).
/// Only the part of `area` inside both rectangles is copied.
pub(crate) fn blit(dst: &mut [u8], dst_r: &Rect, src: &[u8], src_r: &Rect, area: &Rect) {
    let Some(a) = intersect(area, dst_r).and_then(|a| intersect(&a, src_r)) else {
        return;
    };
    let n = a.w as usize / 8;
    let (ds, ss) = (dst_r.w as usize / 8, src_r.w as usize / 8);
    let (dx, sx) = ((a.x - dst_r.x) as usize / 8, (a.x - src_r.x) as usize / 8);
    for y in a.y..a.y + a.h {
        let d = (y - dst_r.y) as usize * ds + dx;
        let s = (y - src_r.y) as usize * ss + sx;
        dst[d..d + n].copy_from_slice(&src[s..s + n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersect_cases() {
        let a = Rect::new(0, 0, 16, 10);
        assert_eq!(
            intersect(&a, &Rect::new(8, 5, 16, 10)),
            Some(Rect::new(8, 5, 8, 5))
        );
        assert_eq!(intersect(&a, &Rect::new(16, 0, 8, 10)), None);
        assert_eq!(intersect(&a, &Rect::new(0, 10, 8, 1)), None);
    }

    #[test]
    fn blit_between_offsets() {
        // src covers (8,2) 16x2, dst covers (0,0) 24x4.
        let src = [1, 2, 3, 4];
        let mut dst = [0u8; 12];
        blit(
            &mut dst,
            &Rect::new(0, 0, 24, 4),
            &src,
            &Rect::new(8, 2, 16, 2),
            &Rect::new(0, 0, 400, 300),
        );
        assert_eq!(dst, [0, 0, 0, 0, 0, 0, 0, 1, 2, 0, 3, 4]);
    }
}
