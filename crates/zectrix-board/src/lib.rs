//! Board support for the ZECTRIX NOTE4 and NOTE4C Devkits.
//!
//! Both devices use the same V1.0 main board (ESP32-S3 N16R8) and the same pin map;
//! only the e-paper panel on the 24-pin FPC differs. See `docs/HARDWARE.md`.
//!
//! Pure logic (pin numbers, battery maths) always builds and is tested on the host.
//! The `hal` feature adds the esp-hal glue.

#![no_std]

pub mod battery;
pub mod pins;

#[cfg(feature = "hal")]
pub mod power;
