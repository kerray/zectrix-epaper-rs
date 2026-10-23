//! Button sampling that keeps going while the panel refreshes.
//!
//! A refresh blocks for ~0.8 s, and presses during it used to be lost. The panel driver
//! waits for BUSY by calling its delay in 1 ms steps, so [`ButtonDelay`] (the driver's
//! delay) samples the three pins on every call and logs each change with its time. The
//! main loop hands the log to [`zectrix_screen::Buttons`], which takes timestamped edges,
//! so hold times stay right even for presses that happened mid-refresh.

use core::cell::{Cell, Ref, RefCell, RefMut};

use embedded_hal::delay::DelayNs;
use esp_hal::{delay::Delay, gpio::Input, time::Instant};
use heapless::Deque;
use zectrix_screen::{Button, ButtonConfig, Buttons};

/// Pin changes between two drains. A refresh with more than this (heavy bouncing) drops
/// the rest; the next [`Sampler::sample`] then logs the level as it is.
const LOG: usize = 32;

const BUTTONS: [Button; 3] = [Button::Up, Button::Down, Button::Ok];

pub fn now_ms() -> u64 {
    Instant::now().duration_since_epoch().as_millis()
}

/// The Up, Down and OK pins (active low) and the changes seen since the last drain.
///
/// Meant to live in a `static`: the panel driver's [`ButtonDelay`] and the main loop both
/// use it. OK sits in a `RefCell` because deep sleep needs it mutably at the very end.
pub struct Sampler {
    up: Input<'static>,
    down: Input<'static>,
    ok: RefCell<Input<'static>>,
    /// Last logged level per pin, true = pressed.
    last: Cell<[bool; 3]>,
    log: RefCell<Deque<(Button, bool, u64), LOG>>,
}

impl Sampler {
    pub fn new(up: Input<'static>, down: Input<'static>, ok: Input<'static>) -> Self {
        let last = [up.is_low(), down.is_low(), ok.is_low()];
        Self {
            up,
            down,
            ok: RefCell::new(ok),
            last: Cell::new(last),
            log: RefCell::new(Deque::new()),
        }
    }

    pub fn up(&self) -> &Input<'static> {
        &self.up
    }

    pub fn down(&self) -> &Input<'static> {
        &self.down
    }

    pub fn ok(&self) -> Ref<'_, Input<'static>> {
        self.ok.borrow()
    }

    /// For deep sleep (wake on OK). Do not sample while this is held.
    pub fn ok_mut(&self) -> RefMut<'_, Input<'static>> {
        self.ok.borrow_mut()
    }

    /// Current levels, true = pressed (Up, Down, OK).
    pub fn levels(&self) -> [bool; 3] {
        [
            self.up.is_low(),
            self.down.is_low(),
            self.ok.borrow().is_low(),
        ]
    }

    /// Read the pins and log the ones that changed.
    pub fn sample(&self) {
        let now = self.levels();
        let mut last = self.last.get();
        if now == last {
            return;
        }
        let t = now_ms();
        let mut log = self.log.borrow_mut();
        for i in 0..3 {
            if now[i] != last[i] && log.push_back((BUTTONS[i], now[i], t)).is_ok() {
                last[i] = now[i];
            }
        }
        self.last.set(last);
    }

    /// Forget logged changes (e.g. made while the lab read the pins itself) and start
    /// from the current levels.
    pub fn reset(&self) {
        self.log.borrow_mut().clear();
        self.last.set(self.levels());
    }

    /// Fresh button state for `cfg`: buttons held right now count only after they were
    /// released (OK that woke the chip, Down that switched it on, a press that left the
    /// lab).
    pub fn fresh_buttons(&self, cfg: ButtonConfig) -> Buttons {
        self.reset();
        let mut buttons = Buttons::new(cfg);
        let t = now_ms();
        for (i, b) in BUTTONS.into_iter().enumerate() {
            if self.last.get()[i] {
                buttons.ignore_until_release(b, t);
            }
        }
        buttons
    }

    /// Sample once more, then feed every logged change to `buttons` in order.
    pub fn drain(&self, buttons: &mut Buttons) {
        self.sample();
        let mut log = self.log.borrow_mut();
        while let Some((b, pressed, t)) = log.pop_front() {
            buttons.input(b, pressed, t);
        }
        // Repeat the levels at the current time, so debounce and hold timers advance.
        let t = now_ms();
        for (i, b) in BUTTONS.into_iter().enumerate() {
            buttons.input(b, self.last.get()[i], t);
        }
    }
}

/// The panel driver's delay: sleeps like [`Delay`], sampling the buttons every
/// millisecond.
pub struct ButtonDelay {
    inner: Delay,
    sampler: &'static Sampler,
}

impl ButtonDelay {
    pub fn new(sampler: &'static Sampler) -> Self {
        Self {
            inner: Delay::new(),
            sampler,
        }
    }
}

impl DelayNs for ButtonDelay {
    fn delay_ns(&mut self, mut ns: u32) {
        const STEP: u32 = 1_000_000;
        loop {
            self.sampler.sample();
            let step = ns.min(STEP);
            self.inner.delay_ns(step);
            ns -= step;
            if ns == 0 {
                break;
            }
        }
    }
}
