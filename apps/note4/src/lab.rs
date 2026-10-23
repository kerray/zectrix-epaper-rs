//! Partial-refresh lab: measures refresh times and shows them on the panel itself
//! (USB logs over WSL are unreliable). See docs/INTERACTIVE.md "Lab screen".
//!
//! Six 40 px rows form a menu, a 60 px status band sits below them.
//!
//! * On entry it runs the benchmark: full refresh, one 400×40 row, two far-apart rows as
//!   two calls, the same two rows as one window, the whole screen as a partial. Each
//!   result appears in its row.
//! * **Up / Down** move the highlight with one partial refresh of a window that holds
//!   the old row, the new row and the status band. A partial refresh costs about the
//!   same at any size (~0.77 s measured), so one big window beats several small ones.
//!   Row 6 shows its time.
//! * **OK** switches the highlight between an inverted row and a thick border, and does
//!   a full refresh.
//! * The status band shows the last refresh and the number of partial refreshes since
//!   the last full one. There is no automatic full refresh, so ghosting can be judged
//!   after 8, 16, 32 … partials.
//! * **OK held 1 s** leaves the lab, as do 60 s without a press. Either way the screen gets
//!   one full refresh first, so the shadow matches a clean panel when the lab returns.

use core::fmt::Write as _;

use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_10X20},
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::Text,
};
use esp_hal::{
    Blocking,
    delay::Delay,
    gpio::{Input, Output},
    spi::master::Spi,
    time::{Duration, Instant},
};
use log::{error, info};
use ssd2683::{MonoFrame, Rect, Ssd2683};

pub type Epd<'d> =
    Ssd2683<Spi<'d, Blocking>, Output<'d>, Output<'d>, Output<'d>, Input<'d>, Output<'d>, Delay>;

const ROWS: usize = 6;
const ROW_H: u16 = 40;
const STATUS: Rect = Rect::new(0, ROWS as u16 * ROW_H, 400, 300 - ROWS as u16 * ROW_H);
const IDLE: Duration = Duration::from_secs(60);
const LONG_PRESS: Duration = Duration::from_millis(1000);

const LABELS: [&str; ROWS] = [
    "Full refresh",
    "Partial 400x40",
    "2 rows, 2 calls",
    "2 rows, 1 window",
    "Full-scr partial",
    "Cursor move",
];

const fn row_rect(i: usize) -> Rect {
    Rect::new(0, i as u16 * ROW_H, 400, ROW_H)
}

/// Debounced buttons, all active low.
pub struct Buttons<'a, 'd> {
    up: &'a Input<'d>,
    down: &'a Input<'d>,
    ok: &'a Input<'d>,
    /// Consecutive low polls of up / down.
    low: [u8; 2],
    ok_since: Option<Instant>,
    ok_long_sent: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Up,
    Down,
    Ok,
    OkLong,
}

impl<'a, 'd> Buttons<'a, 'd> {
    pub fn new(up: &'a Input<'d>, down: &'a Input<'d>, ok: &'a Input<'d>) -> Self {
        // A button that is already down (e.g. OK that woke the chip) only counts after
        // it was released.
        Self {
            up,
            down,
            ok,
            low: [u8::MAX; 2],
            ok_since: None,
            ok_long_sent: ok.is_low(),
        }
    }

    /// Call every ~10 ms.
    pub fn poll(&mut self) -> Option<Press> {
        let mut out = None;
        for (i, (pin, press)) in [(self.up, Press::Up), (self.down, Press::Down)]
            .into_iter()
            .enumerate()
        {
            if pin.is_low() {
                self.low[i] = self.low[i].saturating_add(1);
                if self.low[i] == 2 {
                    out = Some(press);
                }
            } else {
                self.low[i] = 0;
            }
        }
        if self.ok.is_low() {
            let since = *self.ok_since.get_or_insert_with(Instant::now);
            if !self.ok_long_sent && since.elapsed() >= LONG_PRESS {
                self.ok_long_sent = true;
                out = Some(Press::OkLong);
            }
        } else {
            if let Some(since) = self.ok_since.take()
                && !self.ok_long_sent
                && since.elapsed() >= Duration::from_millis(20)
            {
                out = Some(Press::Ok);
            }
            self.ok_long_sent = false;
        }
        out
    }
}

/// Wait up to `timeout` for a press.
pub fn wait_press(buttons: &mut Buttons, delay: &Delay, timeout: Duration) -> Option<Press> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Some(p) = buttons.poll() {
            return Some(p);
        }
        delay.delay_millis(10);
    }
    None
}

