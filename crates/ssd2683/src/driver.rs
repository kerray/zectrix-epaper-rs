use embedded_hal::{
    delay::DelayNs,
    digital::{InputPin, OutputPin},
    spi::SpiBus,
};

use crate::{HEIGHT, MonoFrame, NATIVE_STRIDE, QuadFrame, command as cmd};

/// Which panel is attached. The pin map is the same on both devices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    /// NOTE4: 400×300 black/white.
    Mono,
    /// NOTE4C: 400×300 black/white/yellow/red.
    Quad,
}

/// Where a BUSY wait timed out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Reset,
    OtpSelect,
    TempActivate,
    DataStart,
    PowerOn,
    Refresh,
    PowerOff,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error<SpiE> {
    Spi(SpiE),
    /// A GPIO operation failed (all ESP32 pins are infallible; kept for mocks).
    Pin,
    /// BUSY stayed low longer than [`Timing::busy_timeout_ms`].
    BusyTimeout(Stage),
    /// After the refresh command BUSY never went low, so the panel did not start
    /// refreshing (four-colour path only).
    RefreshNotStarted,
    /// A frame for the other panel type was passed.
    WrongPanel,
    /// A frame was sent while the panel rail was off; call [`Ssd2683::power_on`].
    NotPowered,
}

/// Wait parameters. The defaults follow emini-home: 120 s per BUSY stage (a four-colour
/// full refresh takes 10–25 s) and 10 ms polling.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    /// How long BUSY may stay low in one stage (power on, refresh, power off).
    pub busy_timeout_ms: u32,
    /// Poll interval while BUSY is low. 0 is treated as 1.
    pub poll_ms: u32,
    /// How long BUSY may take to assert after the refresh command (four-colour only).
    pub refresh_start_ms: u32,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            busy_timeout_ms: 120_000,
            poll_ms: 10,
            refresh_start_ms: 1_000,
        }
    }
}

/// OTP temperature code for the mono panel, from the official Zectrix driver.
/// `celsius` is the value read from the controller's sensor (0x40).
pub const fn mono_temperature_code(celsius: u8) -> u8 {
    match celsius {
        0..=5 => 232,
        6..=10 => 235,
        11..=20 => 238,
        21..=30 => 241,
        _ => 244,
    }
}

/// SSD2683 on a write-only SPI bus with software chip select.
///
/// * `pwr` switches the panel's 3V3 rail (GPIO6 on NOTE4/NOTE4C, active high).
/// * `busy` is **low while the controller is busy** (GPIO8).
/// * CS is driven here rather than by the SPI peripheral so that one command and its
///   data, or a whole frame, stays inside one CS-low window, as the C drivers do.
pub struct Ssd2683<SPI, CS, DC, RST, BUSY, PWR, D> {
    spi: SPI,
    cs: CS,
    dc: DC,
    rst: RST,
    busy: BUSY,
    pwr: PWR,
    delay: D,
    panel: Panel,
    timing: Timing,
    powered: bool,
}

