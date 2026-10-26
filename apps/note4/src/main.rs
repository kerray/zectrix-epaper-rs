//! NOTE4 (black/white) firmware: an on-device menu driven by the three buttons.
//!
//! * Reset or OK wakes it into the menu ([`zectrix_screen::menu`]). Up/Down move, OK opens
//!   a submenu or runs the item, every submenu has a Back row on top, Up held goes back.
//! * Each change is one partial refresh of whatever changed ([`zectrix_screen::Presenter`];
//!   ~0.8 s on the NOTE4), with a full refresh after 16 partials. Up/Down act on press.
//! * Presses made during a refresh are logged ([`input`]) and applied together afterwards,
//!   then shown with one refresh: three quick Downs cost two refreshes, not three.
//! * After 60 s without a press the device shows a sleep screen (full refresh) and goes to
//!   deep sleep; OK wakes it.
//! * Down held 3 s switches it off (the power latch is released; on USB power it sleeps
//!   instead).
//!
//! Lights, Calendar, Weather and Last answer are placeholders until the device fetches
//! screens from a server (docs/INTERACTIVE.md, I3/I4). System holds local screens: device
//! info, a button test and the partial-refresh lab ([`lab`]).

#![no_std]
#![no_main]
// From the esp-generate 1.4.0 template (Copyright (c) 2024 esp-rs, MIT OR Apache-2.0).
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

use core::fmt::Write as _;

mod crash;
mod input;
mod lab;
#[cfg(screen_server)]
mod remote;

use embassy_executor::Spawner;
use embassy_time::Timer;
use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_10X20},
    pixelcolor::BinaryColor,
};
use esp_hal::{
    clock::CpuClock,
    delay::Delay,
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull},
    rtc_cntl::WakeupSource,
    spi::{
        Mode as SpiMode,
        master::{Config as SpiConfig, Spi},
    },
    time::{Instant, Rate},
    timer::timg::TimerGroup,
};
use input::{ButtonDelay, Sampler, now_ms};
use log::{error, info, warn};
use ssd2683::{MonoFrame, Panel, Ssd2683, Timing};
use static_cell::{ConstStaticCell, StaticCell};
#[cfg(screen_server)]
use tinyrlibc as _;
use zectrix_board::power::{self, Battery, PowerLatch};
use zectrix_screen::{
    ButtonConfig, Event, Layout, Merge, Presenter, RefreshPolicy, Screen, Session, Shown, Update,
    menu::{Item, Menu, Nav, Outcome},
    view::{self, Label, Theme},
};

esp_bootloader_esp_idf::esp_app_desc!();

/// 15 000 bytes, kept out of the stack.
static FRAME: ConstStaticCell<MonoFrame> = ConstStaticCell::new(MonoFrame::new());
/// What is on the panel now, for partial refreshes (another 15 000 bytes).
static SHADOW: ConstStaticCell<MonoFrame> = ConstStaticCell::new(MonoFrame::new());
/// The buttons, shared by the main loop and the panel driver's delay.
static SAMPLER: StaticCell<Sampler> = StaticCell::new();
/// The last screen bundle from the server (a 10-row list is about 35 KB).
static BUNDLE: ConstStaticCell<[u8; BUNDLE_MAX]> = ConstStaticCell::new([0; BUNDLE_MAX]);
const BUNDLE_MAX: usize = 40 * 1024;

/// Partial refreshes before a full one. Ghosting was hard to see after many more.
const MAX_PARTIALS: u16 = 16;
const BUTTON_POLL_MS: u32 = 10;

/// What a menu item does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Open(View),
    /// Fetch a screen from the server.
    Fetch(Source),
    Lab,
    FullRefresh,
    Sleep,
}

/// A server screen: its id in `screen=<id>` and a title for the loading and error screens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Source {
    id: &'static str,
    title: &'static str,
}

/// A screen other than the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Info,
    ButtonTest,
    /// The bundle in [`Ui::bundle`].
    Server(Source),
    Loading(Source),
    /// Loading failed; [`Ui::error`] says why.
    Failed(Source),
    Asleep,
    Off,
    /// The previous run panicked; [`Ui::crash`] has the message.
    Crashed,
}

const fn fetch(id: &'static str, title: &'static str) -> Action {
    Action::Fetch(Source { id, title })
}