#[derive(Clone, Copy)]
enum Last {
    None,
    Full(u32),
    Partial(&'static str, u32),
    Failed(&'static str),
}

struct Lab {
    sel: usize,
    values: [Option<u32>; ROWS],
    last: Last,
    /// Partial refreshes since the last full refresh.
    partials: u32,
    /// Highlight with a thick border instead of inverting the row.
    border: bool,
    /// `shadow` matches the panel. False after any failed refresh.
    shadow_ok: bool,
}

/// Run the lab until 60 s pass without a press. The panel must be powered.
pub fn run(
    epd: &mut Epd,
    frame: &mut MonoFrame,
    shadow: &mut MonoFrame,
    buttons: &mut Buttons,
    delay: &Delay,
) {
    let mut lab = Lab {
        sel: 0,
        values: [None; ROWS],
        last: Last::None,
        partials: 0,
        border: false,
        shadow_ok: false,
    };
    lab.bench(epd, frame, shadow);
    loop {
        match wait_press(buttons, delay, IDLE) {
            Some(Press::Up) => lab.move_cursor(epd, frame, shadow, ROWS - 1),
            Some(Press::Down) => lab.move_cursor(epd, frame, shadow, 1),
            Some(Press::Ok) => {
                lab.border = !lab.border;
                lab.draw_all(frame, false);
                if let Some(ms) = lab.full(epd, frame, shadow) {
                    lab.values[0] = Some(ms);
                }
                // Show the new full-refresh time.
                lab.draw_all(frame, false);
                lab.partial(epd, frame, shadow, &[Rect::full()], "results");
            }
            Some(Press::OkLong) | None => break,
        }
    }
    info!("lab: leaving, final full refresh");
    lab.draw_all(frame, true);
    lab.full(epd, frame, shadow);
}

impl Lab {
    fn bench(&mut self, epd: &mut Epd, frame: &mut MonoFrame, shadow: &mut MonoFrame) {
        info!("lab: benchmark");
        self.sel = 0;
        self.draw_all(frame, false);
        self.values[0] = self.full(epd, frame, shadow);

        // One 400×40 row.
        self.draw_row(frame, 2, true);
        self.values[1] = self.partial(epd, frame, shadow, &[row_rect(2)], "1 row");

        // Rows 0 and 5 (160 px apart) as two calls …
        self.draw_row(frame, 0, false);
        self.draw_row(frame, 5, true);
        self.values[2] = self.partial(epd, frame, shadow, &[row_rect(0), row_rect(5)], "2 calls");

        // … and back as one 400×240 window.
        self.draw_row(frame, 0, true);
        self.draw_row(frame, 5, false);
        let both = row_rect(0).union(&row_rect(5));
        self.values[3] = self.partial(epd, frame, shadow, &[both], "1 window");

        // The whole screen as a partial, showing the results so far.
        self.draw_all(frame, false);
        self.values[4] = self.partial(epd, frame, shadow, &[Rect::full()], "full scr");

        // Show the full-screen time: row 4 down to the status band in one window.
        self.draw_row(frame, 4, self.sel == 4);
        self.draw_status(frame, None);
        let rect = row_rect(4).union(&STATUS);
        self.partial(epd, frame, shadow, &[rect], "results");
    }

    fn move_cursor(
        &mut self,
        epd: &mut Epd,
        frame: &mut MonoFrame,
        shadow: &mut MonoFrame,
        step: usize,
    ) {
        let old = self.sel;
        self.sel = (self.sel + step) % ROWS;
        self.draw_row(frame, old, false);
        self.draw_row(frame, self.sel, true);
        // The status band shows the previous move's time and the partial count after
        // this one, so it can go in the same window.
        self.partials += 1;
        self.draw_status(frame, None);
        self.partials -= 1;
        let rect = row_rect(old.min(self.sel)).union(&STATUS);
        if let Some(ms) = self.partial(epd, frame, shadow, &[rect], "cursor") {
            self.values[5] = Some(ms);
        }
    }

    /// Full refresh of `frame`; returns its time in ms.
    fn full(
        &mut self,
        epd: &mut Epd,
        frame: &mut MonoFrame,
        shadow: &mut MonoFrame,
    ) -> Option<u32> {
        let t = Instant::now();
        let r = epd.show_mono(frame, None);
        let ms = t.elapsed().as_millis() as u32;
        match r {
            Ok(()) => {
                info!("lab: full {ms} ms");
                shadow.copy_from(frame);
                self.shadow_ok = true;
                self.partials = 0;
                self.last = Last::Full(ms);
                Some(ms)
            }
            Err(e) => {
                error!("lab: full refresh failed: {e:?}");
                self.shadow_ok = false;
                self.last = Last::Failed("full");
                None
            }
        }
    }

    /// Partial refreshes of `rects`, one call each; returns the total time in ms.
    /// Without a valid shadow it does a full refresh instead and returns `None`.
    fn partial(
        &mut self,
        epd: &mut Epd,
        frame: &mut MonoFrame,
        shadow: &mut MonoFrame,
        rects: &[Rect],
        what: &'static str,
    ) -> Option<u32> {
        if !self.shadow_ok {
            self.full(epd, frame, shadow);
            return None;
        }
        let t = Instant::now();
        for &rect in rects {
            if let Err(e) = epd.show_mono_partial(shadow, frame, rect) {
                error!("lab: partial {what} failed: {e:?}");
                self.shadow_ok = false;
                self.last = Last::Failed(what);
                return None;
            }
            self.partials += 1;
        }
        let ms = t.elapsed().as_millis() as u32;
        info!("lab: partial {what} {ms} ms");
        self.last = Last::Partial(what, ms);
        Some(ms)
    }

    fn draw_all(&self, frame: &mut MonoFrame, leaving: bool) {
        let _ = frame.clear(BinaryColor::Off);
        for i in 0..ROWS {
            self.draw_row(frame, i, i == self.sel);
        }
        self.draw_status(frame, leaving.then_some("Back to the menu."));
    }

    fn draw_row(&self, frame: &mut MonoFrame, i: usize, selected: bool) {
        let inverted = selected && !self.border;
        let (bg, fg) = if inverted {
            (BinaryColor::On, BinaryColor::Off)
        } else {
            (BinaryColor::Off, BinaryColor::On)
        };
        let y = i as i32 * ROW_H as i32;
        let _ = Rectangle::new(Point::new(0, y), Size::new(400, ROW_H as u32))
            .into_styled(PrimitiveStyle::with_fill(bg))
            .draw(frame);
        if selected && self.border {
            let _ = Rectangle::new(Point::new(2, y + 2), Size::new(396, ROW_H as u32 - 4))
                .into_styled(PrimitiveStyle::with_stroke(fg, 4))
                .draw(frame);
        } else if !inverted {
            // Thin separator, so the row grid is visible.
            let _ = Rectangle::new(Point::new(0, y + ROW_H as i32 - 1), Size::new(400, 1))
                .into_styled(PrimitiveStyle::with_fill(fg))
                .draw(frame);
        }
        let style = MonoTextStyle::new(&FONT_10X20, fg);
        let _ = Text::new(LABELS[i], Point::new(10, y + 26), style).draw(frame);
        let mut v: heapless::String<16> = heapless::String::new();
        match self.values[i] {
            Some(ms) => {
                let _ = write!(v, "{ms:>5} ms");
            }
            None => {
                let _ = v.push_str("   -- ms");
            }
        }
        let _ = Text::new(&v, Point::new(300, y + 26), style).draw(frame);
    }

    fn draw_status(&self, frame: &mut MonoFrame, override_line1: Option<&str>) {
        let top = STATUS.y as i32;
        let _ = Rectangle::new(
            Point::new(0, top),
            Size::new(STATUS.w as u32, STATUS.h as u32),
        )
        .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
        .draw(frame);
        let ink = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);
        let mut l1: heapless::String<48> = heapless::String::new();
        match (override_line1, self.last) {
            (Some(s), _) => {
                let _ = l1.push_str(s);
            }
            (None, Last::None) => {
                let _ = l1.push_str("Last: -");
            }
            (None, Last::Full(ms)) => {
                let _ = write!(l1, "Last: full {ms} ms");
            }
            (None, Last::Partial(what, ms)) => {
                let _ = write!(l1, "Last: {what} {ms} ms");
            }
            (None, Last::Failed(what)) => {
                let _ = write!(l1, "FAILED: {what}");
            }
        }
        let mut l2: heapless::String<48> = heapless::String::new();
        let _ = write!(l2, "Partials: {}", self.partials);
        let _ = l2.push_str(if self.border {
            "  border"
        } else {
            "  inverted"
        });
        let _ = Text::new(&l1, Point::new(10, top + 25), ink).draw(frame);
        let _ = Text::new(&l2, Point::new(10, top + 50), ink).draw(frame);
    }
}
