//! An embassy-net driver wrapper that counts frames, for diagnostics on the panel.
//!
//! Over WSL USB/IP the serial log is rarely available, so "did DHCP send anything, did
//! anything come back" has to be answerable from numbers the device can draw.

use core::sync::atomic::{AtomicU32, Ordering};
use core::task::Context;

use embassy_net_driver::{Capabilities, Driver, HardwareAddress, LinkState, RxToken, TxToken};

/// Frames received from and sent to the radio since boot.
pub static RX_FRAMES: AtomicU32 = AtomicU32::new(0);
pub static TX_FRAMES: AtomicU32 = AtomicU32::new(0);

pub fn rx_frames() -> u32 {
    RX_FRAMES.load(Ordering::Relaxed)
}

pub fn tx_frames() -> u32 {
    TX_FRAMES.load(Ordering::Relaxed)
}

pub struct Counting<D>(pub D);

pub struct CountRx<T>(T);
pub struct CountTx<T>(T);

impl<T: RxToken> RxToken for CountRx<T> {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, f: F) -> R {
        RX_FRAMES.fetch_add(1, Ordering::Relaxed);
        self.0.consume(f)
    }
}

impl<T: TxToken> TxToken for CountTx<T> {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        TX_FRAMES.fetch_add(1, Ordering::Relaxed);
        self.0.consume(len, f)
    }
}

impl<D: Driver> Driver for Counting<D> {
    type RxToken<'a>
        = CountRx<D::RxToken<'a>>
    where
        Self: 'a;
    type TxToken<'a>
        = CountTx<D::TxToken<'a>>
    where
        Self: 'a;

    fn receive(&mut self, cx: &mut Context<'_>) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        self.0
            .receive(cx)
            .map(|(rx, tx)| (CountRx(rx), CountTx(tx)))
    }

    fn transmit(&mut self, cx: &mut Context<'_>) -> Option<Self::TxToken<'_>> {
        self.0.transmit(cx).map(CountTx)
    }

    fn link_state(&mut self, cx: &mut Context<'_>) -> LinkState {
        self.0.link_state(cx)
    }

    fn capabilities(&self) -> Capabilities {
        self.0.capabilities()
    }

    fn hardware_address(&self) -> HardwareAddress {
        self.0.hardware_address()
    }
}