impl<SPI, CS, DC, RST, BUSY, PWR, D> Ssd2683<SPI, CS, DC, RST, BUSY, PWR, D>
where
    SPI: SpiBus,
    CS: OutputPin,
    DC: OutputPin,
    RST: OutputPin,
    BUSY: InputPin,
    PWR: OutputPin,
    D: DelayNs,
{
    /// Wrap the bus and pins. Nothing is sent and the panel rail stays as it is until
    /// [`power_on`](Self::power_on). `panel` must match the attached panel: the driver
    /// refuses frames of the other type, and the mono and four-colour sequences must not
    /// be mixed. Uses [`Timing::default`].
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        spi: SPI,
        cs: CS,
        dc: DC,
        rst: RST,
        busy: BUSY,
        pwr: PWR,
        delay: D,
        panel: Panel,
    ) -> Self {
        Self {
            spi,
            cs,
            dc,
            rst,
            busy,
            pwr,
            delay,
            panel,
            timing: Timing::default(),
            powered: false,
        }
    }

    /// Replace the wait parameters (e.g. shorter timeouts in tests).
    pub fn with_timing(mut self, timing: Timing) -> Self {
        self.timing = timing;
        self
    }

    /// The panel type given to [`new`](Self::new).
    pub fn panel(&self) -> Panel {
        self.panel
    }

    /// Whether [`power_on`](Self::power_on) ran and [`power_off`](Self::power_off) has not.
    pub fn is_powered(&self) -> bool {
        self.powered
    }

    /// Give the pins back, e.g. to put them into sleep hold.
    pub fn release(self) -> (SPI, CS, DC, RST, BUSY, PWR, D) {
        (
            self.spi, self.cs, self.dc, self.rst, self.busy, self.pwr, self.delay,
        )
    }

    /// Switch the panel rail on, reset the controller and select the OTP waveform.
    pub fn power_on(&mut self) -> Result<(), Error<SPI::Error>> {
        self.cs.set_high().map_err(|_| Error::Pin)?;
        self.pwr.set_high().map_err(|_| Error::Pin)?;
        self.powered = true;
        self.delay.delay_ms(10);
        self.reset_and_select_otp()
    }

    /// Show a four-colour frame (NOTE4C) with a full refresh.
    pub fn show_quad(&mut self, frame: &QuadFrame) -> Result<(), Error<SPI::Error>> {
        if self.panel != Panel::Quad {
            return Err(Error::WrongPanel);
        }
        self.ensure_powered()?;
        self.command(cmd::DATA_START)?;
        self.wait_busy(Stage::DataStart)?;
        self.begin_data()?;
        for row in frame.as_bytes().chunks(NATIVE_STRIDE) {
            self.spi.write(row).map_err(Error::Spi)?;
        }
        self.end_data()?;
        self.refresh()
    }

    /// Show a black/white frame (NOTE4) with a full refresh.
    ///
    /// `celsius` selects the OTP waveform; pass the reading of the controller's own
    /// sensor when it is available, or `None` for the official driver's 25 °C fallback.
    pub fn show_mono(
        &mut self,
        frame: &MonoFrame,
        celsius: Option<u8>,
    ) -> Result<(), Error<SPI::Error>> {
        if self.panel != Panel::Mono {
            return Err(Error::WrongPanel);
        }
        self.ensure_powered()?;
        // The controller does not reliably accept a second refresh after POF without a
        // fresh reset and OTP selection (official driver, `PrepareOtpRefresh`).
        self.reset_and_select_otp()?;
        self.command_with_data(cmd::CASCADE_SETTING, &[0x02])?;
        self.command_with_data(
            cmd::TEMP_SET,
            &[mono_temperature_code(celsius.unwrap_or(25))],
        )?;
        self.command(cmd::TEMP_ACTIVATE)?;
        self.wait_busy(Stage::TempActivate)?;
        self.delay.delay_ms(10);

        self.command(cmd::DATA_START)?;
        self.begin_data()?;
        let mut row = [0u8; NATIVE_STRIDE];
        for y in 0..HEIGHT as usize {
            frame.native_row(y, &mut row);
            self.spi.write(&row).map_err(Error::Spi)?;
        }
        self.end_data()?;
        self.refresh()
    }

    /// Deep-sleep the controller and switch the panel rail off. The image stays.
    pub fn power_off(&mut self) -> Result<(), Error<SPI::Error>> {
        if self.powered {
            // Best effort: the rail goes off even if the controller does not answer.
            let sleep = self.command_with_data(cmd::DEEP_SLEEP, &[cmd::DEEP_SLEEP_CHECK]);
            self.pwr.set_low().map_err(|_| Error::Pin)?;
            self.powered = false;
            sleep?;
        }
        Ok(())
    }

    fn ensure_powered(&self) -> Result<(), Error<SPI::Error>> {
        if self.powered {
            Ok(())
        } else {
            Err(Error::NotPowered)
        }
    }

    fn reset_and_select_otp(&mut self) -> Result<(), Error<SPI::Error>> {
        self.rst.set_high().map_err(|_| Error::Pin)?;
        self.delay.delay_ms(10);
        self.rst.set_low().map_err(|_| Error::Pin)?;
        self.delay.delay_ms(20);
        self.rst.set_high().map_err(|_| Error::Pin)?;
        self.delay.delay_ms(10);
        self.wait_busy(Stage::Reset)?;

        if self.panel == Panel::Mono {
            // Official Zectrix value. Other community drivers use 2F 2E; see docs.
            self.command_with_data(cmd::PANEL_SETTING, &[0x2F, 0x0E])?;
        }
        self.command_with_data(cmd::OTP_SELECT, &[0x01])?;
        self.wait_busy(Stage::OtpSelect)
    }

    /// PON → DRF → POF, shared by both panels.
    fn refresh(&mut self) -> Result<(), Error<SPI::Error>> {
        self.command(cmd::POWER_ON)?;
        self.wait_busy(Stage::PowerOn)?;
        self.delay.delay_ms(10);

        self.command_with_data(cmd::DISPLAY_REFRESH, &[0x00])?;
        if self.panel == Panel::Quad {
            self.wait_refresh_started()?;
        }
        self.delay.delay_ms(10);
        self.wait_busy(Stage::Refresh)?;

        self.command_with_data(cmd::POWER_OFF, &[0x00])?;
        self.wait_busy(Stage::PowerOff)?;
        self.delay.delay_ms(20);
        Ok(())
    }

    fn wait_refresh_started(&mut self) -> Result<(), Error<SPI::Error>> {
        let mut waited = 0;
        while self.busy.is_high().map_err(|_| Error::Pin)? {
            if waited >= self.timing.refresh_start_ms {
                return Err(Error::RefreshNotStarted);
            }
            self.delay.delay_ms(1);
            waited += 1;
        }
        Ok(())
    }

    fn wait_busy(&mut self, stage: Stage) -> Result<(), Error<SPI::Error>> {
        let mut waited = 0;
        while self.busy.is_low().map_err(|_| Error::Pin)? {
            if waited >= self.timing.busy_timeout_ms {
                return Err(Error::BusyTimeout(stage));
            }
            let poll = self.timing.poll_ms.max(1);
            self.delay.delay_ms(poll);
            waited += poll;
        }
        Ok(())
    }

    fn command(&mut self, c: u8) -> Result<(), Error<SPI::Error>> {
        self.dc.set_low().map_err(|_| Error::Pin)?;
        self.cs.set_low().map_err(|_| Error::Pin)?;
        let r = self.spi.write(&[c]).and_then(|_| self.spi.flush());
        self.cs.set_high().map_err(|_| Error::Pin)?;
        r.map_err(Error::Spi)
    }

    fn command_with_data(&mut self, c: u8, data: &[u8]) -> Result<(), Error<SPI::Error>> {
        self.command(c)?;
        self.begin_data()?;
        let r = self.spi.write(data);
        self.end_data()?;
        r.map_err(Error::Spi)
    }

    fn begin_data(&mut self) -> Result<(), Error<SPI::Error>> {
        self.dc.set_high().map_err(|_| Error::Pin)?;
        self.cs.set_low().map_err(|_| Error::Pin)
    }

    fn end_data(&mut self) -> Result<(), Error<SPI::Error>> {
        let r = self.spi.flush();
        self.cs.set_high().map_err(|_| Error::Pin)?;
        r.map_err(Error::Spi)
    }
}
