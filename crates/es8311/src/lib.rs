//! Driver for the Everest Semiconductor **ES8311** mono audio codec (one ADC, one DAC),
//! as fitted to the ZECTRIX NOTE4 / NOTE4C main board.
//!
//! Scope: the codec as an I2S **slave** with an external MCLK, standard (Philips) I2S,
//! 16-bit words, an analog microphone on MIC1P/N and the DAC driving a speaker amplifier.
//! The audio data itself goes over I2S, which this crate does not touch; it only writes
//! control registers over I2C (blocking, `embedded-hal` 1.0).
//!
//! The register sequences are ported from Espressif `esp_codec_dev` (`device/es8311`,
//! Apache-2.0), the codec driver that the official Zectrix board demo (MIT) uses. The
//! default [`Config`] is that demo's setting: 16 kHz, MCLK = 256 × fs, 30 dB mic gain,
//! volume 80 %. See `THIRD_PARTY_NOTICES.md` and `docs/AUDIO.md`.
//!
//! ```ignore
//! let mut codec = Es8311::new(i2c, es8311::ADDRESS);
//! codec.init(&Config::default())?;   // MCLK must already be running
//! codec.set_volume(60)?;
//! // ... stream I2S ...
//! codec.power_down()?;
//! ```

#![no_std]

pub mod coeff;
pub mod reg;

use embedded_hal::i2c::I2c;

/// 7-bit I2C address with the CE pin low (as on the NOTE4 board).
pub const ADDRESS: u8 = 0x18;

/// Expected contents of the chip ID registers `0xFD`, `0xFE`.
pub const CHIP_ID: [u8; 2] = [0x83, 0x11];

/// Digital mic gain (ADC scale, register `0x16`), in 6 dB steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum MicGain {
    Db0 = 0,
    Db6 = 1,
    Db12 = 2,
    Db18 = 3,
    Db24 = 4,
    Db30 = 5,
    Db36 = 6,
    Db42 = 7,
}

impl MicGain {
    /// The step at or below `db`, as `esp_codec_dev` rounds it (e.g. 29 dB → 24 dB).
    pub const fn from_db(db: u8) -> Self {
        match db {
            0..6 => Self::Db0,
            6..12 => Self::Db6,
            12..18 => Self::Db12,
            18..24 => Self::Db18,
            24..30 => Self::Db24,
            30..36 => Self::Db30,
            36..42 => Self::Db36,
            _ => Self::Db42,
        }
    }

    /// Gain in dB.
    pub const fn db(self) -> u8 {
        self as u8 * 6
    }
}

/// Codec setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// Sample rate in Hz. ADC and DAC share it (one LRCK).
    pub sample_rate: u32,
    /// MCLK as a multiple of the sample rate. The ESP32-S3 I2S peripheral generates
    /// 256 × fs for 16-bit data.
    pub mclk_multiple: u32,
    /// Digital mic gain. The analog PGA is fixed at the reference setting (register
    /// `0x14` = `0x1A`: MIC1P/N, 30 dB).
    pub mic_gain: MicGain,
    /// ADC volume register (`0x17`), 0.5 dB steps, `0xBF` = 0 dB.
    pub adc_volume: u8,
    /// Speaker volume in percent, see [`volume_to_register`].
    pub volume: u8,
    /// Route the DAC output into the right ADC slot (register `0x44` = `0x58`), so a
    /// stereo read carries mic (left) + playback reference (right) for echo cancelling.
    /// The reference driver does this by default; `false` leaves the right slot empty.
    pub dac_reference: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            sample_rate: 16_000,
            mclk_multiple: 256,
            mic_gain: MicGain::Db30,
            adc_volume: 0xBF,
            volume: 80,
            dac_reference: true,
        }
    }
}

impl Config {
    /// MCLK in Hz.
    pub const fn mclk_hz(&self) -> u32 {
        self.sample_rate * self.mclk_multiple
    }
}

