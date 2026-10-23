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

use crate::{HEIGHT, NATIVE_FRAME_BYTES, NATIVE_STRIDE, QuadColor, Rect, WIDTH};

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

    /// Copy all pixels from `src`.
    pub fn copy_from(&mut self, src: &MonoFrame) {
        self.buf.copy_from_slice(&src.buf);
    }

    /// Copy the pixels inside `rect` from `src`. The parts of `rect` outside the panel
    /// are ignored.
    pub fn copy_rect(&mut self, src: &MonoFrame, rect: Rect) {
        let x1 = rect.right().min(WIDTH);
        let y1 = rect.bottom().min(HEIGHT);
        if rect.x.is_multiple_of(8) && x1.is_multiple_of(8) {
            // Whole bytes, e.g. a refreshed window.
            let bytes = rect.x as usize / 8..x1 as usize / 8;
            for y in rect.y as usize..y1 as usize {
                let row = y * MONO_STRIDE;
                self.buf[row + bytes.start..row + bytes.end]
                    .copy_from_slice(&src.buf[row + bytes.start..row + bytes.end]);
            }
            return;
        }
        for y in rect.y as u32..y1 {
            for x in rect.x as u32..x1 {
                if let Some(c) = src.get(x, y) {
                    self.set(x, y, c);
                }
            }
        }
    }

    /// The smallest rectangle that holds every pixel that differs between `self` and
    /// `other`, or `None` if the frames are equal. Pass it to
    /// [`Ssd2683::show_mono_partial`](crate::Ssd2683::show_mono_partial), which widens
    /// it to whole bytes.
    pub fn dirty_rect(&self, other: &MonoFrame) -> Option<Rect> {
        let mut out: Option<Rect> = None;
        for y in 0..HEIGHT as usize {
            let a = &self.buf[y * MONO_STRIDE..(y + 1) * MONO_STRIDE];
            let b = &other.buf[y * MONO_STRIDE..(y + 1) * MONO_STRIDE];
            let Some(first) = (0..MONO_STRIDE).find(|&i| a[i] != b[i]) else {
                continue;
            };
            let last = (first..MONO_STRIDE).rev().find(|&i| a[i] != b[i])?;
            let x0 = first * 8 + (a[first] ^ b[first]).leading_zeros() as usize;
            let x1 = last * 8 + 8 - (a[last] ^ b[last]).trailing_zeros() as usize;
            let row = Rect::new(x0 as u16, y as u16, (x1 - x0) as u16, 1);
            out = Some(out.map_or(row, |r| r.union(&row)));
        }
        out
    }

    /// Transition codes `(old << 1) | new` for row `y`, 1 bpp bytes `bytes` (8 px each),
    /// two output bytes per input byte. `self` is the old image.
    pub(crate) fn transition_row(
        &self,
        new: &MonoFrame,
        y: usize,
        bytes: core::ops::Range<usize>,
        out: &mut [u8],
    ) {
        let base = y * MONO_STRIDE;
        for (k, i) in bytes.enumerate() {
            let (o, n) = (self.buf[base + i], new.buf[base + i]);
            out[k * 2] = (expand_nibble(o >> 4) << 1) | expand_nibble(n >> 4);
            out[k * 2 + 1] = (expand_nibble(o & 0x0F) << 1) | expand_nibble(n & 0x0F);
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
