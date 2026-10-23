//! Host tests for the NOTE4 mono partial refresh, checked against the official Zectrix
//! driver (`zectrix_epd_refresh_partial_1bpp`, docs/vendor/RESEARCH_NOTES.md §3.2).

mod common;

use common::*;
use embedded_graphics_core::pixelcolor::BinaryColor::{Off as W, On as B};
use ssd2683::{Error, MonoFrame, Panel, Rect, Timing};

type Outcome = Result<Rect, Error<core::convert::Infallible>>;

/// Power on, clear the log, run one partial refresh; return the result and the
/// (command, data) pairs of the partial refresh alone.
fn partial(shadow: &mut MonoFrame, new: &MonoFrame, rect: Rect) -> (Outcome, Vec<(u8, Vec<u8>)>) {
    let (mut d, log) = driver(Panel::Mono);
    d.power_on().unwrap();
    log.borrow_mut().clear();
    let r = d.show_mono_partial(shadow, new, rect);
    (r, transactions(&log))
}

/// The `83` window bytes for an inclusive window.
fn window(x0: u16, x1: u16, y0: u16, y1: u16) -> Vec<u8> {
    let mut v = Vec::new();
    for n in [x0, x1, y0, y1] {
        v.push((n >> 8) as u8 & 0x03);
        v.push(n as u8);
    }
    v.push(0x01);
    v
}

fn window_of(t: &[(u8, Vec<u8>)]) -> Vec<u8> {
    t.iter().find(|(c, _)| *c == 0x83).expect("no 83").1.clone()
}

fn data_of(t: &[(u8, Vec<u8>)]) -> Vec<u8> {
    t.iter().find(|(c, _)| *c == 0x10).expect("no 10").1.clone()
}

#[test]
fn small_window_matches_official_sequence() {
    let mut shadow = MonoFrame::new();
    let mut new = MonoFrame::new();
    new.set(8, 10, B);
    let (r, t) = partial(&mut shadow, &new, Rect::new(8, 10, 16, 2));
    assert_eq!(r, Ok(Rect::new(8, 10, 16, 2)));
    assert_eq!(
        t,
        vec![
            // reset + OTP before every refresh
            (0x00, vec![0x2F, 0x0E]),
            (0xE9, vec![0x01]),
            (0x50, vec![0x77]),
            (0xE0, vec![0x00]),
            (0xA5, vec![]),
            (0x83, window(8, 23, 10, 11)),
            // 2 rows × 16 px × 2 bit = 2 × 4 bytes; (8,10) W→B = 10, the rest W→W = 11
            (0x10, vec![0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]),
            (0x04, vec![]),
            (0x12, vec![0x00]),
            (0x02, vec![0x00]),
        ]
    );
    assert_eq!(shadow.get(8, 10), Some(B));
}

#[test]
fn transition_codes_for_all_four_pairs() {
    // old B B W W, new B W B W → 00 01 10 11
    let mut shadow = MonoFrame::new();
    shadow.set(0, 0, B);
    shadow.set(1, 0, B);
    let mut new = MonoFrame::new();
    new.set(0, 0, B);
    new.set(2, 0, B);
    let (r, t) = partial(&mut shadow, &new, Rect::new(0, 0, 4, 1));
    assert_eq!(r, Ok(Rect::new(0, 0, 8, 1)));
    assert_eq!(data_of(&t), vec![0b00_01_10_11, 0xFF]);
    assert_eq!(
        [0, 1, 2, 3].map(|x| shadow.get(x, 0).unwrap()),
        [B, W, B, W]
    );
}

#[test]
fn transition_codes_in_low_nibble_and_second_byte() {
    // Same four pairs at x = 12..16, which land in the low nibble of a 1 bpp byte.
    let mut shadow = MonoFrame::new();
    let mut new = MonoFrame::new();
    shadow.set(12, 3, B);
    shadow.set(13, 3, B);
    new.set(12, 3, B);
    new.set(14, 3, B);
    let (_, t) = partial(&mut shadow, &new, Rect::new(8, 3, 8, 1));
    assert_eq!(data_of(&t), vec![0xFF, 0b00_01_10_11]);
}

#[test]
fn unaligned_start_and_end_are_widened_to_bytes() {
    let mut shadow = MonoFrame::new();
    let new = MonoFrame::new();
    let (r, t) = partial(&mut shadow, &new, Rect::new(5, 7, 10, 3));
    assert_eq!(r, Ok(Rect::new(0, 7, 16, 3)));
    assert_eq!(window_of(&t), window(0, 15, 7, 9));
    assert_eq!(data_of(&t).len(), 3 * 16 / 4);
}

