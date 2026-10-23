//! NOTE4 (black/white) firmware: an on-device menu driven by the three buttons.
//!
//! * Reset or OK wakes it into the menu ([`zectrix_screen::menu`]). Up/Down move, OK opens
//!   a submenu or runs the item, every submenu has a Back row on top, Up held goes back.
//! * Each change is one partial refresh of whatever changed ([`zectrix_screen::Presenter`];
//!   ~0.8 s on the NOTE4), with a full refresh after 16 partials.
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

mod lab;

use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_10X20},
    pixelcolor::BinaryColor,
};
use esp_backtrace as _;
use esp_hal::{
    delay::Delay,
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull},
    main,
    rtc_cntl::WakeupSource,
    spi::{
        Mode as SpiMode,
        master::{Config as SpiConfig, Spi},
    },
    time::{Duration, Instant, Rate},
};
use log::{error, info, warn};
use ssd2683::{MonoFrame, Panel, Ssd2683, Timing};
use static_cell::ConstStaticCell;
use zectrix_board::power::{self, Battery, PowerLatch};
use zectrix_screen::{
    Button, Buttons, Event, Presenter, RefreshPolicy, Session, Shown, Update,
    menu::{Item, Menu, Nav, Outcome},
    view::{self, Label, Theme},
};

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

/// Partial refreshes before a full one. Ghosting was hard to see after many more.
const MAX_PARTIALS: u16 = 16;
const BUTTON_POLL_MS: u32 = 10;

/// What a menu item does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Open(View),
    Lab,
    FullRefresh,
    Sleep,
}

/// A screen other than the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Info,
    ButtonTest,
    /// Not built yet; the label names it.
    Soon(&'static str),
    Asleep,
    Off,
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
        Item::action("Lights", Action::Open(View::Soon("Lights"))),
        Item::action("Calendar", Action::Open(View::Soon("Calendar"))),
        Item::action("Weather", Action::Open(View::Soon("Weather"))),
        Item::action("Last answer", Action::Open(View::Soon("Last answer"))),
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
}

impl Ui {
    fn draw(&self, frame: &mut MonoFrame, status: &Status) {
        let t = &self.theme;
        let Some(view) = self.view else {
            let partials = self.presenter.policy().partials();
            let _ = view::draw_menu(frame, t, &self.nav, |a, out| match a {
                Action::FullRefresh => write!(out, "{partials}/{MAX_PARTIALS}"),
                Action::Open(View::Soon(_)) => out.write_str("soon"),
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
            View::Soon(name) => {
                let _ = lines[0].push_str(name);
                let _ = lines[1].push_str("comes from the server;");
                let _ = lines[2].push_str("not built yet.");
                "Coming soon"
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
        let back = matches!(view, View::Info | View::ButtonTest | View::Soon(_));
        let _ = view::draw_title(frame, t, title, back.then_some("OK: back"));
        let _ = view::draw_lines(frame, t, lines.iter().map(|l| l.as_str()));
    }
}

/// What the main loop does after an event.
enum Effect {
    None,
    Redraw(Update),
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

fn now_ms() -> u64 {
    Instant::now().duration_since_epoch().as_millis()
}

#[main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default());
    // First: keep ourselves powered on battery.
    let latch = PowerLatch::engage(p.GPIO17);

    esp_println::logger::init_logger_from_env();
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
    let mut ok = Input::new(p.GPIO0, pull_up);
    let down = Input::new(p.GPIO18, pull_up);
    let up = Input::new(p.GPIO39, pull_up);
    let charging_pin = Input::new(p.GPIO2, pull_up);
    let full_pin = Input::new(p.GPIO1, InputConfig::default());
    let mut battery = Battery::new(p.ADC1, p.GPIO4);
    let read_status = |battery: &mut Battery| Status {
        battery_mv: battery.read_mv(),
        charging: charging_pin.is_low(),
        full: full_pin.is_high(),
        woke_by_ok,
    };
    let mut status = read_status(&mut battery);
    info!(
        "battery {:?} mV, charging {}, full {}",
        status.battery_mv, status.charging, status.full
    );

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
        delay,
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
    };

    let mut buttons = Buttons::default();
    // Buttons held at start (OK that woke us, Down that switched us on) count only after
    // they were released.
    for (b, pin) in [(Button::Ok, &ok), (Button::Down, &down), (Button::Up, &up)] {
        if pin.is_low() {
            buttons.ignore_until_release(b, now_ms());
        }
    }

    if let Err(e) = epd.power_on() {
        error!("panel power: {e:?}");
    }
    show(&mut ui, &mut epd, frame, shadow, &status, Update::NewScreen);

    let mut session = Session::with_default(now_ms());
    let off = loop {
        let t = now_ms();
        ui.levels = [up.is_low(), down.is_low(), ok.is_low()];
        buttons.input(Button::Up, ui.levels[0], t);
        buttons.input(Button::Down, ui.levels[1], t);
        buttons.input(Button::Ok, ui.levels[2], t);
        let mut leave = None;
        while let Some(event) = buttons.poll(t) {
            session.touch(t);
            info!("button {event:?}");
            match ui.handle(event) {
                Effect::None => {}
                Effect::Redraw(update) => {
                    if ui.view == Some(View::Info) {
                        status = read_status(&mut battery);
                    }
                    show(&mut ui, &mut epd, frame, shadow, &status, update);
                }
                Effect::FullRefresh => {
                    ui.draw(frame, &status);
                    if let Err(e) = ui.presenter.full(&mut epd, frame, shadow) {
                        error!("display: {e:?}");
                    }
                }
                Effect::Lab => {
                    let mut lab_buttons = lab::Buttons::new(&up, &down, &ok);
                    lab::run(&mut epd, frame, shadow, &mut lab_buttons, &delay);
                    // The lab ends with a full refresh of its own screen.
                    ui.presenter.policy_mut().shadow_restored();
                    buttons.ignore_until_release(Button::Ok, now_ms());
                    session.touch(now_ms());
                    show(&mut ui, &mut epd, frame, shadow, &status, Update::NewScreen);
                }
                Effect::Sleep => leave = Some(false),
                Effect::Off => leave = Some(true),
            }
        }
        if let Some(off) = leave {
            break off;
        }
        if session.should_sleep(t) {
            break false;
        }
        delay.delay_millis(BUTTON_POLL_MS);
    };

    // Last screen: a clean full refresh, so the image stays sharp while asleep.
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
        while down.is_low() {
            delay.delay_millis(BUTTON_POLL_MS);
        }
        latch.release();
        // Still running: USB power keeps the board on. Sleep instead.
        delay.delay_millis(500);
        warn!("still powered (USB?), sleeping instead");
    }
    info!("sleeping; OK wakes");
    power::deep_sleep(p.LPWR, &mut ok, None)
}
