//! NOTE4C (black/white/yellow/red) firmware: a thin frame client.
//!
//! Every wake: latch power, join Wi-Fi, fetch a finished 30 000-byte frame from
//! `FRAME_URL`, show it, and deep-sleep until the time the server sent in
//! `X-Next-Refresh`. Wi-Fi credentials, the URL, the optional device key and trust anchor
//! are compiled in from the environment or a git-ignored `.env` (see build.rs and README).
//! The server side is described in docs/FRAME_PROTOCOL.md.
//!
//! - 304 Not Modified: the panel is left alone (no 10–25 s refresh).
//! - A press of OK always fetches without the ETag, so the board redraws on demand.
//! - A failed panel refresh drops the ETag (so the next fetch redraws) and retries in
//!   15 minutes once; after that the server's schedule applies again, to spare a weak
//!   battery.
//! - Each request tells the server how the device is doing (see docs/FRAME_PROTOCOL.md):
//!   `X-Wake`, the previous wake's `X-Battery-mV` and the last refresh's `X-Last-Display`.
//! - Errors never replace the content with something that looks current: the panel keeps
//!   the last frame and the device retries in 15 minutes. Only before the first frame, or
//!   after [`FAILS_BEFORE_CARD`] failures in a row, it draws the diagnostic card instead.
//!
//! Diagnostics and the previous run's panic message go on the panel rather than only to
//! the log: over WSL USB/IP the serial port re-attaches too slowly to catch a run.

#![no_std]
#![no_main]
// From the esp-generate 1.4.0 template (Copyright (c) 2024 esp-rs, MIT OR Apache-2.0).
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

use core::fmt::Write as _;

use embassy_executor::Spawner;
use embassy_time::{Duration, with_timeout};
use embedded_graphics::{
    mono_font::{
        MonoTextStyle,
        ascii::{FONT_9X15, FONT_9X15_BOLD, FONT_10X20},
    },
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::Text,
};
use esp_hal::{
    clock::CpuClock,
    delay::Delay,
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull},
    rng::Trng,
    rtc_cntl::WakeupSource,
    spi::{
        Mode,
        master::{Config as SpiConfig, Spi},
    },
    time::{Duration as HalDuration, Instant, Rate},
    timer::timg::TimerGroup,
};
use log::{error, info, warn};
use mbedtls_rs::Tls;
use mbedtls_rs::sys::hook::backend::esp::EspAccel;
use ssd2683::{NATIVE_FRAME_BYTES, Panel, QuadColor, QuadFrame, Ssd2683};
use static_cell::{ConstStaticCell, StaticCell};
use tinyrlibc as _;
use zectrix_board::power::{self, Battery, PowerLatch};
use zectrix_net::{https, wifi};

mod crash;
mod state;

esp_bootloader_esp_idf::esp_app_desc!();

/// 30 000 bytes, kept out of the stack. The frame body is received straight into it.
static FRAME: ConstStaticCell<QuadFrame> = ConstStaticCell::new(QuadFrame::new());
static TRNG: StaticCell<Trng> = StaticCell::new();

// Set by build.rs from the environment or `.env`.
const WIFI_SSID: &str = env!("WIFI_SSID");
const WIFI_PASSWORD: &str = env!("WIFI_PASSWORD");
/// Bearer token for the frame server; empty means no `Authorization` header.
const FRAME_KEY: &str = env!("FRAME_KEY");
/// The `Authorization` value, built at compile time (build.rs checks that it fits the request).
const FRAME_AUTH: &str = concat!("Bearer ", env!("FRAME_KEY"));

/// Where the frame comes from: the parts of `FRAME_URL`.
const FRAME_SERVER: https::Endpoint<'static> = https::Endpoint {
    host: match core::ffi::CStr::from_bytes_with_nul(concat!(env!("FRAME_HOST"), "\0").as_bytes()) {
        Ok(host) => host,
        Err(_) => panic!("FRAME_HOST contains a NUL"),
    },
    port: match u16::from_str_radix(env!("FRAME_PORT"), 10) {
        Ok(port) => port,
        Err(_) => panic!("FRAME_PORT is not a port number"),
    },
    path: env!("FRAME_PATH"),
    ca_pem: FRAME_CA_PEM,
};