#[test]
fn window_at_right_edge_ends_at_399() {
    for rect in [Rect::new(395, 0, 5, 1), Rect::new(390, 0, 10, 1)] {
        let mut shadow = MonoFrame::new();
        let (r, t) = partial(&mut shadow, &MonoFrame::new(), rect);
        assert_eq!(r, Ok(Rect::new(rect.x & !7, 0, 400 - (rect.x & !7), 1)));
        assert_eq!(window_of(&t), window(rect.x & !7, 399, 0, 0));
        // 399 = 0x18F: the high bits go in the first byte
        assert_eq!(&window_of(&t)[2..4], &[0x01, 0x8F]);
    }
}

#[test]
fn full_screen_partial() {
    let mut shadow = MonoFrame::new();
    let mut new = MonoFrame::new();
    new.set(399, 299, B);
    let (r, t) = partial(&mut shadow, &new, Rect::full());
    assert_eq!(r, Ok(Rect::full()));
    assert_eq!(
        window_of(&t),
        vec![0, 0, 0x01, 0x8F, 0, 0, 0x01, 0x2B, 0x01]
    );
    let data = data_of(&t);
    assert_eq!(data.len(), 30_000);
    assert!(data[..29_999].iter().all(|&b| b == 0xFF));
    // last pixel W→B
    assert_eq!(data[29_999], 0b11_11_11_10);
    assert_eq!(shadow.get(399, 299), Some(B));
}

#[test]
fn extra_pixels_of_the_widened_window_come_from_new() {
    let mut shadow = MonoFrame::new();
    let mut new = MonoFrame::new();
    new.set(0, 0, B); // outside the requested rect, inside the aligned window
    new.set(100, 0, B); // outside the window
    let (r, t) = partial(&mut shadow, &new, Rect::new(4, 0, 2, 1));
    assert_eq!(r, Ok(Rect::new(0, 0, 8, 1)));
    assert_eq!(data_of(&t), vec![0b10_11_11_11, 0xFF]);
    assert_eq!(shadow.get(0, 0), Some(B));
    assert_eq!(shadow.get(100, 0), Some(W), "outside the window: unchanged");
}

#[test]
fn refused_on_four_colour_panel() {
    let (mut d, log) = driver(Panel::Quad);
    d.power_on().unwrap();
    log.borrow_mut().clear();
    let mut shadow = MonoFrame::new();
    assert_eq!(
        d.show_mono_partial(&mut shadow, &MonoFrame::new(), Rect::new(0, 0, 8, 8)),
        Err(Error::WrongPanel)
    );
    assert!(log.borrow().is_empty(), "nothing may reach the NOTE4C");
}

#[test]
fn invalid_rects_are_rejected_before_anything_is_sent() {
    for rect in [
        Rect::new(0, 0, 0, 10),
        Rect::new(0, 0, 10, 0),
        Rect::new(392, 0, 16, 1),
        Rect::new(0, 290, 8, 11),
        Rect::new(400, 0, 1, 1),
        Rect::new(0, 300, 1, 1),
        Rect::new(u16::MAX, 0, 8, 1),
    ] {
        let (mut d, log) = driver(Panel::Mono);
        d.power_on().unwrap();
        log.borrow_mut().clear();
        let mut shadow = MonoFrame::new();
        assert_eq!(
            d.show_mono_partial(&mut shadow, &MonoFrame::new(), rect),
            Err(Error::InvalidRect),
            "{rect:?}"
        );
        assert!(log.borrow().is_empty());
    }
}

#[test]
fn refused_while_unpowered() {
    let (mut d, _) = driver(Panel::Mono);
    let mut shadow = MonoFrame::new();
    assert_eq!(
        d.show_mono_partial(&mut shadow, &MonoFrame::new(), Rect::full()),
        Err(Error::NotPowered)
    );
}

#[test]
fn shadow_is_kept_when_the_refresh_times_out() {
    let (d, _) = driver_with_busy(
        Panel::Mono,
        Busy {
            reads: 0,
            stuck_low: false,
            // power_on and the start of the partial refresh pass, then BUSY sticks
            stuck_after: Some(8),
        },
    );
    let mut d = d.with_timing(Timing {
        busy_timeout_ms: 50,
        poll_ms: 10,
        refresh_start_ms: 10,
    });
    d.power_on().unwrap();
    let mut shadow = MonoFrame::new();
    let mut new = MonoFrame::new();
    new.set(0, 0, B);
    let r = d.show_mono_partial(&mut shadow, &new, Rect::new(0, 0, 8, 1));
    assert!(matches!(r, Err(Error::BusyTimeout(_))), "{r:?}");
    assert_eq!(shadow.get(0, 0), Some(W));
}

#[test]
fn full_refresh_after_partial_restores_the_otp_setup() {
    let (mut d, log) = driver(Panel::Mono);
    d.power_on().unwrap();
    let mut shadow = MonoFrame::new();
    let new = MonoFrame::new();
    d.show_mono_partial(&mut shadow, &new, Rect::new(0, 0, 8, 8))
        .unwrap();
    log.borrow_mut().clear();
    d.show_mono(&new, None).unwrap();
    let t = summary(&transactions(&log));
    assert_eq!(
        &t[..5],
        &[
            (0x00, vec![0x2F, 0x0E]),
            (0xE9, vec![0x01]),
            (0xE0, vec![0x02]),
            (0xE6, vec![241]),
            (0xA5, vec![]),
        ]
    );
}

