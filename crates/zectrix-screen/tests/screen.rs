//! Bundle and patch parsing, cursor moves and patches on a reference bundle, and a
//! simulated session (panel + shadow + refresh policy).

mod common;

use common::*;
use embedded_graphics_core::pixelcolor::BinaryColor;
use zectrix_screen::{
    BundleError, Dirty, Layout, Merge, MonoFrame, Move, Patch, PatchError, Rect, RefreshKind,
    RefreshPolicy, Screen, Update,
};

const BASE_END: usize = 8 + 15_000;

#[test]
fn reference_bundle_layout() {
    let mut b = bundle(true, 5, 2);
    assert_eq!(b.len(), BASE_END + 8 + 5 * (8 + 2000));
    let l = Layout::parse(&b).unwrap();
    assert!(l.has_header());
    assert_eq!(l.n_rows(), 6);
    assert_eq!(l.selectable(), 1..6);
    assert_eq!(l.initial_selected(), 2);
    assert_eq!(l.len(), b.len());
    assert_eq!(l.row_rect(0), Some(Rect::new(0, 0, 0, 0)));
    assert_eq!(l.row_rect(1), Some(Rect::new(0, 40, 400, 40)));
    assert_eq!(l.row_rect(5), Some(Rect::new(0, 200, 400, 40)));
    assert_eq!(l.row_rect(6), None);

    let s = Screen::new(&mut b).unwrap();
    assert_eq!(s.selected(), Some(2));
    assert_eq!(s.strip(0).unwrap().len(), 0);
    assert_eq!(s.strip(3).unwrap().len(), 2000);
    assert_eq!(s.base().len(), 15_000);
    let mut f = MonoFrame::new();
    s.compose(&mut f);
    assert_eq!(f.as_bytes(), expected(true, 5, Some(2)).as_bytes());
}

#[test]
fn cursor_wraps_and_skips_header() {
    let mut b = bundle(true, 3, 1);
    let mut s = Screen::new(&mut b).unwrap();
    let mut f = MonoFrame::new();
    s.compose(&mut f);
    let r = |i| item_rect(true, i - 1);
    let mut seen = Vec::new();
    for dir in [Move::Down, Move::Down, Move::Down, Move::Up, Move::Up] {
        let d = s.move_cursor(dir, &mut f, Merge::Never);
        let sel = s.selected().unwrap();
        seen.push(sel);
        assert_eq!(
            f.as_bytes(),
            expected(true, 3, Some(sel)).as_bytes(),
            "after {dir:?}"
        );
        assert!(matches!(d, Dirty::Two(_, new) if new == r(sel)));
    }
    assert_eq!(seen, [2, 3, 1, 3, 2]);
}

#[test]
fn cursor_without_header_uses_row_zero() {
    let mut b = bundle(false, 3, 0);
    let mut s = Screen::new(&mut b).unwrap();
    let mut f = MonoFrame::new();
    assert_eq!(
        s.move_cursor(Move::Up, &mut f, Merge::Never),
        Dirty::Two(item_rect(false, 0), item_rect(false, 2))
    );
    assert_eq!(s.selected(), Some(2));
}

#[test]
fn dirty_rects_cover_every_changed_pixel() {
    let mut b = bundle(true, 6, 1);
    let mut s = Screen::new(&mut b).unwrap();
    let mut f = MonoFrame::new();
    s.compose(&mut f);
    let mut shadow = MonoFrame::new();
    shadow.copy_from(&f);
    for i in [4, 5, 1, 2] {
        let d = s.select(i, &mut f, Merge::Never);
        let changed = shadow.dirty_rect(&f).unwrap();
        let bounds = d.bounds().unwrap();
        assert_eq!(changed.union(&bounds), bounds, "row {i}");
        shadow.copy_from(&f);
    }
}