/// Driver errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error<E> {
    /// The I2C transfer failed (no ACK usually means the audio rail is off).
    I2c(E),
    /// The chip ID registers did not read `0x83 0x11`.
    WrongChip([u8; 2]),
    /// No clock table entry for this MCLK and sample rate.
    UnsupportedRate { mclk: u32, rate: u32 },
}

impl<E> From<E> for Error<E> {
    fn from(e: E) -> Self {
        Self::I2c(e)
    }
}

/// DAC volume register value for a volume in percent, using the reference volume curve:
/// 0 % = off (`0x00`), 1–100 % = −49.5 dB … 0 dB on a linear dB scale, plus the 3.6 dB the
/// reference adds back for a 3.3 V codec driving a 5 V amplifier.
/// `esp_codec_dev` computes `(int)((−50 + 0.5·v + 3.61 + 95.5) · 2)`, which is `98 + v`.
pub const fn volume_to_register(percent: u8) -> u8 {
    if percent == 0 {
        0
    } else if percent >= 100 {
        198
    } else {
        98 + percent
    }
}

/// ES8311 driver.
pub struct Es8311<I2C> {
    i2c: I2C,
    address: u8,
}

impl<I2C: I2c> Es8311<I2C> {
    pub fn new(i2c: I2C, address: u8) -> Self {
        Self { i2c, address }
    }

    /// Give the bus back.
    pub fn release(self) -> I2C {
        self.i2c
    }

    pub fn write_reg(&mut self, reg: u8, value: u8) -> Result<(), Error<I2C::Error>> {
        self.i2c.write(self.address, &[reg, value])?;
        Ok(())
    }

    pub fn read_reg(&mut self, reg: u8) -> Result<u8, Error<I2C::Error>> {
        let mut buf = [0u8];
        self.i2c.write_read(self.address, &[reg], &mut buf)?;
        Ok(buf[0])
    }

    fn modify_reg(&mut self, reg: u8, f: impl FnOnce(u8) -> u8) -> Result<(), Error<I2C::Error>> {
        let v = self.read_reg(reg)?;
        self.write_reg(reg, f(v))
    }

    /// Chip ID registers `0xFD`, `0xFE` (`[0x83, 0x11]` on an ES8311).
    pub fn chip_id(&mut self) -> Result<[u8; 2], Error<I2C::Error>> {
        Ok([self.read_reg(reg::CHIP_ID1)?, self.read_reg(reg::CHIP_ID2)?])
    }

    /// Full bring-up: check the chip ID, reset to known values, set the I2S format and
    /// clocks, power up ADC and DAC, then set mic gain and volume and unmute.
    ///
    /// MCLK should already be running. The speaker amplifier is the caller's business:
    /// switch it on after this returns.
    pub fn init(&mut self, config: &Config) -> Result<(), Error<I2C::Error>> {
        let coeff =
            coeff::lookup(config.mclk_hz(), config.sample_rate).ok_or(Error::UnsupportedRate {
                mclk: config.mclk_hz(),
                rate: config.sample_rate,
            })?;
        let id = self.chip_id()?;
        if id != CHIP_ID {
            return Err(Error::WrongChip(id));
        }
        self.open(config.dac_reference)?;
        self.set_format_16bit_i2s()?;
        self.set_clocks(coeff)?;
        self.start(config.adc_volume)?;
        self.set_mic_gain(config.mic_gain)?;
        self.set_volume(config.volume)?;
        self.set_mute(false)
    }

