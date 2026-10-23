//! In-memory frames that implement `embedded_graphics_core::draw_target::DrawTarget`.
//!
//! [`QuadFrame`] is already in the controller's native 2 bpp layout and is sent as is.
//! [`MonoFrame`] is 1 bpp (half the RAM) and is expanded to 2 bpp row by row while it
//! is sent.

use core::convert::Infallible;

use embedded_graphics_core::{
    Pixel,
    draw_target::DrawTarget,
    geometry::{OriginDimensions, Size},
    pixelcolor::BinaryColor,
};

use crate::{HEIGHT, NATIVE_FRAME_BYTES, NATIVE_STRIDE, QuadColor, WIDTH};

/// Bytes per row of a 1 bpp frame.
pub const MONO_STRIDE: usize = WIDTH as usize / 8;
/// Bytes of a 1 bpp frame.
pub const MONO_FRAME_BYTES: usize = MONO_STRIDE * HEIGHT as usize;

/// 1 bpp frame for the NOTE4 mono panel. Bit set = white (paper), clear = black (ink),
/// MSB = leftmost pixel. `BinaryColor::On` draws ink.
pub struct MonoFrame {
    buf: [u8; MONO_FRAME_BYTES],
}

impl MonoFrame {
    /// A white frame.
    pub const fn new() -> Self {
        Self {
            buf: [0xFF; MONO_FRAME_BYTES],
        }
    }

    pub fn as_bytes(&self) -> &[u8; MONO_FRAME_BYTES] {
        &self.buf
    }

    pub fn as_bytes_mut(&mut self) -> &mut [u8; MONO_FRAME_BYTES] {
        &mut self.buf
    }

    /// Fill the whole frame with one colour.
    pub fn fill(&mut self, color: BinaryColor) {
        self.buf.fill(match color {
            BinaryColor::On => 0x00,
            BinaryColor::Off => 0xFF,
        });
    }

    /// Pixel at (x, y); `None` outside the panel.
    pub fn get(&self, x: u32, y: u32) -> Option<BinaryColor> {
        if x >= WIDTH || y >= HEIGHT {
            return None;
        }
        let byte = self.buf[y as usize * MONO_STRIDE + x as usize / 8];
        let white = (byte >> (7 - (x % 8))) & 1 == 1;
        Some(if white {
            BinaryColor::Off
        } else {
            BinaryColor::On
        })
    }

    /// Set pixel at (x, y); ignored outside the panel.
    pub fn set(&mut self, x: u32, y: u32, color: BinaryColor) {
        if x >= WIDTH || y >= HEIGHT {
            return;
        }
        let byte = &mut self.buf[y as usize * MONO_STRIDE + x as usize / 8];
        let mask = 1u8 << (7 - (x % 8));
        match color {
            BinaryColor::Off => *byte |= mask,
            BinaryColor::On => *byte &= !mask,
        }
    }

    /// Row `y` of the frame, expanded to the controller's 2 bpp codes.
    pub fn native_row(&self, y: usize, out: &mut [u8; NATIVE_STRIDE]) {
        let row = &self.buf[y * MONO_STRIDE..(y + 1) * MONO_STRIDE];
        for (i, &b) in row.iter().enumerate() {
            out[i * 2] = expand_nibble(b >> 4);
            out[i * 2 + 1] = expand_nibble(b & 0x0F);
        }
    }
}

impl Default for MonoFrame {
    fn default() -> Self {
        Self::new()
    }
}

/// Map 4 one-bit pixels to 4 two-bit codes: bit `b` becomes `0b0b`
/// (0 = black `00`, 1 = white `01`).
const fn expand_nibble(n: u8) -> u8 {
    ((n & 0b1000) << 3) | ((n & 0b0100) << 2) | ((n & 0b0010) << 1) | (n & 0b0001)
}

impl OriginDimensions for MonoFrame {
    fn size(&self) -> Size {
        Size::new(WIDTH, HEIGHT)
    }
}

impl DrawTarget for MonoFrame {
    type Color = BinaryColor;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(p, c) in pixels {
            if p.x >= 0 && p.y >= 0 {
                self.set(p.x as u32, p.y as u32, c);
            }
        }
        Ok(())
    }

    fn clear(&mut self, color: Self::Color) -> Result<(), Self::Error> {
        self.fill(color);
        Ok(())
    }
}

/// 2 bpp frame for the NOTE4C four-colour panel, in the controller's native layout.
pub struct QuadFrame {
    buf: [u8; NATIVE_FRAME_BYTES],
}

impl QuadFrame {
    /// A white frame.
    pub const fn new() -> Self {
        Self {
            buf: [QuadColor::White.solid_byte(); NATIVE_FRAME_BYTES],
        }
    }

    pub fn as_bytes(&self) -> &[u8; NATIVE_FRAME_BYTES] {
        &self.buf
    }

    pub fn as_bytes_mut(&mut self) -> &mut [u8; NATIVE_FRAME_BYTES] {
        &mut self.buf
    }

    pub fn fill(&mut self, color: QuadColor) {
        self.buf.fill(color.solid_byte());
    }

    pub fn get(&self, x: u32, y: u32) -> Option<QuadColor> {
        if x >= WIDTH || y >= HEIGHT {
            return None;
        }
        let byte = self.buf[y as usize * NATIVE_STRIDE + x as usize / 4];
        Some(QuadColor::from_code(byte >> (6 - 2 * (x % 4))))
    }

    pub fn set(&mut self, x: u32, y: u32, color: QuadColor) {
        if x >= WIDTH || y >= HEIGHT {
            return;
        }
        let byte = &mut self.buf[y as usize * NATIVE_STRIDE + x as usize / 4];
        let shift = 6 - 2 * (x % 4);
        *byte = (*byte & !(0b11 << shift)) | (color.code() << shift);
    }
}

impl Default for QuadFrame {
    fn default() -> Self {
        Self::new()
    }
}

impl OriginDimensions for QuadFrame {
    fn size(&self) -> Size {
        Size::new(WIDTH, HEIGHT)
    }
}

impl DrawTarget for QuadFrame {
    type Color = QuadColor;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(p, c) in pixels {
            if p.x >= 0 && p.y >= 0 {
                self.set(p.x as u32, p.y as u32, c);
            }
        }
        Ok(())
    }

    fn clear(&mut self, color: Self::Color) -> Result<(), Self::Error> {
        self.fill(color);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_nibble_maps_bits_to_codes() {
        assert_eq!(expand_nibble(0b0000), 0x00);
        assert_eq!(expand_nibble(0b1111), 0x55);
        assert_eq!(expand_nibble(0b1000), 0b0100_0000);
        assert_eq!(expand_nibble(0b0001), 0b0000_0001);
        assert_eq!(expand_nibble(0b1010), 0b0100_0100);
    }
}
