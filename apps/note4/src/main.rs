//! NOTE4 (black/white) bring-up firmware.
//!
//! On every boot it latches power, reads the battery, draws a black/white test card
//! with a status line and refreshes the panel. Unless the 30-minute timer woke it, it
//! then waits 60 s for OK, which opens the partial-refresh lab ([`lab`],
//! docs/INTERACTIVE.md). Then it switches the panel off and deep-sleeps. OK (GPIO0) or
//! the timer wakes it again. The device keeps its power latch held; a real "power off"
//! gesture is on the roadmap (docs/ROADMAP.md).

#![no_std]
#![no_main]
// From the esp-generate 1.4.0 template (Copyright (c) 2024 esp-rs, MIT OR Apache-2.0).
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

use core::fmt::Write as _;

mod lab;

use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_10X20},
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::Text,
};
use esp_backtrace as _;
use esp_hal::{
    delay::Delay,
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull},
    main,
    rtc_cntl::WakeupSource,
    spi::{
        Mode,
        master::{Config as SpiConfig, Spi},
    },
    time::{Duration, Rate},
};
use log::{error, info};
use ssd2683::{MonoFrame, Panel, Ssd2683, Timing};
use static_cell::ConstStaticCell;
use zectrix_board::power::{self, Battery, PowerLatch};

esp_bootloader_esp_idf::esp_app_desc!();

/// 15 000 bytes, kept out of the stack.
static FRAME: ConstStaticCell<MonoFrame> = ConstStaticCell::new(MonoFrame::new());
/// What is on the panel now, for partial refreshes (another 15 000 bytes).
static SHADOW: ConstStaticCell<MonoFrame> = ConstStaticCell::new(MonoFrame::new());

/// After a panic: device off on battery; on USB retry after this long.
const PANIC_RETRY: Duration = Duration::from_secs(30 * 60);

#[unsafe(no_mangle)]
fn custom_halt() -> ! {
    power::halt_after_panic(PANIC_RETRY)
}

const WAKE_INTERVAL: Duration = Duration::from_secs(30 * 60);
/// How long the test card waits for OK (lab screen) before deep sleep.
const AWAKE: Duration = Duration::from_secs(60);

#[main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default());
    // First: keep ourselves powered on battery.
    let _latch = PowerLatch::engage(p.GPIO17);

    esp_println::logger::init_logger_from_env();
    let by_timer = esp_hal::system::wakeup_cause().contains(WakeupSource::Timer);
    info!(
        "NOTE4 firmware {} (timer wake {by_timer})",
        env!("CARGO_PKG_VERSION")
    );

    let delay = Delay::new();
    let mut led = Output::new(p.GPIO3, Level::Low, OutputConfig::default()); // on
    let pull_up = InputConfig::default().with_pull(Pull::Up);
    let mut ok = Input::new(p.GPIO0, pull_up);
    let down = Input::new(p.GPIO18, pull_up);
    let up = Input::new(p.GPIO39, pull_up);
    let charging = Input::new(p.GPIO2, pull_up).is_low();
    let full = Input::new(p.GPIO1, InputConfig::default()).is_high();

    // GPIO18 is also the power key: if it switched us on, wait for its release so it is
    // not read as a button press later (youn-ink does the same).
    let mut waited = 0;
    while down.is_low() && waited < 5_000 {
        delay.delay_millis(10);
        waited += 10;
    }

    let battery_mv = Battery::new(p.ADC1, p.GPIO4).read_mv();
    info!("battery {battery_mv:?} mV, charging {charging}, full {full}");

    let frame = FRAME.take();
    // Raw button levels on the card (1 = released), to check the pins on new hardware.
    let levels = [up.is_high(), down.is_high(), ok.is_high()];
    draw(frame, battery_mv, charging, full, !by_timer, levels);

    let spi = Spi::new(
        p.SPI3,
        SpiConfig::default()
            .with_frequency(Rate::from_mhz(20))
            .with_mode(Mode::_0),
    )
    .expect("SPI config")
    .with_sck(p.GPIO12)
    .with_mosi(p.GPIO13);
    let mut epd = Ssd2683::new(
        spi,
        Output::new(p.GPIO11, Level::High, OutputConfig::default()), // CS
        Output::new(p.GPIO10, Level::High, OutputConfig::default()), // DC
        Output::new(p.GPIO9, Level::High, OutputConfig::default()),  // RST
        Input::new(p.GPIO8, pull_up),                                // BUSY, low = busy
        Output::new(p.GPIO6, Level::Low, OutputConfig::default()),   // panel rail
        delay,
        Panel::Mono,
    )
    // 1 ms BUSY polling so the lab's timings are not rounded to 10 ms.
    .with_timing(Timing {
        poll_ms: 1,
        ..Timing::default()
    });

    let result = epd.power_on().and_then(|_| epd.show_mono(frame, None));
    if let Err(e) = &result {
        error!("display: {e:?}");
    }
    if !by_timer && result.is_ok() {
        let mut buttons = lab::Buttons::new(&up, &down, &ok);
        // Only OK opens the lab. Up/Down are ignored here: GPIO18 doubles as the power key
        // and may read low, which used to end the wait at once and send the device to sleep.
        if lab::wait_for(&mut buttons, &delay, AWAKE, lab::Press::Ok) {
            lab::run(&mut epd, frame, SHADOW.take(), &mut buttons, &delay);
        }
    }
    let _ = epd.power_off();
    let (_spi, _cs, _dc, _rst, _busy, mut rail, _delay) = epd.release();
    power::hold_through_sleep(&mut rail);

    led.set_high(); // off
    info!("sleeping; OK or {} min wakes", WAKE_INTERVAL.as_secs() / 60);
    power::deep_sleep(p.LPWR, &mut ok, Some(WAKE_INTERVAL))
}