/// Trust anchor for the frame server: `FRAME_CA_PEM` if given, else ISRG Root X1.
#[cfg(frame_custom_ca)]
const FRAME_CA_PEM: &core::ffi::CStr = match core::ffi::CStr::from_bytes_with_nul(
    concat!(include_str!(env!("FRAME_CA_FILE")), "\0").as_bytes(),
) {
    Ok(pem) => pem,
    Err(_) => panic!("FRAME_CA_PEM contains a NUL"),
};
#[cfg(not(frame_custom_ca))]
const FRAME_CA_PEM: &core::ffi::CStr = zectrix_net::ISRG_ROOT_X1_PEM;

/// Bounds association and DHCP separately.
const WIFI_TIMEOUT: Duration = Duration::from_secs(20);
/// Bounds the frame fetch. The server may render for a while (forecast, warnings,
/// calendar fetched in parallel); it asks devices to allow 90 s.
const FETCH_TIMEOUT: Duration = Duration::from_secs(90);

/// Sleep when the server gives no `X-Next-Refresh`.
const DEFAULT_SLEEP_S: u32 = 2 * 3600;
/// Retry delay after a failed fetch.
const RETRY_SLEEP_S: u32 = 15 * 60;
/// Bounds for the server's choice, so a bad header cannot make the board sleep for days
/// or wake every minute.
const MIN_SLEEP_S: u32 = 5 * 60;
const MAX_SLEEP_S: u32 = 12 * 3600;
/// After this many failures in a row (about 2 h at 15-minute retries), the diagnostic card
/// replaces the last frame so a stale frame does not pass for current.
const FAILS_BEFORE_CARD: u32 = 8;

/// Line of text for the diagnostic card.
type Line = heapless::String<44>;

/// What happened, for the diagnostic card.
struct Report {
    wifi: Line,
    ip: Line,
    fetch: Line,
    detail: Line,
    ok: [bool; 2],
}

/// The result of one wake.
enum Outcome {
    /// A new frame is in `FRAME`.
    Frame {
        next_s: u32,
    },
    NotModified {
        next_s: u32,
    },
    Failed,
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    // First: keep ourselves powered on battery.
    let _latch = PowerLatch::engage(p.GPIO17);