#[test]
fn merge_modes() {
    let mut b = bundle(true, 6, 1);
    let mut s = Screen::new(&mut b).unwrap();
    let mut f = MonoFrame::new();
    let (r1, r2, r6) = (item_rect(true, 0), item_rect(true, 1), item_rect(true, 5));
    assert_eq!(
        s.select(2, &mut f, Merge::Always),
        Dirty::One(r1.union(&r2))
    );
    // Adjacent rows (gap 0) merge with IfGapAtMost(0); rows 2 and 6 are 120 px apart.
    assert_eq!(
        s.select(1, &mut f, Merge::IfGapAtMost(0)),
        Dirty::One(r1.union(&r2))
    );
    assert_eq!(
        s.select(6, &mut f, Merge::IfGapAtMost(119)),
        Dirty::Two(r1, r6)
    );
    assert_eq!(
        s.select(1, &mut f, Merge::IfGapAtMost(160)),
        Dirty::One(Rect::new(0, 40, 400, 240))
    );
    assert_eq!(
        Dirty::Two(r1, r6).bounds(),
        Some(Rect::new(0, 40, 400, 240))
    );
    assert_eq!(Dirty::Two(r1, r6).rects().collect::<Vec<_>>(), [r1, r6]);
    assert_eq!(Dirty::None.rects().count(), 0);
}

#[test]
fn select_rejects_header_same_and_out_of_range() {
    let mut b = bundle(true, 3, 1);
    let mut s = Screen::new(&mut b).unwrap();
    let mut f = MonoFrame::new();
    s.compose(&mut f);
    let before = *f.as_bytes();
    for i in [0, 1, 4, 99] {
        assert_eq!(s.select(i, &mut f, Merge::Never), Dirty::None);
    }
    assert_eq!(s.selected(), Some(1));
    assert_eq!(f.as_bytes(), &before);
}

#[test]
fn screens_without_selectable_rows() {
    // A static screen (no rows), a header only, and a single row (nothing to move to).
    for (title, items) in [(false, 0), (true, 0), (false, 1)] {
        let mut b = bundle(title, items, 0);
        let mut s = Screen::new(&mut b).unwrap();
        let mut f = MonoFrame::new();
        assert_eq!(s.move_cursor(Move::Down, &mut f, Merge::Never), Dirty::None);
        s.compose(&mut f);
        let sel = (items == 1).then_some(0);
        assert_eq!(s.selected(), sel);
        assert_eq!(f.as_bytes(), expected(title, items, sel).as_bytes());
    }
    assert_eq!(bundle(false, 0, 0).len(), BASE_END);
}

fn err(b: &[u8]) -> BundleError {
    Layout::parse(b).unwrap_err()
}

fn with_row(rect: [u16; 4], strip: usize) -> Vec<u8> {
    let mut b = bundle(false, 0, 0);
    b[6] = 1;
    for n in rect {
        b.extend(n.to_be_bytes());
    }
    b.resize(b.len() + strip, 0xFF);
    b
}

#[test]
fn bundle_errors() {
    let good = bundle(true, 2, 1);
    assert_eq!(err(&good[..BASE_END - 1]), BundleError::TooShort);
    let mut b = good.clone();
    b[3] = b'2';
    assert_eq!(err(&b), BundleError::BadMagic);
    let mut b = good.clone();
    b[4] = 2;
    assert_eq!(err(&b), BundleError::UnsupportedVersion(2));
    let mut b = good.clone();
    b[5] = 0x03;
    assert_eq!(err(&b), BundleError::UnknownFlags(3));
    let mut b = bundle(false, 0, 0);
    b[6] = 33;
    assert_eq!(err(&b), BundleError::TooManyRows(33));
    let mut b = good.clone();
    b[7] = 0; // the header
    assert_eq!(err(&b), BundleError::BadSelected(0));
    b[7] = 3;
    assert_eq!(err(&b), BundleError::BadSelected(3));
    let mut b = bundle(false, 0, 0);
    b[7] = 1;
    assert_eq!(err(&b), BundleError::BadSelected(1));
    // Truncated in the last strip, in a row header, and trailing bytes.
    assert_eq!(err(&good[..good.len() - 1]), BundleError::RowTruncated(2));
    assert_eq!(err(&good[..BASE_END + 8 + 4]), BundleError::RowTruncated(1));
    let mut b = good.clone();
    b.push(0);
    assert_eq!(err(&b), BundleError::TrailingBytes(good.len() + 1));
    // Rectangles.
    assert_eq!(
        err(&with_row([4, 0, 8, 1], 1)),
        BundleError::RowMisaligned(0)
    );
    assert_eq!(
        err(&with_row([0, 0, 12, 1], 2)),
        BundleError::RowMisaligned(0)
    );
    assert_eq!(
        err(&with_row([392, 0, 16, 1], 2)),
        BundleError::RowOutOfBounds(0)
    );
    assert_eq!(
        err(&with_row([0, 299, 8, 2], 2)),
        BundleError::RowOutOfBounds(0)
    );
    assert_eq!(
        err(&with_row([0, 0, 0, 0], 0)),
        BundleError::RowOutOfBounds(0)
    ); // empty, selectable
    assert!(Layout::parse(&with_row([392, 299, 8, 1], 1)).is_ok());
    // An empty header with a non-zero position is fine; a non-empty header too.
    let mut b = with_row([0, 0, 400, 1], 50);
    b[5] = 1;
    assert!(Layout::parse(&b).is_ok());
    // 32 rows is the limit.
    let mut b = bundle(false, 0, 0);
    b[6] = 32;
    for i in 0..32u16 {
        for n in [0, i, 8, 1] {
            b.extend(n.to_be_bytes());
        }
        b.push(0);
    }
    assert_eq!(Layout::parse(&b).unwrap().n_rows(), 32);
}