fn draw(
    frame: &mut MonoFrame,
    battery_mv: Option<u32>,
    charging: bool,
    full: bool,
    awake: bool,
    levels: [bool; 3],
) {
    let _ = frame.clear(BinaryColor::Off);
    let ink = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);

    let _ = Rectangle::new(Point::new(0, 0), Size::new(400, 80))
        .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
        .draw(frame);
    let paper = MonoTextStyle::new(&FONT_10X20, BinaryColor::Off);
    let _ = Text::new("ZECTRIX NOTE4", Point::new(20, 36), paper).draw(frame);
    let _ = Text::new(
        concat!("Rust firmware v", env!("CARGO_PKG_VERSION")),
        Point::new(20, 64),
        paper,
    )
    .draw(frame);

    // Checkerboard and line gauges to judge ghosting and sharpness.
    for i in 0..8 {
        for j in 0..4 {
            if (i + j) % 2 == 0 {
                let _ = Rectangle::new(Point::new(20 + i * 20, 100 + j * 20), Size::new(20, 20))
                    .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
                    .draw(frame);
            }
        }
    }
    for k in 0..10 {
        let _ = Rectangle::new(
            Point::new(200 + k * 18, 100),
            Size::new(1 + k as u32 / 3, 80),
        )
        .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
        .draw(frame);
    }

    let mut line: heapless::String<48> = heapless::String::new();
    match battery_mv {
        Some(mv) => {
            let _ = write!(
                line,
                "Battery {mv} mV ~{}%",
                zectrix_board::battery::percent(mv)
            );
        }
        None => {
            let _ = write!(line, "Battery: no reading");
        }
    }
    if charging {
        let _ = line.push_str(" chg");
    } else if full {
        let _ = line.push_str(" full");
    }
    let _ = Text::new(&line, Point::new(20, 250), ink).draw(frame);
    let hint = if awake {
        "OK within 60 s = lab"
    } else {
        "OK = wake"
    };
    let _ = Text::new(hint, Point::new(20, 280), ink).draw(frame);
    let mut pins: heapless::String<16> = heapless::String::new();
    let [u, d, o] = levels.map(u8::from);
    let _ = write!(pins, "U{u} D{d} OK{o}");
    let _ = Text::new(&pins, Point::new(260, 280), ink).draw(frame);
}