static SYSTEM: Menu<Action> = Menu::new(
    "System",
    &[
        Item::action("Device info", Action::Open(View::Info)),
        Item::action("Button test", Action::Open(View::ButtonTest)),
        Item::action("Refresh lab", Action::Lab),
        Item::action("Full refresh", Action::FullRefresh),
        Item::action("Sleep now", Action::Sleep),
    ],
);

static ROOT: Menu<Action> = Menu::new(
    "ZECTRIX NOTE4",
    &[
        Item::action("Lights", fetch("lights", "Lights")),
        Item::action("Calendar", fetch("calendar", "Calendar")),
        Item::action("Weather", fetch("weather", "Weather")),
        Item::action("Last answer", fetch("answer", "Last answer")),
        Item::action("Server demo", fetch("demo", "Server demo")),
        Item::submenu("System", &SYSTEM),
    ],
);

/// Readings shown on the info screen.
struct Status {
    battery_mv: Option<u32>,
    charging: bool,
    full: bool,
    woke_by_ok: bool,
}

struct Ui {
    nav: Nav<Action>,
    /// `None` while the menu is shown.
    view: Option<View>,
    theme: Theme<MonoTextStyle<'static, BinaryColor>>,
    presenter: Presenter,
    /// Button test: the last event and the raw levels (up, down, ok; true = pressed).
    last_event: Option<Event>,
    levels: [bool; 3],
    /// The server screen: bundle bytes, length and selected row.
    bundle: &'static mut [u8; BUNDLE_MAX],
    bundle_len: usize,
    bundle_sel: Option<usize>,
    /// Why the last server screen failed.
    error: Label,
    /// The previous run's panic message, shown once at boot.
    crash: Option<heapless::String<120>>,
}

impl Ui {
    fn draw(&mut self, frame: &mut MonoFrame, status: &Status) {
        if let Some(View::Server(_)) = self.view
            && let Ok(mut screen) = Screen::new(&mut self.bundle[..self.bundle_len])
        {
            if let Some(sel) = self.bundle_sel {
                screen.select(sel, frame, Merge::Always);
            }
            screen.compose(frame);
            return;
        }
        let t = &self.theme;
        let Some(view) = self.view else {
            let partials = self.presenter.policy().partials();
            let _ = view::draw_menu(frame, t, &self.nav, |a, out| match a {
                Action::FullRefresh => write!(out, "{partials}/{MAX_PARTIALS}"),
                Action::Fetch(_) if !cfg!(screen_server) => out.write_str("n/a"),
                _ => Ok(()),
            });
            return;
        };
        let mut lines: [Label; 6] = Default::default();
        let title = match view {
            View::Info => {
                let _ = write!(lines[0], "Firmware {}", env!("CARGO_PKG_VERSION"));
                match status.battery_mv {
                    Some(mv) => {
                        let _ = write!(
                            lines[1],
                            "Battery {mv} mV, ~{}%",
                            zectrix_board::battery::percent(mv)
                        );
                    }
                    None if cfg!(screen_server) => {
                        let _ = lines[1].push_str("Battery: read after Wi-Fi");
                    }
                    None => {
                        let _ = lines[1].push_str("Battery: no reading");
                    }
                }
                let _ = lines[2].push_str(if status.charging {
                    "Charging"
                } else if status.full {
                    "Charged"
                } else {
                    "On battery or USB"
                });
                let up_s = Instant::now().duration_since_epoch().as_secs();
                let _ = write!(lines[3], "Awake {} min {} s", up_s / 60, up_s % 60);
                let _ = lines[4].push_str(if status.woke_by_ok {
                    "Woken by OK"
                } else {
                    "Started by reset"
                });
                let _ = write!(
                    lines[5],
                    "Partials since full: {}",
                    self.presenter.policy().partials()
                );
                "Device info"
            }
            View::ButtonTest => {
                let [u, d, o] = self.levels.map(|p| if p { "down" } else { "up" });
                let _ = write!(lines[0], "Up {u}  Down {d}  OK {o}");
                let _ = match self.last_event {
                    Some(e) => write!(lines[1], "Last: {e:?}"),
                    None => lines[1].write_str("Last: -"),
                };
                let _ = lines[3].push_str("Hold Up 1 s to go back.");
                let _ = lines[4].push_str("Hold OK: talk, Down 3 s: off.");
                "Button test"
            }
            View::Loading(src) => {
                let _ = lines[0].push_str("Loading...");
                let _ = lines[1].push_str("Wi-Fi and server,");
                let _ = lines[2].push_str("a few seconds the first time.");
                src.title
            }
            View::Failed(src) | View::Server(src) => {
                let _ = lines[0].push_str("Could not load:");
                let _ = lines[1].push_str(&self.error);
                let _ = write!(lines[2], "free heap {} KB", esp_alloc::HEAP.free() / 1024);
                let _ = lines[3].push_str("OK: back");
                src.title
            }
            View::Crashed => {
                let _ = lines[0].push_str("The last run crashed:");
                if let Some(msg) = &self.crash {
                    // 38 characters fit a row of the 10x20 font.
                    let mut rest = msg.as_str();
                    for line in lines.iter_mut().skip(1).take(4) {
                        let cut = rest.char_indices().nth(38).map_or(rest.len(), |(i, _)| i);
                        let _ = line.push_str(&rest[..cut]);
                        rest = &rest[cut..];
                    }
                }
                let _ = lines[5].push_str("OK: menu");
                "ZECTRIX NOTE4"
            }
            View::Asleep => {
                let _ = lines[0].push_str("Asleep.");
                let _ = lines[1].push_str("OK opens the menu.");
                if let Some(mv) = status.battery_mv {
                    let _ = write!(
                        lines[3],
                        "Battery ~{}%",
                        zectrix_board::battery::percent(mv)
                    );
                }
                "ZECTRIX NOTE4"
            }
            View::Off => {
                let _ = lines[0].push_str("Switched off.");
                let _ = lines[1].push_str("The power key (Down)");
                let _ = lines[2].push_str("switches it on.");
                "ZECTRIX NOTE4"
            }
        };
        let back = matches!(
            view,
            View::Info | View::ButtonTest | View::Failed(_) | View::Crashed
        );
        let _ = view::draw_title(frame, t, title, back.then_some("OK: back"));
        let _ = view::draw_lines(frame, t, lines.iter().map(|l| l.as_str()));
    }
}

