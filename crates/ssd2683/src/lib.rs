//! Driver for the Solomon Systech **SSD2683** e-paper controller as fitted to the
//! ZECTRIX NOTE4 (400×300 black/white) and NOTE4C (400×300 black/white/yellow/red).
//!
//! The controller RAM is always 2 bits per pixel, 4 pixels per byte, MSB = leftmost
//! pixel, 100 bytes per row, 30 000 bytes per frame. The two panels differ in what the
//! 2-bit codes mean and in the refresh sequence:
//!
//! * **NOTE4C** (four colour): `00` black, `01` white, `10` yellow, `11` red. The OTP
//!   waveform is selected with `E9 01` and nothing else is configured. Full refresh only.
//! * **NOTE4** (mono): `00` black, `01` white. The panel setting register is written
//!   (`00 2F 0E`), and before every full refresh the OTP waveform is picked for the panel
//!   temperature (`E0 02`, `E6 <code>`, `A5`).
//!
//! Sequences are ported from MIT-licensed C drivers, see `THIRD_PARTY_NOTICES.md`:
//! Zectrix Lab `zectrix-note4-epd-demo` (mono) and LazyYoun `youn-ink-fourcolor-firmware`
//! as adapted in `fiedoruk/emini-home` (four colour).
//!
//! The crate is `no_std`, has no allocator requirement and is tested on the host.

#![no_std]

pub mod color;
pub mod command;
mod driver;
pub mod framebuffer;

pub use color::QuadColor;
pub use driver::{Error, Panel, Ssd2683, Stage, Timing};
pub use framebuffer::{MonoFrame, QuadFrame};

/// Panel width in pixels (both devices).
pub const WIDTH: u32 = 400;
/// Panel height in pixels (both devices).
pub const HEIGHT: u32 = 300;
/// Bytes per row in controller RAM (2 bpp).
pub const NATIVE_STRIDE: usize = WIDTH as usize / 4;
/// Bytes of a full frame in controller RAM (2 bpp).
pub const NATIVE_FRAME_BYTES: usize = NATIVE_STRIDE * HEIGHT as usize;
