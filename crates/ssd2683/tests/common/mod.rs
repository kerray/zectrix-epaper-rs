//! Mock SPI, pins and delay that record what the driver puts on the wire.

#![allow(dead_code)]

use std::{cell::RefCell, convert::Infallible, rc::Rc};

use embedded_hal::{delay::DelayNs, digital, spi};
use ssd2683::{Panel, Ssd2683};

#[derive(Debug, Clone, PartialEq)]
pub enum Ev {
    Pin(&'static str, bool),
    Byte(u8),
}

pub type Log = Rc<RefCell<Vec<Ev>>>;

pub struct Spi(pub Log);
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

pub struct Out(pub &'static str, pub Log);
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
pub struct Busy {
    pub reads: u32,
    pub stuck_low: bool,
    /// Become stuck low (busy forever) after this many reads.
    pub stuck_after: Option<u32>,
}
impl digital::ErrorType for Busy {
    type Error = Infallible;
}
impl digital::InputPin for Busy {
    fn is_high(&mut self) -> Result<bool, Infallible> {
        self.reads += 1;
        let stuck = self.stuck_low || self.stuck_after.is_some_and(|n| self.reads > n);
        Ok(!stuck && self.reads.is_multiple_of(2))
    }
    fn is_low(&mut self) -> Result<bool, Infallible> {
        self.is_high().map(|h| !h)
    }
}

pub struct NoDelay;
impl DelayNs for NoDelay {
    fn delay_ns(&mut self, _: u32) {}
}

pub type Drv = Ssd2683<Spi, Out, Out, Out, Busy, Out, NoDelay>;

pub fn driver(panel: Panel) -> (Drv, Log) {
    driver_with_busy(
        panel,
        Busy {
            reads: 0,
            stuck_low: false,
            stuck_after: None,
        },
    )
}

pub fn driver_with_busy(panel: Panel, busy: Busy) -> (Drv, Log) {
    let log: Log = Rc::default();
    let d = Ssd2683::new(
        Spi(log.clone()),
        Out("cs", log.clone()),
        Out("dc", log.clone()),
        Out("rst", log.clone()),
        busy,
        Out("pwr", log.clone()),
        NoDelay,
        panel,
    );
    (d, log)
}

/// Decode the log into (command, data) pairs using the DC line.
pub fn transactions(log: &Log) -> Vec<(u8, Vec<u8>)> {
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

pub fn summary(t: &[(u8, Vec<u8>)]) -> Vec<(u8, Vec<u8>)> {
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
