//! Colour type for the four-pigment NOTE4C panel.

use embedded_graphics_core::pixelcolor::{BinaryColor, PixelColor, raw::RawU2};

/// One of the four pigments of the NOTE4C panel. The discriminant is the 2-bit code
/// the SSD2683 expects in RAM.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum QuadColor {
    Black = 0b00,
    White = 0b01,
    Yellow = 0b10,
    Red = 0b11,
}

impl QuadColor {
    /// The 2-bit RAM code.
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// Decode a 2-bit RAM code (upper bits are ignored).
    pub const fn from_code(code: u8) -> Self {
        match code & 0b11 {
            0b00 => Self::Black,
            0b01 => Self::White,
            0b10 => Self::Yellow,
            _ => Self::Red,
        }
    }

    /// A byte with four pixels of this colour.
    pub const fn solid_byte(self) -> u8 {
        let c = self.code();
        (c << 6) | (c << 4) | (c << 2) | c
    }
}

impl PixelColor for QuadColor {
    type Raw = RawU2;
}

impl From<RawU2> for QuadColor {
    fn from(raw: RawU2) -> Self {
        use embedded_graphics_core::pixelcolor::raw::RawData;
        Self::from_code(raw.into_inner())
    }
}

/// `BinaryColor::On` is ink (black), `Off` is paper (white), matching the usual
/// embedded-graphics convention for e-paper.
impl From<BinaryColor> for QuadColor {
    fn from(c: BinaryColor) -> Self {
        match c {
            BinaryColor::On => Self::Black,
            BinaryColor::Off => Self::White,
        }
    }
}