/// What the main loop does after an event.
enum Effect {
    None,
    Redraw(Update),
    Fetch(Source),
    Lab,
    FullRefresh,
    Sleep,
    Off,
}

impl Ui {
    fn handle(&mut self, event: Event) -> Effect {
        if event == Event::Off {
            return Effect::Off;
        }
        if let Some(View::Server(src)) = self.view {
            return match event {
                Event::Up | Event::Down => {
                    self.move_server(event == Event::Down);
                    Effect::Redraw(Update::Change)
                }
                // OK reloads; actions on rows come with the event endpoint.
                Event::Select => Effect::Fetch(src),
                Event::Back => {
                    self.view = None;
                    Effect::Redraw(Update::NewScreen)
                }
                _ => Effect::None,
            };
        }
        if let Some(View::Loading(_)) = self.view {
            return Effect::None;
        }
        if let Some(view) = self.view {
            if view == View::ButtonTest {
                self.last_event = Some(event);
                if event != Event::Back {
                    return Effect::Redraw(Update::Change);
                }
            }
            return match event {
                Event::Select | Event::Back => {
                    self.view = None;
                    Effect::Redraw(Update::NewScreen)
                }
                // Info: Up/Down re-read the values.
                Event::Up | Event::Down if view == View::Info => Effect::Redraw(Update::Change),
                _ => Effect::None,
            };
        }
        match self.nav.handle(event) {
            Outcome::Activate(Action::Open(view)) => {
                self.view = Some(view);
                Effect::Redraw(Update::NewScreen)
            }
            Outcome::Activate(Action::Fetch(src)) => Effect::Fetch(src),
            Outcome::Activate(Action::Lab) => Effect::Lab,
            Outcome::Activate(Action::FullRefresh) => Effect::FullRefresh,
            Outcome::Activate(Action::Sleep) => Effect::Sleep,
            o if o.page_changed() => Effect::Redraw(Update::NewScreen),
            o if o.redraw() => Effect::Redraw(Update::Change),
            // Talk (OK held) comes with voice; Back on the root does nothing.
            _ => Effect::None,
        }
    }
}

