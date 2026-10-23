//! Host tests: record what the driver puts on the wire and compare it with the
//! sequences of the reference C drivers (see docs/vendor/RESEARCH_NOTES.md §3).

use std::{cell::RefCell, convert::Infallible, rc::Rc};

use embedded_graphics_core::{pixelcolor::BinaryColor, prelude::*};
use embedded_hal::{delay::DelayNs, digital, spi};
use ssd2683::{Error, MonoFrame, Panel, QuadColor, QuadFrame, Ssd2683, Timing};

#[derive(Debug, Clone, PartialEq)]
enum Ev {
    Pin(&'static str, bool),
    Byte(u8),
}

type Log = Rc<RefCell<Vec<Ev>>>;

struct Spi(Log);
impl spi::ErrorType for Spi {
    type Error = Infallible;
}
impl spi::SpiBus for Spi {
    fn read(&mut self, _: &mut [u8]) -> Result<(), Infallible> {
        unimplemented!()
    }
    fn write(&mut self, w: &[u8]) -> Result<(), Infallible> {
        self.0.borrow_mut().extend(w.iter().map(|&b| Ev::Byte(b)));
        Ok(())
    }
    fn transfer(&mut self, _: &mut [u8], _: &[u8]) -> Result<(), Infallible> {
        unimplemented!()
    }
    fn transfer_in_place(&mut self, _: &mut [u8]) -> Result<(), Infallible> {
        unimplemented!()
    }
    fn flush(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
}

struct Out(&'static str, Log);
impl digital::ErrorType for Out {
    type Error = Infallible;
}
impl digital::OutputPin for Out {
    fn set_low(&mut self) -> Result<(), Infallible> {
        self.1.borrow_mut().push(Ev::Pin(self.0, false));
        Ok(())
    }
    fn set_high(&mut self) -> Result<(), Infallible> {
        self.1.borrow_mut().push(Ev::Pin(self.0, true));
        Ok(())
    }
}

/// BUSY that alternates low (busy) / high (idle) on every read, which satisfies both
/// "wait while busy" and "wait until the refresh starts".
struct Busy {
    reads: u32,
    stuck_low: bool,
}
impl digital::ErrorType for Busy {
    type Error = Infallible;
}
impl digital::InputPin for Busy {
    fn is_high(&mut self) -> Result<bool, Infallible> {
        self.reads += 1;
        Ok(!self.stuck_low && self.reads.is_multiple_of(2))
    }
    fn is_low(&mut self) -> Result<bool, Infallible> {
        self.is_high().map(|h| !h)
    }
}

struct NoDelay;
impl DelayNs for NoDelay {
    fn delay_ns(&mut self, _: u32) {}
}

type Drv = Ssd2683<Spi, Out, Out, Out, Busy, Out, NoDelay>;

fn driver(panel: Panel) -> (Drv, Log) {
    let log: Log = Rc::default();
    let d = Ssd2683::new(
        Spi(log.clone()),
        Out("cs", log.clone()),
        Out("dc", log.clone()),
        Out("rst", log.clone()),
        Busy {
            reads: 0,
            stuck_low: false,
        },
        Out("pwr", log.clone()),
        NoDelay,
        panel,
    );
    (d, log)
}

/// Decode the log into (command, data) pairs using the DC line.
fn transactions(log: &Log) -> Vec<(u8, Vec<u8>)> {
    let mut dc = false;
    let mut out: Vec<(u8, Vec<u8>)> = Vec::new();
    for ev in log.borrow().iter() {
        match ev {
            Ev::Pin("dc", level) => dc = *level,
            Ev::Byte(b) if !dc => out.push((*b, Vec::new())),
            Ev::Byte(b) => out.last_mut().expect("data before command").1.push(*b),
            _ => {}
        }
    }
    out
}

fn summary(t: &[(u8, Vec<u8>)]) -> Vec<(u8, Vec<u8>)> {
    // Replace long pixel payloads by their length so the sequence stays readable.
    t.iter()
        .map(|(c, d)| {
            if d.len() > 16 {
                (*c, vec![0xDD])
            } else {
                (*c, d.clone())
            }
        })
        .collect()
}

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