    esp_println::logger::init_logger_from_env();
    // Wi-Fi and mbedtls allocate; the bootloader's RAM is free after boot.
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 112 * 1024);

    let t0 = Instant::now();
    let cause = esp_hal::system::wakeup_cause();
    let by_timer = cause.contains(WakeupSource::Timer);
    let reset = esp_hal::system::reset_reason();
    info!(
        "NOTE4C firmware {} (reset {reset:?}, timer wake {by_timer})",
        env!("CARGO_PKG_VERSION"),
    );
    // `X-Wake`: the wake-up sources (e.g. `Timer`, `Gpio` for OK; `none` after a reset),
    // then the reset reason.
    let mut wake: heapless::String<48> = heapless::String::new();
    for (i, source) in cause.iter().enumerate() {
        let _ = write!(wake, "{}{source:?}", if i > 0 { "+" } else { "" });
    }
    if wake.is_empty() {
        let _ = wake.push_str("none");
    }
    let _ = write!(wake, " {reset:?}");

    let timg0 = TimerGroup::new(p.TIMG0);
    esp_rtos::start(timg0.timer0, p.FROM_CPU_INTR0);

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

    let crashed = crash::take();
    if let Some(msg) = &crashed {
        error!("previous run panicked: {msg}");
    }
    let mut st = state::load();
    // Only a timer wake may reuse the ETag; OK, reset or power-on always redraw.
    let etag = if by_timer { st.etag() } else { None };

    let frame = FRAME.take();
    let mut report = Report {
        wifi: Line::new(),
        ip: Line::new(),
        fetch: Line::new(),
        detail: Line::new(),
        ok: [false; 2],
    };
    let outcome = fetch_frame(
        spawner,
        p.WIFI,
        (p.SHA, p.RSA, p.AES),
        (etag.as_deref(), &wake),
        frame,
        &mut st,
        &mut report,
    )
    .await;
    info!("fetch done at {} ms", t0.elapsed().as_millis());

    // The ADC is read only after networking: an ADC left configured shares the RF/ADC
    // path with the radio (see docs/ROADMAP.md M1).
    let battery_mv = Battery::new(p.ADC1, p.GPIO4).read_mv();
    info!("battery {battery_mv:?} mV, charging {charging}, full {full}");
    st.battery_mv = battery_mv.unwrap_or(0);

    let (show, sleep_s) = match outcome {
        Outcome::Frame { next_s } => {
            st.fails = 0;
            st.have_frame = true;
            (Show::Frame, next_s)
        }
        Outcome::NotModified { next_s } => {
            st.fails = 0;
            (Show::Nothing, next_s)
        }
        Outcome::Failed => {
            st.fails = st.fails.saturating_add(1);
            let card = !st.have_frame || st.fails >= FAILS_BEFORE_CARD || !by_timer;
            if card {
                // The card replaces the frame, so the next success must redraw.
                st.have_frame = false;
                st.clear_etag();
            }
            (if card { Show::Card } else { Show::Nothing }, RETRY_SLEEP_S)
        }
    };
    // Stored again after the panel refresh; storing now keeps the fetch result if the
    // refresh ends in a reset (e.g. a brownout).
    state::store(&st);

    let mut sleep_s = sleep_s;
    if !matches!(show, Show::Nothing) {
        let what = if matches!(show, Show::Card) {
            draw_card(
                frame,
                &report,
                crashed.as_deref(),
                &st,
                battery_mv,
                charging,
                full,
            );
            "card"
        } else {
            "frame"
        };
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
            Panel::Quad,
        );
        info!("display: start at {} ms", t0.elapsed().as_millis());
        let t = Instant::now();
        let result = epd.power_on().and_then(|_| epd.show_quad(frame));
        let took = Tenths(t.elapsed().as_millis() as u32);
        st.display.clear();
        match result {
            Ok(()) => {
                info!("display: done at {} ms", t0.elapsed().as_millis());
                let _ = write!(st.display, "{what} ok {took}s");
                st.display_fails = 0;
            }
            Err(e) => {
                error!("display: {e:?} at {} ms", t0.elapsed().as_millis());
                let _ = write!(st.display, "{what} {e:?} {took}s");
                st.display_fails = st.display_fails.saturating_add(1);
                // The panel still shows something older: the next fetch must not get a
                // 304 for this frame.
                st.clear_etag();
                if st.display_fails == 1 {
                    sleep_s = RETRY_SLEEP_S;
                }
            }
        }
        state::store(&st);
        let _ = epd.power_off();
        let (_spi, _cs, _dc, _rst, _busy, mut rail, _delay) = epd.release();
        power::hold_through_sleep(&mut rail);
    } else {
        info!("display: unchanged");
        // Keep the panel rail off through sleep (it was never switched on).
        let mut rail = Output::new(p.GPIO6, Level::Low, OutputConfig::default());
        power::hold_through_sleep(&mut rail);
    }

    led.set_high(); // off
    let sleep_s = sleep_s.clamp(MIN_SLEEP_S, MAX_SLEEP_S);
    info!("sleeping {sleep_s} s; OK wakes earlier");
    power::deep_sleep(
        p.LPWR,
        &mut ok,
        Some(HalDuration::from_secs(u64::from(sleep_s))),
    )
}

enum Show {
    Frame,
    Card,
    Nothing,
}