#[test]
fn patch_parse() {
    let r = Rect::new(8, 10, 16, 3);
    let a = [1u8, 2, 3, 4, 5, 6];
    let h = [9u8; 6];
    let p1 = patch(r, &[&a]);
    let p = Patch::parse(&p1).unwrap();
    assert_eq!(
        (p.rect(), p.pixels(), p.highlighted(), p.has_highlighted()),
        (r, &a[..], &a[..], false)
    );
    let p2 = patch(r, &[&a, &h]);
    let p = Patch::parse(&p2).unwrap();
    assert_eq!(
        (p.pixels(), p.highlighted(), p.has_highlighted()),
        (&a[..], &h[..], true)
    );

    assert_eq!(Patch::parse(&p1[..7]), Err(PatchError::TooShort));
    assert_eq!(Patch::parse(&p1[..13]), Err(PatchError::BadLength(13)));
    assert_eq!(
        Patch::parse(&patch(r, &[&a, &a, &a])),
        Err(PatchError::BadLength(26))
    );
    assert_eq!(
        Patch::parse(&patch(Rect::new(4, 0, 8, 1), &[&[0]])),
        Err(PatchError::Misaligned)
    );
    assert_eq!(
        Patch::parse(&patch(Rect::new(0, 0, 4, 1), &[&[0]])),
        Err(PatchError::Misaligned)
    );
    assert_eq!(
        Patch::parse(&patch(Rect::new(0, 0, 0, 1), &[])),
        Err(PatchError::OutOfBounds)
    );
    assert_eq!(
        Patch::parse(&patch(Rect::new(0, 300, 8, 1), &[&[0]])),
        Err(PatchError::OutOfBounds)
    );
    assert_eq!(
        Patch::parse(&patch(Rect::new(400, 0, 8, 1), &[&[0]])),
        Err(PatchError::OutOfBounds)
    );
}

/// Black box in the right half of row `i` (as a patch would draw "on").
fn on_marker(title: bool, i: usize) -> Rect {
    let r = item_rect(title, i);
    Rect::new(320, r.y + 8, 64, 24)
}

#[test]
fn patch_replaces_base_and_strip_so_moves_do_not_restore_stale_pixels() {
    let mut b = bundle(true, 3, 1);
    let mut s = Screen::new(&mut b).unwrap();
    let mut f = MonoFrame::new();
    s.compose(&mut f);
    // OK on row 1: the server answers with a patch of the whole row, one plane.
    let row = item_rect(true, 0);
    let (mut want_base, _) = layout(true, 3);
    fill(&mut want_base, on_marker(true, 0), BinaryColor::On);
    let body = patch(row, &[&cut(&want_base, row, false)]);
    let dirty = s.apply_patch(&Patch::parse(&body).unwrap(), &mut f);
    assert_eq!(dirty, row);
    // One plane: the row shows the patch pixels as they are, even though it is selected.
    assert_eq!(cut(&f, row, false), cut(&want_base, row, false));
    assert_eq!(s.base(), &want_base.as_bytes()[..]);
    assert_eq!(s.strip(1).unwrap(), &cut(&want_base, row, false)[..]);
    // Moving away shows the patched base, not the old row.
    s.move_cursor(Move::Down, &mut f, Merge::Never);
    assert_eq!(cut(&f, row, false), cut(&want_base, row, false));
}

