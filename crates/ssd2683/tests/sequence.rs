//! Host tests: record what the driver puts on the wire and compare it with the
//! sequences of the reference C drivers (see docs/vendor/RESEARCH_NOTES.md §3).

mod common;

use std::rc::Rc;

use common::*;
use embedded_graphics_core::{pixelcolor::BinaryColor, prelude::*};
use ssd2683::{Error, MonoFrame, Panel, QuadColor, QuadFrame, Ssd2683, Timing};

#[test]
fn quad_full_refresh_matches_reference_sequence() {
    let (mut d, log) = driver(Panel::Quad);
    let mut frame = QuadFrame::new();
    frame.set(0, 0, QuadColor::Red);
    frame.set(1, 0, QuadColor::Yellow);
    frame.set(2, 0, QuadColor::Black);
    frame.set(399, 299, QuadColor::Red);

    d.power_on().unwrap();
    d.show_quad(&frame).unwrap();
    d.power_off().unwrap();

    assert_eq!(
        log.borrow().iter().find(|e| matches!(e, Ev::Pin("pwr", _))),
        Some(&Ev::Pin("pwr", true))
    );
    assert_eq!(log.borrow().last(), Some(&Ev::Pin("pwr", false)));

    let t = transactions(&log);
    assert_eq!(
        summary(&t),
        vec![
            (0xE9, vec![0x01]),
            (0x10, vec![0xDD]),
            (0x04, vec![]),
            (0x12, vec![0x00]),
            (0x02, vec![0x00]),
            (0x07, vec![0xA5]),
        ]
    );
    let pixels = &t[1].1;
    assert_eq!(pixels.len(), 30_000);
    // red, yellow, black, white
    assert_eq!(pixels[0], 0b11_10_00_01);
    assert_eq!(pixels[1], 0x55);
    assert_eq!(pixels[29_999], 0b01_01_01_11);
}

#[test]
fn mono_full_refresh_matches_official_driver() {
    let (mut d, log) = driver(Panel::Mono);
    let mut frame = MonoFrame::new();
    frame.set(0, 0, BinaryColor::On);
    frame.set(3, 0, BinaryColor::On);

    d.power_on().unwrap();
    d.show_mono(&frame, Some(18)).unwrap();

    let t = transactions(&log);
    assert_eq!(
        summary(&t),
        vec![
            // power_on
            (0x00, vec![0x2F, 0x0E]),
            (0xE9, vec![0x01]),
            // reset + OTP again before each refresh
            (0x00, vec![0x2F, 0x0E]),
            (0xE9, vec![0x01]),
            (0xE0, vec![0x02]),
            (0xE6, vec![238]),
            (0xA5, vec![]),
            (0x10, vec![0xDD]),
            (0x04, vec![]),
            (0x12, vec![0x00]),
            (0x02, vec![0x00]),
        ]
    );
    let pixels = &t[7].1;
    assert_eq!(pixels.len(), 30_000);
    // black, white, white, black → 00 01 01 00
    assert_eq!(pixels[0], 0b00_01_01_00);
    assert_eq!(pixels[1], 0x55);
}

#[test]
fn mono_default_temperature_is_25c() {
    let (mut d, log) = driver(Panel::Mono);
    d.power_on().unwrap();
    d.show_mono(&MonoFrame::new(), None).unwrap();
    assert!(transactions(&log).contains(&(0xE6, vec![241])));
}

#[test]
fn wrong_frame_type_is_rejected() {
    let (mut d, _) = driver(Panel::Mono);
    d.power_on().unwrap();
    assert_eq!(d.show_quad(&QuadFrame::new()), Err(Error::WrongPanel));
}

#[test]
fn refuses_to_send_while_unpowered() {
    let (mut d, _) = driver(Panel::Quad);
    assert_eq!(d.show_quad(&QuadFrame::new()), Err(Error::NotPowered));
}

#[test]
fn busy_timeout_is_reported() {
    let log: Log = Rc::default();
    let mut d = Ssd2683::new(
        Spi(log.clone()),
        Out("cs", log.clone()),
        Out("dc", log.clone()),
        Out("rst", log.clone()),
        Busy {
            reads: 0,
            stuck_low: true,
            stuck_after: None,
        },
        Out("pwr", log.clone()),
        NoDelay,
        Panel::Quad,
    )
    .with_timing(Timing {
        busy_timeout_ms: 100,
        poll_ms: 10,
        refresh_start_ms: 10,
    });
    assert_eq!(d.power_on(), Err(Error::BusyTimeout(ssd2683::Stage::Reset)));
}

#[test]
fn draw_target_roundtrip() {
    use embedded_graphics_core::primitives::Rectangle;
    let mut q = QuadFrame::new();
    q.fill_solid(
        &Rectangle::new(Point::new(10, 10), Size::new(5, 5)),
        QuadColor::Yellow,
    )
    .unwrap();
    assert_eq!(q.get(12, 12), Some(QuadColor::Yellow));
    assert_eq!(q.get(9, 12), Some(QuadColor::White));
    assert_eq!(q.get(400, 0), None);

    let mut m = MonoFrame::new();
    Pixel(Point::new(-1, 5), BinaryColor::On)
        .draw(&mut m)
        .unwrap();
    Pixel(Point::new(7, 5), BinaryColor::On)
        .draw(&mut m)
        .unwrap();
    assert_eq!(m.get(7, 5), Some(BinaryColor::On));
    assert_eq!(m.get(8, 5), Some(BinaryColor::Off));
}
