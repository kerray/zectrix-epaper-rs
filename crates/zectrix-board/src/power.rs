//! esp-hal glue: power latch, panel rail, battery ADC and deep sleep.
//!
//! The NOTE4 / NOTE4C stays powered on battery only while GPIO17 is high. Engage the
//! latch first thing in `main` and keep it held, including through deep sleep. On USB
//! power the board stays on whatever the latch does.

use esp_hal::{
    analog::adc::{Adc, AdcCalCurve, AdcConfig, AdcPin, Attenuation},
    gpio::{Event, Input, Level, Output, OutputConfig, WakeupConfig},
    peripherals::{ADC1, GPIO4, GPIO6, GPIO17, LPWR},
    rtc_cntl::sleep::{LowPower, RtcSleepConfig},
    time::{Duration, Instant},
};

use crate::battery;

/// The GPIO17 power latch.
pub struct PowerLatch<'d> {
    pin: Output<'d>,
}

impl<'d> PowerLatch<'d> {
    /// Drive the latch high and hold the pad, so the level also survives deep sleep.
    pub fn engage(pin: GPIO17<'d>) -> Self {
        let mut pin = Output::new(pin, Level::High, OutputConfig::default());
        pin.set_pad_hold(true);
        Self { pin }
    }

    /// Release the latch. On battery this cuts power to the whole board as soon as the
    /// power key is released. On USB it does nothing visible, so callers should fall back
    /// to [`deep_sleep`] afterwards.
    pub fn release(mut self) {
        self.pin.set_pad_hold(false);
        self.pin.set_low();
    }
}

/// Hold an output pad at its current level through deep sleep, e.g. the panel rail
/// (GPIO6) low, so the panel draws nothing.
pub fn hold_through_sleep(pin: &mut Output<'_>) {
    pin.set_pad_hold(true);
}

/// Put the chip into deep sleep. It wakes (with a reset) when `wake_button` reads low,
/// and after `timeout` if one is given. The power latch must already be held.
///
/// `wake_button` must be a low-power (RTC) pad with a pull-up, such as OK/BOOT (GPIO0).
pub fn deep_sleep(lpwr: LPWR<'_>, wake_button: &mut Input<'_>, timeout: Option<Duration>) -> ! {
    let mut low_power = LowPower::new(lpwr);
    // GPIO0 always has a low-power path on the ESP32-S3, so this cannot fail there.
    let _ = wake_button.apply_wakeup_config(&WakeupConfig::default().with_low_power_path(true));
    wake_button.listen(Event::LowLevel);
    if let Some(timeout) = timeout {
        low_power.set_wakeup_deadline(Instant::now() + timeout);
    }
    low_power.sleep_deep(RtcSleepConfig::deep())
}

/// Last resort after a panic, for `esp-backtrace`'s `custom-halt` hook. Without it the
/// panic handler spins forever with the latch held and drains the battery.
///
/// Switches the panel rail off and releases the power latch, which turns the device off
/// on battery. On USB the board stays powered, so the chip deep-sleeps and restarts
/// after `retry_after`.
pub fn halt_after_panic(retry_after: Duration) -> ! {
    // SAFETY: the program is finished; nothing else will touch these peripherals again.
    let (rail, latch, lpwr) = unsafe { (GPIO6::steal(), GPIO17::steal(), LPWR::steal()) };
    // `Output::new` releases a pad hold and then drives the new level.
    let mut rail = Output::new(rail, Level::Low, OutputConfig::default());
    rail.set_pad_hold(true);
    let _latch = Output::new(latch, Level::Low, OutputConfig::default());
    let mut low_power = LowPower::new(lpwr);
    low_power.set_wakeup_deadline(Instant::now() + retry_after);
    low_power.sleep_deep(RtcSleepConfig::deep())
}

/// Battery voltage on GPIO4 (ADC1 channel 3) through the 2:1 divider.
pub struct Battery<'d> {
    adc: Adc<'d, ADC1<'d>, esp_hal::Blocking>,
    pin: AdcPin<GPIO4<'d>, ADC1<'d>, AdcCalCurve<ADC1<'d>>>,
}

impl<'d> Battery<'d> {
    pub fn new(adc1: ADC1<'d>, pin: GPIO4<'d>) -> Self {
        let mut config = AdcConfig::new();
        let pin = config.enable_pin_with_cal(pin, Attenuation::_11dB);
        Self {
            adc: Adc::new(adc1, config),
            pin,
        }
    }

    /// Battery millivolts, averaged over 10 calibrated samples, or `None` if the value
    /// is outside the plausible window (e.g. no battery).
    pub fn read_mv(&mut self) -> Option<u32> {
        let mut sum = 0u32;
        for _ in 0..10 {
            sum += u32::from(self.adc.read_blocking(&mut self.pin));
        }
        battery::validate(battery::battery_mv(sum / 10))
    }
}