#[test]
fn pixel_data_stays_in_one_chip_select_window() {
    let (mut d, log) = driver(Panel::Mono);
    d.power_on().unwrap();
    log.borrow_mut().clear();
    let mut shadow = MonoFrame::new();
    d.show_mono_partial(&mut shadow, &MonoFrame::new(), Rect::new(0, 0, 400, 40))
        .unwrap();
    // Find the 0x10 command byte, then count CS edges until the next command.
    let log = log.borrow();
    let start = log.iter().position(|e| *e == Ev::Byte(0x10)).unwrap();
    let mut cs_lows = 0;
    let mut bytes = 0;
    let mut dc = false;
    for e in &log[start + 1..] {
        match e {
            Ev::Pin("cs", false) if dc => cs_lows += 1,
            Ev::Pin("dc", l) => dc = *l,
            Ev::Byte(_) if dc => bytes += 1,
            Ev::Byte(_) => break,
            _ => {}
        }
    }
    assert_eq!(cs_lows, 1);
    assert_eq!(bytes, 40 * 100);
}

// --- Rect and MonoFrame helpers ---

#[test]
fn rect_alignment() {
    assert_eq!(Rect::new(5, 1, 10, 2).aligned(), Rect::new(0, 1, 16, 2));
    assert_eq!(Rect::new(8, 0, 8, 1).aligned(), Rect::new(8, 0, 8, 1));
    assert_eq!(Rect::new(9, 0, 1, 1).aligned(), Rect::new(8, 0, 8, 1));
    assert_eq!(Rect::new(399, 0, 1, 1).aligned(), Rect::new(392, 0, 8, 1));
    assert_eq!(Rect::full().aligned(), Rect::full());
}

#[test]
fn rect_validity() {
    assert!(Rect::full().is_valid());
    assert!(Rect::new(399, 299, 1, 1).is_valid());
    assert!(!Rect::new(399, 299, 2, 1).is_valid());
    assert!(!Rect::new(0, 0, 0, 1).is_valid());
}

#[test]
fn rect_union() {
    let a = Rect::new(0, 0, 400, 40);
    let b = Rect::new(0, 200, 400, 40);
    assert_eq!(a.union(&b), Rect::new(0, 0, 400, 240));
    assert_eq!(
        Rect::new(10, 10, 5, 5).union(&Rect::new(3, 12, 2, 10)),
        Rect::new(3, 10, 12, 12)
    );
    assert_eq!(Rect::new(0, 0, 0, 0).union(&b), b);
    assert_eq!(b.union(&Rect::new(5, 5, 0, 3)), b);
}

#[test]
fn dirty_rect_of_equal_frames_is_none() {
    assert_eq!(MonoFrame::new().dirty_rect(&MonoFrame::new()), None);
}

#[test]
fn dirty_rect_is_pixel_exact() {
    let a = MonoFrame::new();
    let mut b = MonoFrame::new();
    b.set(13, 7, B);
    assert_eq!(a.dirty_rect(&b), Some(Rect::new(13, 7, 1, 1)));
    b.set(250, 120, B);
    assert_eq!(a.dirty_rect(&b), Some(Rect::new(13, 7, 238, 114)));
    b.set(399, 299, B);
    b.set(0, 0, B);
    assert_eq!(a.dirty_rect(&b), Some(Rect::full()));
}

#[test]
fn dirty_rect_spans_bytes_in_one_row() {
    let mut a = MonoFrame::new();
    let b = MonoFrame::new();
    a.set(6, 4, B);
    a.set(17, 4, B);
    assert_eq!(a.dirty_rect(&b), Some(Rect::new(6, 4, 12, 1)));
}

#[test]
fn copy_rect_unaligned_and_aligned() {
    let mut src = MonoFrame::new();
    src.fill(B);
    let mut dst = MonoFrame::new();
    dst.copy_rect(&src, Rect::new(3, 2, 2, 1));
    assert_eq!(dst.get(2, 2), Some(W));
    assert_eq!(dst.get(3, 2), Some(B));
    assert_eq!(dst.get(4, 2), Some(B));
    assert_eq!(dst.get(5, 2), Some(W));
    dst.copy_rect(&src, Rect::new(392, 299, 8, 1));
    assert_eq!(dst.get(391, 299), Some(W));
    assert_eq!(dst.get(392, 299), Some(B));
    assert_eq!(dst.get(399, 299), Some(B));
    assert_eq!(dst.get(399, 298), Some(W));
    dst.copy_from(&src);
    assert_eq!(dst.dirty_rect(&src), None);
}