#[test]
fn two_plane_patch_keeps_the_highlight() {
    let mut b = bundle(true, 3, 1);
    let mut s = Screen::new(&mut b).unwrap();
    let mut f = MonoFrame::new();
    s.compose(&mut f);
    // A patch covering the marker area of rows 1 and 2 (crosses the row boundary) with
    // an unselected and a selected plane.
    let area = Rect::new(320, 48, 64, 64);
    let (mut base, _) = layout(true, 3);
    fill(&mut base, on_marker(true, 0), BinaryColor::On);
    fill(&mut base, on_marker(true, 1), BinaryColor::On);
    let plain = cut(&base, area, false);
    let inverted = cut(&base, area, true);
    s.apply_patch(
        &Patch::parse(&patch(area, &[&plain, &inverted])).unwrap(),
        &mut f,
    );
    // Rows 1 and 2 strips now hold the inverted marker: every selection matches
    // "base with the new marker, selected row inverted".
    let want = |sel: usize| {
        let (mut w, rows) = layout(true, 3);
        fill(&mut w, on_marker(true, 0), BinaryColor::On);
        fill(&mut w, on_marker(true, 1), BinaryColor::On);
        let r = rows[sel].0;
        for y in r.y as usize..r.bottom() as usize {
            for b in &mut w.as_bytes_mut()[y * 50..y * 50 + 50] {
                *b = !*b;
            }
        }
        w
    };
    assert_eq!(f.as_bytes(), want(1).as_bytes());
    for sel in [2, 3, 1, 2] {
        s.select(sel, &mut f, Merge::Never);
        assert_eq!(f.as_bytes(), want(sel).as_bytes(), "row {sel}");
    }
}

/// A panel stand-in: what the driver would leave on the glass.
struct Panel {
    glass: MonoFrame,
    shadow: MonoFrame,
    fulls: u32,
    partials: u32,
}

impl Panel {
    fn show(
        &mut self,
        policy: &mut RefreshPolicy,
        update: Update,
        frame: &MonoFrame,
        dirty: Dirty,
    ) {
        match policy.plan(update) {
            RefreshKind::Full => {
                self.glass.copy_from(frame);
                self.shadow.copy_from(frame);
                self.fulls += 1;
                policy.full_done(true);
            }
            RefreshKind::Partial => {
                for r in dirty.rects() {
                    let w = r.aligned();
                    self.glass.copy_rect(frame, w);
                    self.shadow.copy_rect(frame, w);
                    self.partials += 1;
                    policy.partial_done(true);
                }
            }
        }
    }
}

#[test]
fn simulated_session() {
    let mut policy = RefreshPolicy::default();
    let mut panel = Panel {
        glass: MonoFrame::new(),
        shadow: MonoFrame::new(),
        fulls: 0,
        partials: 0,
    };
    let mut b = bundle(true, 5, 1);
    let mut s = Screen::new(&mut b).unwrap();
    let mut frame = MonoFrame::new();
    s.compose(&mut frame);
    panel.show(
        &mut policy,
        Update::NewScreen,
        &frame,
        Dirty::One(Rect::full()),
    );
    assert_eq!(panel.fulls, 1);
    // Moves of two rows each: 4 moves (8 partials), a full, 4 more, a full, one more.
    for _ in 0..11 {
        let d = s.move_cursor(Move::Down, &mut frame, Merge::Never);
        panel.show(&mut policy, Update::Change, &frame, d);
        assert_eq!(panel.glass.as_bytes(), frame.as_bytes());
        assert_eq!(panel.shadow.as_bytes(), frame.as_bytes());
    }
    assert_eq!((panel.fulls, panel.partials), (3, 18));
    assert!(policy.full_before_sleep());
    panel.show(&mut policy, Update::NewScreen, &frame, Dirty::None);
    assert!(!policy.full_before_sleep());
    assert_eq!(
        panel.glass.as_bytes(),
        expected(true, 5, s.selected()).as_bytes()
    );
}
