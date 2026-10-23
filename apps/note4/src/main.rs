//! NOTE4 (black/white) bring-up firmware.
//!
//! On every boot it latches power, reads the battery, draws a black/white test card
//! with a status line, refreshes the panel, switches the
//! panel off and deep-sleeps. OK (GPIO0) or a 30-minute timer wakes it for the next
//! refresh. The device keeps its power latch held; a real "power off" gesture is on the
//! roadmap (docs/ROADMAP.md).

#![no_std]
#![no_main]
// From the esp-generate 1.4.0 template (Copyright (c) 2024 esp-rs, MIT OR Apache-2.0).
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

use core::fmt::Write as _;

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
    spi::{
        Mode,
        master::{Config as SpiConfig, Spi},
    },
    time::{Duration, Rate},
};
use log::{error, info};
use ssd2683::{MonoFrame, Panel, Ssd2683};
use static_cell::ConstStaticCell;
use zectrix_board::power::{self, Battery, PowerLatch};

esp_bootloader_esp_idf::esp_app_desc!();

/// 15 000 bytes, kept out of the stack.
static FRAME: ConstStaticCell<MonoFrame> = ConstStaticCell::new(MonoFrame::new());

/// After a panic: device off on battery; on USB retry after this long.
const PANIC_RETRY: Duration = Duration::from_secs(30 * 60);

#[unsafe(no_mangle)]
fn custom_halt() -> ! {
    power::halt_after_panic(PANIC_RETRY)
}

const WAKE_INTERVAL: Duration = Duration::from_secs(30 * 60);

#[main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default());
    // First: keep ourselves powered on battery.
    let _latch = PowerLatch::engage(p.GPIO17);

    esp_println::logger::init_logger_from_env();
    info!("NOTE4 firmware {}", env!("CARGO_PKG_VERSION"));

    let delay = Delay::new();
    let mut led = Output::new(p.GPIO3, Level::Low, OutputConfig::default()); // on
    let pull_up = InputConfig::default().with_pull(Pull::Up);
    let mut ok = Input::new(p.GPIO0, pull_up);
    let down = Input::new(p.GPIO18, pull_up);
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
    draw(frame, battery_mv, charging, full);

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
    );

    let result = epd.power_on().and_then(|_| epd.show_mono(frame, None));
    if let Err(e) = &result {
        error!("display: {e:?}");
    }
    let _ = epd.power_off();
    let (_spi, _cs, _dc, _rst, _busy, mut rail, _delay) = epd.release();
    power::hold_through_sleep(&mut rail);

    led.set_high(); // off
    info!("sleeping; OK or {} min wakes", WAKE_INTERVAL.as_secs() / 60);
    power::deep_sleep(p.LPWR, &mut ok, Some(WAKE_INTERVAL))
}

fn draw(frame: &mut MonoFrame, battery_mv: Option<u32>, charging: bool, full: bool) {
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
    let _ = Text::new("OK = refresh", Point::new(20, 280), ink).draw(frame);
}