    /// `es8311_open`: known register state, slave mode, MCLK from the MCLK pin.
    fn open(&mut self, dac_reference: bool) -> Result<(), Error<I2C::Error>> {
        if self.read_reg(reg::SYSTEM_0D)? != 0xFA {
            self.write_reg(reg::SYSTEM_0D, 0xFA)?;
        }
        // I2C noise immunity. Written twice: the first write after power-up sometimes
        // fails on this chip (reference driver comment).
        let _ = self.write_reg(reg::GPIO_44, 0x08);
        self.write_reg(reg::GPIO_44, 0x08)?;
        for (r, v) in [
            (reg::CLK_MANAGER_01, 0x30),
            (reg::CLK_MANAGER_02, 0x00),
            (reg::CLK_MANAGER_03, 0x10),
            (reg::ADC_16, 0x24),
            (reg::CLK_MANAGER_04, 0x10),
            (reg::CLK_MANAGER_05, 0x00),
            (reg::SYSTEM_0B, 0x00),
            (reg::SYSTEM_0C, 0x00),
            (reg::SYSTEM_10, 0x1F),
            (reg::SYSTEM_11, 0x7F),
            // CSM on, slave mode (bit 6 clear).
            (reg::RESET, 0x80),
            // MCLK from the MCLK pin (bit 7 clear), not inverted (bit 6 clear), clocks on.
            (reg::CLK_MANAGER_01, 0x3F),
        ] {
            self.write_reg(r, v)?;
        }
        // BCLK not inverted.
        self.modify_reg(reg::CLK_MANAGER_06, |v| v & !0x20)?;
        self.write_reg(reg::SYSTEM_13, 0x10)?;
        self.write_reg(reg::ADC_1B, 0x0A)?;
        self.write_reg(reg::ADC_1C, 0x6A)?;
        self.write_reg(reg::GPIO_44, if dac_reference { 0x58 } else { 0x08 })
    }

    /// `es8311_set_bits_per_sample(16)` + `es8311_config_fmt(I2S_NORMAL)`.
    fn set_format_16bit_i2s(&mut self) -> Result<(), Error<I2C::Error>> {
        // Word length bits 4:2 = 0b011 (16 bit); format bits 1:0 = 0b00 (I2S).
        let f = |v: u8| (v & !0x1F) | 0x0C;
        self.modify_reg(reg::SDPIN, f)?;
        self.modify_reg(reg::SDPOUT, f)
    }

    /// `es8311_config_sample`. Registers 02–05 hold the values `open` wrote, so they are
    /// computed rather than read back.
    fn set_clocks(&mut self, c: &coeff::Coeff) -> Result<(), Error<I2C::Error>> {
        let multi = match c.pre_multi {
            1 => 0,
            2 => 1,
            4 => 2,
            _ => 3,
        };
        self.write_reg(reg::CLK_MANAGER_02, ((c.pre_div - 1) << 5) | (multi << 3))?;
        self.write_reg(
            reg::CLK_MANAGER_05,
            ((c.adc_div - 1) << 4) | (c.dac_div - 1),
        )?;
        self.write_reg(reg::CLK_MANAGER_03, (c.fs_mode << 6) | c.adc_osr)?;
        self.write_reg(reg::CLK_MANAGER_04, c.dac_osr)?;
        self.modify_reg(reg::CLK_MANAGER_07, |v| (v & 0xC0) | c.lrck_h)?;
        self.write_reg(reg::CLK_MANAGER_08, c.lrck_l)?;
        let bclk = if c.bclk_div < 19 {
            c.bclk_div - 1
        } else {
            c.bclk_div
        };
        self.modify_reg(reg::CLK_MANAGER_06, |v| (v & 0xE0) | bclk)
    }

    /// `es8311_start` for ADC + DAC with an analog mic.
    fn start(&mut self, adc_volume: u8) -> Result<(), Error<I2C::Error>> {
        self.write_reg(reg::RESET, 0x80)?;
        self.write_reg(reg::CLK_MANAGER_01, 0x3F)?;
        // Unmute both serial ports (bit 6).
        self.modify_reg(reg::SDPIN, |v| v & !0x40)?;
        self.modify_reg(reg::SDPOUT, |v| v & !0x40)?;
        for (r, v) in [
            (reg::ADC_VOLUME, adc_volume),
            // Power up PGA and ADC modulator.
            (reg::SYSTEM_0E, 0x02),
            // Power up the DAC.
            (reg::SYSTEM_12, 0x00),
            // Analog mic on MIC1P/N (bit 6 clear = not PDM), PGA 30 dB.
            (reg::SYSTEM_14, 0x1A),
            // Analog power up.
            (reg::SYSTEM_0D, 0x01),
            (reg::ADC_15, 0x40),
            (reg::DAC_37, 0x08),
            (reg::GP_45, 0x00),
        ] {
            self.write_reg(r, v)?;
        }
        Ok(())
    }