impl Ui {
    /// Move the server screen's selection one selectable row, wrapping around.
    fn move_server(&mut self, down: bool) {
        let Ok(layout) = Layout::parse(&self.bundle[..self.bundle_len]) else {
            return;
        };
        let rows = layout.selectable();
        if rows.is_empty() {
            return;
        }
        let cur = self.bundle_sel.unwrap_or(rows.start);
        self.bundle_sel = Some(if down {
            if cur + 1 >= rows.end {
                rows.start
            } else {
                cur + 1
            }
        } else if cur <= rows.start {
            rows.end - 1
        } else {
            cur - 1
        });
    }

    /// The result of a fetch: show the bundle, or why it failed.
    fn loaded(&mut self, src: Source, result: Result<usize, Label>) {
        match result.and_then(|len| match Layout::parse(&self.bundle[..len]) {
            Ok(layout) => Ok((len, layout)),
            Err(e) => {
                let mut why = Label::new();
                let _ = write!(why, "bad screen: {e:?}");
                Err(why)
            }
        }) {
            Ok((len, layout)) => {
                self.bundle_len = len;
                self.bundle_sel =
                    (!layout.selectable().is_empty()).then(|| layout.initial_selected());
                self.view = Some(View::Server(src));
            }
            Err(why) => {
                self.error = why;
                self.view = Some(View::Failed(src));
            }
        }
    }
}

/// The background Wi-Fi start.
#[cfg(screen_server)]
enum NetState {
    Starting,
    Up(remote::Net),
    Failed(Label),
}

#[cfg(screen_server)]
impl From<Result<remote::Net, Label>> for NetState {
    fn from(r: Result<remote::Net, Label>) -> Self {
        match r {
            Ok(net) => NetState::Up(net),
            Err(why) => {
                warn!("network: {why}");
                NetState::Failed(why)
            }
        }
    }
}

