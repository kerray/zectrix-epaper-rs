//! ES8311 register addresses (ES8311 datasheet; names follow Espressif `esp_codec_dev`
//! `es8311_reg.h`).

/// Reset, CSM power-on, master/slave mode.
pub const RESET: u8 = 0x00;
/// Clock manager: MCLK source, MCLK invert, clock enables.
pub const CLK_MANAGER_01: u8 = 0x01;
/// Clock manager: pre-divider and pre-multiplier.
pub const CLK_MANAGER_02: u8 = 0x02;
/// Clock manager: ADC fs mode and oversampling rate.
pub const CLK_MANAGER_03: u8 = 0x03;
/// Clock manager: DAC oversampling rate.
pub const CLK_MANAGER_04: u8 = 0x04;
/// Clock manager: ADC and DAC clock dividers.
pub const CLK_MANAGER_05: u8 = 0x05;
/// Clock manager: BCLK invert and divider.
pub const CLK_MANAGER_06: u8 = 0x06;
/// Clock manager: tri-state, LRCK divider high bits.
pub const CLK_MANAGER_07: u8 = 0x07;
/// Clock manager: LRCK divider low bits.
pub const CLK_MANAGER_08: u8 = 0x08;
/// Serial data port in (to the DAC): format, word length, mute.
pub const SDPIN: u8 = 0x09;
/// Serial data port out (from the ADC): format, word length, mute.
pub const SDPOUT: u8 = 0x0A;
pub const SYSTEM_0B: u8 = 0x0B;
pub const SYSTEM_0C: u8 = 0x0C;
/// Analog power up / down.
pub const SYSTEM_0D: u8 = 0x0D;
/// PGA and ADC modulator power.
pub const SYSTEM_0E: u8 = 0x0E;
pub const SYSTEM_10: u8 = 0x10;
pub const SYSTEM_11: u8 = 0x11;
/// DAC power.
pub const SYSTEM_12: u8 = 0x12;
/// Output (HP drive) setup.
pub const SYSTEM_13: u8 = 0x13;
/// Mic input select (analog MIC1P/N or PDM), analog PGA gain.
pub const SYSTEM_14: u8 = 0x14;
/// ADC ramp rate, DMIC sense.
pub const ADC_15: u8 = 0x15;
/// ADC digital gain scale ("mic gain", 0–42 dB in 6 dB steps).
pub const ADC_16: u8 = 0x16;
/// ADC volume (0xBF = 0 dB).
pub const ADC_VOLUME: u8 = 0x17;
/// ALC / auto-mute, ADC high-pass stage 1.
pub const ADC_1B: u8 = 0x1B;
/// ADC equaliser bypass, high-pass stage 2.
pub const ADC_1C: u8 = 0x1C;
/// DAC soft mute.
pub const DAC_MUTE: u8 = 0x31;
/// DAC volume (0x00 = -95.5 dB, 0xBF = 0 dB, 0xFF = +32 dB, 0.5 dB steps).
pub const DAC_VOLUME: u8 = 0x32;
/// DAC ramp rate.
pub const DAC_37: u8 = 0x37;
/// GPIO / internal DAC-to-ADC reference routing, I2C noise filter.
pub const GPIO_44: u8 = 0x44;
/// General purpose control.
pub const GP_45: u8 = 0x45;
/// Chip ID 1 (reads 0x83).
pub const CHIP_ID1: u8 = 0xFD;
/// Chip ID 2 (reads 0x11).
pub const CHIP_ID2: u8 = 0xFE;
/// Chip version.
pub const CHIP_VERSION: u8 = 0xFF;
