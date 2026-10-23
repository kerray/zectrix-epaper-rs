//! GPIO numbers of the NOTE4 / NOTE4C V1.0 board. Verified against the V1.0 schematic,
//! both wiki development guides and the reference firmwares (docs/vendor/RESEARCH_NOTES.md §2).
//! "Active low" means the signal reads 0 when asserted.

// E-paper, SPI3, CS driven as a plain GPIO.
pub const EPD_PWR_EN: u8 = 6; // active high, switches 3V3_EPD
pub const EPD_BUSY: u8 = 8; // low = busy
pub const EPD_RST: u8 = 9;
pub const EPD_DC: u8 = 10; // 0 = command, 1 = data
pub const EPD_CS: u8 = 11;
pub const EPD_SCK: u8 = 12;
pub const EPD_MOSI: u8 = 13; // bidirectional for the 3-wire temperature read

// Power
/// Must be driven high as early as possible after boot and held through deep sleep,
/// otherwise the battery rail drops as soon as the power key is released.
pub const PWR_LATCH: u8 = 17;
pub const AUDIO_PWR_EN: u8 = 42; // also powers the I2C pull-ups
pub const SPEAKER_AMP_EN: u8 = 46; // strapping pin, keep low

// Buttons, all active low
pub const BTN_OK: u8 = 0; // "BOOT", strapping pin
pub const BTN_DOWN: u8 = 18; // shared with the power key, noisier level
pub const BTN_UP: u8 = 39;

// Status
pub const LED: u8 = 3; // green, active low
pub const CHARGE_FULL: u8 = 1; // active high
pub const CHARGING: u8 = 2; // active low
pub const BATTERY_ADC: u8 = 4; // ADC1 channel 3, 2:1 divider

// I2C (port 0): ES8311 0x18, PCF8563 RTC 0x51, NFC 0x55
pub const I2C_SDA: u8 = 47;
pub const I2C_SCL: u8 = 48;
pub const RTC_INT: u8 = 5; // PCF8563 INT, open drain, active low
pub const NFC_FD: u8 = 7; // field detect, active low
pub const NFC_PWR: u8 = 21;

// I2S to the ES8311 codec
pub const I2S_MCLK: u8 = 14;
pub const I2S_BCLK: u8 = 15;
pub const I2S_DIN: u8 = 16; // codec → ESP
pub const I2S_WS: u8 = 38;
pub const I2S_DOUT: u8 = 45; // ESP → codec

// Expansion header J3: 3V3, GND, TX 43, RX 44, GPIO40, GPIO41
pub const EXP_GPIO_A: u8 = 40;
pub const EXP_GPIO_B: u8 = 41;

pub const I2C_ADDR_CODEC: u8 = 0x18;
pub const I2C_ADDR_RTC: u8 = 0x51;
pub const I2C_ADDR_NFC: u8 = 0x55;