/// The battery ADC, set up on the first read.
enum BatteryAdc {
    Idle(
        esp_hal::peripherals::ADC1<'static>,
        esp_hal::peripherals::GPIO4<'static>,
    ),
    Ready(Battery<'static>),
    Taken,
}

impl BatteryAdc {
    fn read_mv(&mut self) -> Option<u32> {
        if let BatteryAdc::Idle(..) = self
            && let BatteryAdc::Idle(adc, pin) = core::mem::replace(self, BatteryAdc::Taken)
        {
            *self = BatteryAdc::Ready(Battery::new(adc, pin));
        }
        match self {
            BatteryAdc::Ready(b) => b.read_mv(),
            _ => None,
        }
    }
}

/// Draw the current screen and show it with as small a refresh as the policy allows.
fn show(
    ui: &mut Ui,
    epd: &mut lab::Epd,
    frame: &mut MonoFrame,
    shadow: &mut MonoFrame,
    status: &Status,
    update: Update,
) {
    ui.draw(frame, status);
    let t = Instant::now();
    match ui.presenter.show(epd, frame, shadow, update) {
        Ok(Shown::Unchanged) => {}
        Ok(shown) => info!("{shown:?} in {} ms", t.elapsed().as_millis()),
        Err(e) => error!("display: {e:?}"),
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    // First: keep ourselves powered on battery.
    let latch = PowerLatch::engage(p.GPIO17);

    esp_println::logger::init_logger_from_env();
    // Wi-Fi and mbedtls allocate; the bootloader's RAM is free after boot.
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 112 * 1024);
    let timg0 = TimerGroup::new(p.TIMG0);
    esp_rtos::start(timg0.timer0, p.FROM_CPU_INTR0);
    let crashed = crash::take();
    // Wi-Fi starts at boot in the background, so it is usually up by the time a server
    // screen opens. The first poll initialises the radio before the panel is touched, as
    // on the NOTE4C.
    #[cfg(screen_server)]
    let mut net_start = core::pin::pin!(remote::start(
        spawner,
        remote::Parts {
            wifi: p.WIFI,
            sha: p.SHA,
            rsa: p.RSA,
            aes: p.AES,
        },
    ));
    // After a crash, start nothing early: the crash may have been in the Wi-Fi start,
    // and the message must reach the panel first.
    #[cfg(screen_server)]
    let net_early = crashed.is_none();
    #[cfg(screen_server)]
    let mut net = if !net_early {
        NetState::Starting
    } else {
        // One poll with the executor's own waker (poll_once's no-op waker panics in the
        // embassy timer queue): yield_now finishes on its second poll.
        match embassy_futures::select::select(net_start.as_mut(), embassy_futures::yield_now())
            .await
        {
            embassy_futures::select::Either::First(r) => NetState::from(r),
            embassy_futures::select::Either::Second(()) => NetState::Starting,
        }
    };
    #[cfg(not(screen_server))]
    let _ = spawner;
    // The only other wake source is the timer, which this firmware does not set.
    let cause = esp_hal::system::wakeup_cause();
    let woke_by_ok = !cause.is_empty() && !cause.contains(WakeupSource::Timer);
    info!(
        "NOTE4 firmware {} (OK wake {woke_by_ok})",
        env!("CARGO_PKG_VERSION")
    );

    let delay = Delay::new();
    let mut led = Output::new(p.GPIO3, Level::Low, OutputConfig::default()); // on
    let pull_up = InputConfig::default().with_pull(Pull::Up);
    let sampler: &'static Sampler = SAMPLER.init(Sampler::new(
        Input::new(p.GPIO39, pull_up),
        Input::new(p.GPIO18, pull_up),
        Input::new(p.GPIO0, pull_up),
    ));
    let charging_pin = Input::new(p.GPIO2, pull_up);
    let full_pin = Input::new(p.GPIO1, InputConfig::default());
    // With a server, the battery ADC is not even set up until Wi-Fi is up (or before
    // sleep): an ADC read just before Wi-Fi left the NOTE4C without a DHCP lease
    // (docs/ROADMAP.md), and with the ADC configured at boot the NOTE4 could not join.
    let mut battery = BatteryAdc::Idle(p.ADC1, p.GPIO4);
    let read_status = |battery: &mut BatteryAdc, adc: bool| Status {
        battery_mv: if adc { battery.read_mv() } else { None },
        charging: charging_pin.is_low(),
        full: full_pin.is_high(),
        woke_by_ok,
    };
    let mut status = read_status(&mut battery, !cfg!(screen_server));
    info!(
        "battery {:?} mV, charging {}, full {}",
        status.battery_mv, status.charging, status.full
    );

    // Up/Down move on press, not release: a hold (Back, Off) then follows the move.
    let button_cfg = ButtonConfig {
        move_on_press: true,
        ..ButtonConfig::default()
    };

    let spi = Spi::new(
        p.SPI3,
        SpiConfig::default()
            .with_frequency(Rate::from_mhz(20))
            .with_mode(SpiMode::_0),
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
        // Samples the buttons while the driver waits for the panel.
        ButtonDelay::new(sampler),
        Panel::Mono,
    )
    // 1 ms BUSY polling so the lab's timings are not rounded to 10 ms.
    .with_timing(Timing {
        poll_ms: 1,
        ..Timing::default()
    });

    let frame = FRAME.take();
    let shadow = SHADOW.take();
    let theme = Theme::note4(MonoTextStyle::new(&FONT_10X20, BinaryColor::On));
    let mut ui = Ui {
        nav: Nav::new(&ROOT, theme.rows_per_page()),
        view: None,
        theme,
        presenter: Presenter::new(RefreshPolicy::new(MAX_PARTIALS).with_partial_screens(true)),
        last_event: None,
        levels: [false; 3],
        bundle: BUNDLE.take(),
        bundle_len: 0,
        bundle_sel: None,
        error: Label::new(),
        crash: crashed,
    };
    if let Some(msg) = &ui.crash {
        error!("previous run panicked: {msg}");
        ui.view = Some(View::Crashed);
    }

    // Buttons held at start (OK that woke us, Down that switched us on) count only after
    // they were released.
    let mut buttons = sampler.fresh_buttons(button_cfg);

    if let Err(e) = epd.power_on() {
        error!("panel power: {e:?}");
    }
    show(&mut ui, &mut epd, frame, shadow, &status, Update::NewScreen);

    let mut session = Session::with_default(now_ms());
    let off = loop {
        sampler.drain(&mut buttons);
        ui.levels = sampler.levels();
        let t = now_ms();
        // Apply every event first and refresh once: presses made during a refresh are
        // logged by the sampler and arrive here together.
        let mut redraw: Option<Update> = None;
        let mut leave = None;
        while let Some(event) = buttons.poll(t) {
            session.touch(t);
            info!("button {event:?}");
            match ui.handle(event) {
                Effect::None => {}
                Effect::Redraw(update) => {
                    redraw = Some(match (redraw, update) {
                        (Some(Update::NewScreen), _) | (_, Update::NewScreen) => Update::NewScreen,
                        _ => Update::Change,
                    });
                }
                Effect::FullRefresh => {
                    ui.draw(frame, &status);
                    if let Err(e) = ui.presenter.full(&mut epd, frame, shadow) {
                        error!("display: {e:?}");
                    }
                    redraw = None;
                }
                Effect::Fetch(src) => {
                    ui.view = Some(View::Loading(src));
                    show(&mut ui, &mut epd, frame, shadow, &status, Update::NewScreen);
                    #[cfg(screen_server)]
                    {
                        if let NetState::Starting = net {
                            net = NetState::from(net_start.as_mut().await);
                        }
                        let result = match &mut net {
                            NetState::Up(n) => n.fetch(src.id, &mut ui.bundle[..]).await,
                            NetState::Failed(why) => Err(why.clone()),
                            NetState::Starting => unreachable!("awaited above"),
                        };
                        ui.loaded(src, result);
                    }
                    #[cfg(not(screen_server))]
                    {
                        let mut why = Label::new();
                        let _ = why.push_str("no server configured");
                        ui.loaded(src, Err(why));
                    }
                    // Presses while loading are dropped.
                    buttons = sampler.fresh_buttons(button_cfg);
                    session.touch(now_ms());
                    redraw = Some(Update::NewScreen);
                    break;
                }
                Effect::Lab => {
                    let ok = sampler.ok();
                    let mut lab_buttons = lab::Buttons::new(sampler.up(), sampler.down(), &ok);
                    lab::run(&mut epd, frame, shadow, &mut lab_buttons, &delay);
                    // The lab ends with a full refresh of its own screen, and read the
                    // pins itself.
                    ui.presenter.policy_mut().shadow_restored();
                    buttons = sampler.fresh_buttons(button_cfg);
                    session.touch(now_ms());
                    redraw = Some(Update::NewScreen);
                    break;
                }
                Effect::Sleep => leave = Some(false),
                Effect::Off => leave = Some(true),
            }
            if leave.is_some() {
                break;
            }
        }
        if let Some(off) = leave {
            break off;
        }
        if let Some(update) = redraw {
            if ui.view == Some(View::Info) {
                #[cfg(screen_server)]
                let adc = matches!(net, NetState::Up(_));
                #[cfg(not(screen_server))]
                let adc = true;
                status = read_status(&mut battery, adc);
            }
            show(&mut ui, &mut epd, frame, shadow, &status, update);
        }
        if session.should_sleep(t) {
            break false;
        }
        // Yields to the network tasks, and drives the Wi-Fi start while it runs.
        #[cfg(screen_server)]
        if net_early && let NetState::Starting = net {
            if let embassy_futures::select::Either::Second(r) = embassy_futures::select::select(
                Timer::after_millis(BUTTON_POLL_MS.into()),
                net_start.as_mut(),
            )
            .await
            {
                net = NetState::from(r);
            }
        } else {
            Timer::after_millis(BUTTON_POLL_MS.into()).await;
        }
        #[cfg(not(screen_server))]
        Timer::after_millis(BUTTON_POLL_MS.into()).await;
    };

    // Last screen: a clean full refresh, so the image stays sharp while asleep. No more
    // networking follows, so the battery can be read.
    status = read_status(&mut battery, true);
    ui.view = Some(if off { View::Off } else { View::Asleep });
    ui.draw(frame, &status);
    if let Err(e) = ui.presenter.full(&mut epd, frame, shadow) {
        error!("display: {e:?}");
    }
    let _ = epd.power_off();
    let (_spi, _cs, _dc, _rst, _busy, mut rail, _delay) = epd.release();
    power::hold_through_sleep(&mut rail);
    led.set_high(); // off

    if off {
        info!("switching off");
        // Wait for Down to be released, or it would switch the device on again.
        while sampler.down().is_low() {
            delay.delay_millis(BUTTON_POLL_MS);
        }
        latch.release();
        // Still running: USB power keeps the board on. Sleep instead.
        delay.delay_millis(500);
        warn!("still powered (USB?), sleeping instead");
    }
    info!("sleeping; OK wakes");
    power::deep_sleep(p.LPWR, &mut sampler.ok_mut(), None)
}