/// Join Wi-Fi and fetch the frame into `frame`. Fills `report` for the diagnostic card.
async fn fetch_frame(
    spawner: Spawner,
    wifi_periph: esp_hal::peripherals::WIFI<'static>,
    (sha, rsa, aes): (
        esp_hal::peripherals::SHA<'static>,
        esp_hal::peripherals::RSA<'static>,
        esp_hal::peripherals::AES<'static>,
    ),
    // The ETag to send, and how this wake came about (`X-Wake`).
    (etag, wake): (Option<&str>, &str),
    frame: &mut QuadFrame,
    st: &mut state::State,
    r: &mut Report,
) -> Outcome {
    let link =
        match wifi::connect(spawner, wifi_periph, WIFI_SSID, WIFI_PASSWORD, WIFI_TIMEOUT).await {
            Ok(link) => link,
            Err(wifi::WifiError::Associate(sv)) => {
                let _ = write!(r.wifi, "Wi-Fi {WIFI_SSID}: no join x{}", sv.attempts);
                let _ = match sv.best {
                    Some((ch, rssi, _)) => write!(r.ip, "seen x{} ch{ch} {rssi} dBm", sv.matches),
                    None => write!(r.ip, "not in scan ({} APs seen)", sv.total),
                };
                if let Some(reason) = sv.last_reason {
                    let _ = write!(r.detail, "reason {reason:?}");
                }
                return Outcome::Failed;
            }
            Err(wifi::WifiError::Dhcp(sv)) => {
                let _ = write!(r.wifi, "Wi-Fi {WIFI_SSID}: joined, no DHCP");
                let _ = write!(
                    r.ip,
                    "{} dBm link {} rx {} tx {}",
                    sv.rssi.unwrap_or(0),
                    sv.link_up,
                    zectrix_net::counting::rx_frames(),
                    zectrix_net::counting::tx_frames()
                );
                return Outcome::Failed;
            }
            Err(e) => {
                let _ = write!(r.wifi, "Wi-Fi {WIFI_SSID}: {e:?}");
                return Outcome::Failed;
            }
        };
    r.ok[0] = true;
    let _ = write!(
        r.wifi,
        "Wi-Fi {} ch{} {} dBm",
        WIFI_SSID,
        link.channel,
        link.rssi.unwrap_or(0)
    );
    let _ = write!(
        r.ip,
        "{}  assoc {}s dhcp {}s",
        link.ip,
        Tenths(link.associate_ms),
        Tenths(link.dhcp_ms)
    );

    // The running radio is the entropy source (no ADC TrngSource, see wifi::connect).
    let Ok(trng) = Trng::try_new() else {
        let _ = write!(r.fetch, "TRNG: no entropy source");
        return Outcome::Failed;
    };
    let tls = match Tls::new(TRNG.init(trng)) {
        Ok(tls) => tls,
        Err(e) => {
            let _ = write!(r.fetch, "TLS init: {e:?}");
            return Outcome::Failed;
        }
    };
    // Hardware SHA/RSA/AES. In software the certificate chain (up to RSA-4096 ISRG Root
    // X1) takes long enough that, with this task busy, TCP can stall mid-handshake.
    let mut accel = EspAccel::new().with_sha(sha).with_rsa(rsa).with_aes(aes);
    let queue = accel.start();
    // SAFETY: `queue` services the hooked work queues and lives until the end of this
    // function, after the TLS session is closed.
    let _hooks = unsafe { queue.hook() };

    let mut battery: heapless::String<10> = heapless::String::new();
    if st.battery_mv != 0 {
        let _ = write!(battery, "{}", st.battery_mv);
    }
    let display = st.display.clone();
    // Room for all five headers below; build.rs sizes the request for the same set.
    let mut headers: heapless::Vec<(&str, &str), 5> = heapless::Vec::new();
    let mut add = |name, value| {
        headers
            .push((name, value))
            .expect("headers: capacity 5 covers every header");
    };
    if !FRAME_KEY.is_empty() {
        add("Authorization", FRAME_AUTH);
    }
    add("X-Wake", wake);
    if !battery.is_empty() {
        add("X-Battery-mV", battery.as_str());
    }
    if !display.is_empty() {
        add("X-Last-Display", display.as_str());
    }
    if let Some(tag) = etag {
        add("If-None-Match", tag);
    }

    let t = embassy_time::Instant::now();
    let body = frame.as_bytes_mut();
    let resp = match with_timeout(
        FETCH_TIMEOUT,
        https::get(link.stack, tls.reference(), FRAME_SERVER, &headers, body),
    )
    .await
    {
        Ok(Ok(resp)) => resp,
        Ok(Err(e)) => {
            let _ = write!(
                r.fetch,
                "frame {}s: {e:?}",
                Tenths(t.elapsed().as_millis() as u32)
            );
            return Outcome::Failed;
        }
        Err(_) => {
            let _ = write!(r.fetch, "frame: timeout {}s", FETCH_TIMEOUT.as_secs());
            return Outcome::Failed;
        }
    };
    let next_s = resp.head.next_refresh_s.unwrap_or(DEFAULT_SLEEP_S);
    let _ = write!(
        r.fetch,
        "frame {} {}B tls {}s all {}s",
        resp.head.status,
        resp.body_len,
        Tenths(resp.tls_ms),
        Tenths(resp.total_ms)
    );
    match resp.head.status {
        200 if resp.body_len == NATIVE_FRAME_BYTES && resp.head.octet_stream => {
            r.ok[1] = true;
            match &resp.head.etag {
                Some(tag) => st.set_etag(tag),
                None => st.clear_etag(),
            }
            Outcome::Frame { next_s }
        }
        304 => {
            r.ok[1] = true;
            Outcome::NotModified { next_s }
        }
        status => {
            warn!("frame: unexpected {status}, {} B", resp.body_len);
            // Show the start of an error body (JSON message) on the card.
            let n = resp.body_len.min(40);
            let text = core::str::from_utf8(&frame.as_bytes()[..n]).unwrap_or("");
            let _ = write!(r.detail, "{}", text.trim());
            Outcome::Failed
        }
    }
}

