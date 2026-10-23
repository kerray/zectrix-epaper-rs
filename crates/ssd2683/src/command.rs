//! SSD2683 command bytes, named after datasheet rev 0.20 where documented.
//! `OTP_SELECT` (0xE9) and `TEMP_ACTIVATE` (0xA5 sent as a command) are not in the
//! public datasheet; they come from the vendor's sample code.

/// PSR – panel setting.
pub const PANEL_SETTING: u8 = 0x00;
/// POF – internal power off (data 0x00).
pub const POWER_OFF: u8 = 0x02;
/// PON – internal power on.
pub const POWER_ON: u8 = 0x04;
/// DSLP – deep sleep (data [`DEEP_SLEEP_CHECK`]).
pub const DEEP_SLEEP: u8 = 0x07;
/// Check code that must follow [`DEEP_SLEEP`].
pub const DEEP_SLEEP_CHECK: u8 = 0xA5;
/// DTM – start pixel data transmission.
pub const DATA_START: u8 = 0x10;
/// DRF – display refresh (data 0x00).
pub const DISPLAY_REFRESH: u8 = 0x12;
/// TSC – read the internal temperature sensor (3-wire read).
pub const TEMP_READ: u8 = 0x40;
/// CDI – VCOM and data interval (used by partial refresh, 0x77).
pub const VCOM_DATA_INTERVAL: u8 = 0x50;
/// PTLW – partial window.
pub const PARTIAL_WINDOW: u8 = 0x83;
/// CCSET – cascade setting (0x02 = use temperature from TSSET).
pub const CASCADE_SETTING: u8 = 0xE0;
/// TSSET – forced temperature code.
pub const TEMP_SET: u8 = 0xE6;
/// Activates the forced temperature / waveform (undocumented, vendor sample).
pub const TEMP_ACTIVATE: u8 = 0xA5;
/// Select the OTP waveform bank (undocumented, vendor sample; data 0x01).
pub const OTP_SELECT: u8 = 0xE9;