    /// Digital mic gain (`0x16`).
    pub fn set_mic_gain(&mut self, gain: MicGain) -> Result<(), Error<I2C::Error>> {
        self.write_reg(reg::ADC_16, gain as u8)
    }

    /// Speaker volume in percent (see [`volume_to_register`]).
    pub fn set_volume(&mut self, percent: u8) -> Result<(), Error<I2C::Error>> {
        self.set_dac_volume_raw(volume_to_register(percent))
    }

    /// Raw DAC volume register (`0x32`): 0.5 dB steps, `0xBF` = 0 dB.
    pub fn set_dac_volume_raw(&mut self, value: u8) -> Result<(), Error<I2C::Error>> {
        self.write_reg(reg::DAC_VOLUME, value)
    }

    /// DAC soft mute (`0x31` bits 6:5).
    pub fn set_mute(&mut self, mute: bool) -> Result<(), Error<I2C::Error>> {
        self.modify_reg(reg::DAC_MUTE, |v| {
            let v = v & 0x9F;
            if mute { v | 0x60 } else { v }
        })
    }

    /// Mute, then `es8311_suspend`: everything powered down, clocks off. Switch the
    /// speaker amplifier off before this. [`Self::init`] brings the codec back.
    pub fn power_down(&mut self) -> Result<(), Error<I2C::Error>> {
        self.set_mute(true)?;
        for (r, v) in [
            (reg::DAC_VOLUME, 0x00),
            (reg::ADC_VOLUME, 0x00),
            (reg::SYSTEM_0E, 0xFF),
            (reg::SYSTEM_12, 0x02),
            (reg::SYSTEM_14, 0x00),
            (reg::SYSTEM_0D, 0xFA),
            (reg::ADC_15, 0x00),
            (reg::CLK_MANAGER_02, 0x10),
            (reg::RESET, 0x00),
            (reg::RESET, 0x1F),
            (reg::CLK_MANAGER_01, 0x30),
            (reg::CLK_MANAGER_01, 0x00),
            (reg::GP_45, 0x00),
            (reg::SYSTEM_0D, 0xFC),
            (reg::CLK_MANAGER_02, 0x00),
        ] {
            self.write_reg(r, v)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_curve_matches_reference() {
        // Zectrix demo default (80 %) gives 0xB2 in esp_codec_dev.
        assert_eq!(volume_to_register(80), 0xB2);
        assert_eq!(volume_to_register(0), 0);
        assert_eq!(volume_to_register(1), 99);
        assert_eq!(volume_to_register(100), 198);
        assert_eq!(volume_to_register(255), 198);
    }

    #[test]
    fn mic_gain_rounds_down() {
        assert_eq!(MicGain::from_db(30), MicGain::Db30);
        assert_eq!(MicGain::from_db(29), MicGain::Db24);
        assert_eq!(MicGain::from_db(5), MicGain::Db0);
        assert_eq!(MicGain::from_db(200), MicGain::Db42);
        assert_eq!(MicGain::Db36.db(), 36);
    }

    #[test]
    fn clock_table_has_256fs_rows() {
        for rate in [8_000, 16_000, 24_000, 32_000, 48_000] {
            assert!(coeff::lookup(rate * 256, rate).is_some(), "{rate}");
        }
        let c = coeff::lookup(4_096_000, 16_000).unwrap();
        assert_eq!((c.pre_div, c.pre_multi, c.bclk_div), (1, 1, 4));
        assert_eq!((c.adc_osr, c.dac_osr), (0x10, 0x20));
        assert!(coeff::lookup(4_000_000, 16_000).is_none());
    }
}