/// Milliseconds shown as seconds with one decimal.
struct Tenths(u32);

impl core::fmt::Display for Tenths {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}.{}", self.0 / 1000, (self.0 % 1000) / 100)
    }
}

fn draw_card(
    frame: &mut QuadFrame,
    r: &Report,
    crashed: Option<&str>,
    st: &state::State,
    battery_mv: Option<u32>,
    charging: bool,
    full: bool,
) {
    let _ = frame.clear(QuadColor::White);
    let title = MonoTextStyle::new(&FONT_10X20, QuadColor::Red);
    let black = MonoTextStyle::new(&FONT_9X15, QuadColor::Black);
    let bold = MonoTextStyle::new(&FONT_9X15_BOLD, QuadColor::Black);
    let red = MonoTextStyle::new(&FONT_9X15_BOLD, QuadColor::Red);

    let _ = Text::new(
        concat!("NOTE4C frame v", env!("CARGO_PKG_VERSION")),
        Point::new(8, 22),
        title,
    )
    .draw(frame);
    let mut sub: heapless::String<44> = heapless::String::new();
    let _ = write!(sub, "no frame - failed {}x, retry 15 min", st.fails);
    let _ = Text::new(&sub, Point::new(100, 48), black).draw(frame);

    // Status lamps for Wi-Fi and the frame fetch: yellow = ok, red = failed.
    for (i, ok) in r.ok.iter().enumerate() {
        let colour = if *ok {
            QuadColor::Yellow
        } else {
            QuadColor::Red
        };
        let lamp = Rectangle::new(Point::new(8 + i as i32 * 44, 34), Size::new(36, 20));
        let _ = lamp
            .into_styled(PrimitiveStyle::with_fill(colour))
            .draw(frame);
        let _ = lamp
            .into_styled(PrimitiveStyle::with_stroke(QuadColor::Black, 2))
            .draw(frame);
    }

    let lines: [(&str, bool); 4] = [
        (&r.wifi, r.ok[0]),
        (&r.ip, r.ok[0]),
        (&r.fetch, r.ok[1]),
        (&r.detail, r.ok[1]),
    ];
    for (i, (text, ok)) in lines.iter().enumerate() {
        if text.is_empty() {
            continue;
        }
        let style = match (i, *ok) {
            (_, false) => red,
            (0, true) => bold,
            _ => black,
        };
        let _ = Text::new(text, Point::new(8, 80 + i as i32 * 24), style).draw(frame);
    }

    // The last panel refresh, if it failed.
    if st.display_fails > 0 {
        let mut line: heapless::String<48> = heapless::String::new();
        let _ = write!(line, "panel: {}", st.display);
        let _ = Text::new(&line, Point::new(8, 176), red).draw(frame);
    }

    // The previous run's panic, in red, split over two lines.
    if let Some(msg) = crashed {
        let (a, b) = msg.split_at(msg.floor_char_boundary(42));
        let _ = Text::new(a, Point::new(8, 198), red).draw(frame);
        let b = &b[..b.floor_char_boundary(42)];
        let _ = Text::new(b, Point::new(8, 212), red).draw(frame);
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
    let _ = Text::new(&line, Point::new(8, 236), bold).draw(frame);
    let _ = Text::new("OK = try again now", Point::new(8, 280), black).draw(frame);
}
