//! Up, Down and OK: debounce, short and held presses (`docs/INTERACTIVE.md`, "Buttons").
//!
//! | Input | Event |
//! | --- | --- |
//! | Up / Down released within `short_max_ms` (1 s) | [`Event::Up`] / [`Event::Down`] |
//! | OK released within `talk_ms` (0.4 s) | [`Event::Select`] |
//! | OK held `talk_ms` | [`Event::TalkStart`], then [`Event::TalkEnd`] on release |
//! | Up held `back_ms` (1 s) | [`Event::Back`] |
//! | Down held `off_ms` (3 s) | [`Event::Off`] |
//!
//! A Down press released between `short_max_ms` and `off_ms` does nothing. With
//! [`ButtonConfig::move_on_press`], Up/Down fire as soon as the press is debounced
//! (faster, but a held Up or Down then also moves the cursor before Back/Off).
//!
//! The machine is pure: feed raw levels with [`Buttons::input`] (on every poll, or on each
//! edge interrupt), then call [`Buttons::poll`] until it returns `None`. A level change
//! counts once it has been stable for `debounce_ms`; the press is timed from the raw edge,
//! so the hold thresholds are not shifted by the debounce. [`Buttons::next_deadline`] says
//! when to poll again without a new edge. Times are milliseconds from a monotonic clock.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Up,
    Down,
    Ok,
}

impl Button {
    const ALL: [Button; 3] = [Button::Ok, Button::Up, Button::Down];

    fn index(self) -> usize {
        match self {
            Button::Ok => 0,
            Button::Up => 1,
            Button::Down => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Move the selection up.
    Up,
    /// Move the selection down.
    Down,
    /// OK short: act on the selected row.
    Select,
    /// OK held: start push-to-talk.
    TalkStart,
    /// OK released after [`TalkStart`](Event::TalkStart).
    TalkEnd,
    /// Up held: back to the screen list.
    Back,
    /// Down held: switch off.
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ButtonConfig {
    pub debounce_ms: u64,
    pub short_max_ms: u64,
    pub talk_ms: u64,
    pub back_ms: u64,
    pub off_ms: u64,
    pub move_on_press: bool,
}

impl Default for ButtonConfig {
    fn default() -> Self {
        Self {
            debounce_ms: 30,
            short_max_ms: 1_000,
            talk_ms: 400,
            back_ms: 1_000,
            off_ms: 3_000,
            move_on_press: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Key {
    /// Last raw level (true = pressed) and when it changed.
    raw: bool,
    raw_since: u64,
    /// Debounced level and when the press started (raw edge).
    down: bool,
    down_since: u64,
    /// The hold event (TalkStart / Back / Off) was sent for this press.
    held_sent: bool,
    /// Ignore this press entirely (it was already down at start).
    ignore: bool,
}

/// Events found while handling an edge, waiting for [`Buttons::poll`]. One edge can
/// settle at most three (press, hold, release).
const QUEUE: usize = 8;

#[derive(Clone, Debug)]
pub struct Buttons {
    cfg: ButtonConfig,
    keys: [Key; 3],
    queue: [Option<Event>; QUEUE],
}

impl Default for Buttons {
    fn default() -> Self {
        Self::new(ButtonConfig::default())
    }
}

impl Buttons {
    pub fn new(cfg: ButtonConfig) -> Self {
        Self {
            cfg,
            keys: [Key::default(); 3],
            queue: [None; QUEUE],
        }
    }

    pub fn config(&self) -> &ButtonConfig {
        &self.cfg
    }

    /// `button` is already down at `now` (e.g. OK that woke the chip): this press gives no
    /// events; the button counts again after it was released.
    pub fn ignore_until_release(&mut self, button: Button, now: u64) {
        self.keys[button.index()] = Key {
            raw: true,
            raw_since: now,
            down: true,
            down_since: now,
            held_sent: true,
            ignore: true,
        };
    }

    /// The raw level of `button` at `now` (true = pressed). Repeating the same level is
    /// fine. On a change, whatever the previous level settled into up to `now` is handled
    /// first (its events wait for [`poll`](Self::poll)), so an edge-driven caller that
    /// does not poll between two edges loses nothing.
    pub fn input(&mut self, button: Button, pressed: bool, now: u64) {
        if self.keys[button.index()].raw != pressed {
            while let Some(e) = self.poll_key(button, now) {
                if let Some(slot) = self.queue.iter_mut().find(|s| s.is_none()) {
                    *slot = Some(e);
                }
            }
            let k = &mut self.keys[button.index()];
            k.raw = pressed;
            k.raw_since = now;
        }
    }

    /// Debounced state of `button`.
    pub fn is_down(&self, button: Button) -> bool {
        self.keys[button.index()].down
    }

    /// The next event at `now`, if any. Call until `None`.
    pub fn poll(&mut self, now: u64) -> Option<Event> {
        if let Some(e) = self.queue[0].take() {
            self.queue.rotate_left(1);
            return Some(e);
        }
        Button::ALL.into_iter().find_map(|b| self.poll_key(b, now))
    }

    fn hold_ms(&self, b: Button) -> u64 {
        match b {
            Button::Ok => self.cfg.talk_ms,
            Button::Up => self.cfg.back_ms,
            Button::Down => self.cfg.off_ms,
        }
    }

    fn poll_key(&mut self, b: Button, now: u64) -> Option<Event> {
        let cfg = self.cfg;
        let hold = self.hold_ms(b);
        let k = &mut self.keys[b.index()];
        // 1. A raw change that has been stable long enough.
        if k.raw != k.down && now.saturating_sub(k.raw_since) >= cfg.debounce_ms {
            k.down = k.raw;
            if k.down {
                k.down_since = k.raw_since;
                k.held_sent = false;
                k.ignore = false;
                if cfg.move_on_press {
                    match b {
                        Button::Up => return Some(Event::Up),
                        Button::Down => return Some(Event::Down),
                        Button::Ok => {}
                    }
                }
            } else {
                if core::mem::take(&mut k.ignore) {
                    return None;
                }
                let held = k.raw_since.saturating_sub(k.down_since);
                return match b {
                    Button::Ok if k.held_sent => Some(Event::TalkEnd),
                    Button::Ok => Some(Event::Select),
                    _ if k.held_sent || cfg.move_on_press || held >= cfg.short_max_ms => None,
                    Button::Up => Some(Event::Up),
                    Button::Down => Some(Event::Down),
                };
            }
        }
        // 2. A hold threshold, measured up to the raw release if one is pending.
        if k.down && !k.held_sent && !k.ignore {
            let end = if k.raw { now } else { k.raw_since };
            if end.saturating_sub(k.down_since) >= hold {
                k.held_sent = true;
                return Some(match b {
                    Button::Ok => Event::TalkStart,
                    Button::Up => Event::Back,
                    Button::Down => Event::Off,
                });
            }
        }
        None
    }

    /// The earliest time at which [`poll`](Self::poll) can return something without a new
    /// [`input`](Self::input): a pending debounce or a hold threshold. `None` when idle.
    pub fn next_deadline(&self) -> Option<u64> {
        Button::ALL
            .into_iter()
            .filter_map(|b| {
                let k = &self.keys[b.index()];
                if k.raw != k.down {
                    Some(k.raw_since.saturating_add(self.cfg.debounce_ms))
                } else if k.down && !k.held_sent && !k.ignore {
                    Some(k.down_since.saturating_add(self.hold_ms(b)))
                } else {
                    None
                }
            })
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Press `b` at `t0`, release at `t1`, polling every 10 ms up to `t1 + 100`.
    fn tap(cfg: ButtonConfig, b: Button, t0: u64, t1: u64) -> Vec<(u64, Event)> {
        let mut bt = Buttons::new(cfg);
        let mut out = Vec::new();
        let mut t = 0;
        while t <= t1 + 100 {
            bt.input(b, (t0..t1).contains(&t), t);
            while let Some(e) = bt.poll(t) {
                out.push((t, e));
            }
            t += 10;
        }
        out
    }

    #[test]
    fn short_presses() {
        let c = ButtonConfig::default();
        assert_eq!(tap(c, Button::Up, 100, 200), [(230, Event::Up)]);
        assert_eq!(tap(c, Button::Down, 100, 200), [(230, Event::Down)]);
        assert_eq!(tap(c, Button::Ok, 100, 300), [(330, Event::Select)]);
    }

    #[test]
    fn held_presses() {
        let c = ButtonConfig::default();
        assert_eq!(
            tap(c, Button::Ok, 100, 1000),
            [(500, Event::TalkStart), (1030, Event::TalkEnd)]
        );
        assert_eq!(tap(c, Button::Up, 100, 1500), [(1100, Event::Back)]);
        assert_eq!(tap(c, Button::Down, 100, 3500), [(3100, Event::Off)]);
        // Down between short and off: nothing.
        assert!(tap(c, Button::Down, 100, 2000).is_empty());
    }

    #[test]
    fn move_on_press() {
        let c = ButtonConfig {
            move_on_press: true,
            ..Default::default()
        };
        assert_eq!(tap(c, Button::Up, 100, 200), [(130, Event::Up)]);
        assert_eq!(
            tap(c, Button::Up, 100, 1500),
            [(130, Event::Up), (1100, Event::Back)]
        );
        assert_eq!(tap(c, Button::Ok, 100, 200), [(230, Event::Select)]);
    }

    #[test]
    fn bounce_is_ignored() {
        let mut b = Buttons::default();
        // Contact bounce on press and release, edges only.
        for (t, lvl) in [(0, true), (3, false), (5, true), (8, false), (9, true)] {
            b.input(Button::Up, lvl, t);
            assert_eq!(b.poll(t), None);
        }
        assert_eq!(b.poll(30), None);
        assert_eq!(b.next_deadline(), Some(39));
        assert_eq!(b.poll(39), None);
        assert!(b.is_down(Button::Up));
        for (t, lvl) in [(100, false), (102, true), (104, false)] {
            b.input(Button::Up, lvl, t);
            assert_eq!(b.poll(t), None);
        }
        assert_eq!(b.poll(133), None);
        assert_eq!(b.poll(134), Some(Event::Up));
        assert_eq!(b.next_deadline(), None);
    }

    #[test]
    fn glitch_shorter_than_debounce_gives_nothing() {
        let mut b = Buttons::default();
        b.input(Button::Ok, true, 0);
        b.input(Button::Ok, false, 20);
        for t in (20..200).step_by(10) {
            assert_eq!(b.poll(t), None);
        }
    }

    #[test]
    fn talk_threshold_uses_raw_release() {
        // Released at 390 ms (raw), polled late at 450 ms: Select, not TalkStart.
        let mut b = Buttons::default();
        b.input(Button::Ok, true, 0);
        assert_eq!(b.poll(30), None);
        b.input(Button::Ok, false, 390);
        assert_eq!(b.poll(450), Some(Event::Select));
        assert_eq!(b.poll(450), None);
    }

    #[test]
    fn hold_deadline() {
        let mut b = Buttons::default();
        b.input(Button::Down, true, 1000);
        assert_eq!(b.next_deadline(), Some(1030));
        assert_eq!(b.poll(1030), None);
        assert_eq!(b.next_deadline(), Some(4000));
        assert_eq!(b.poll(4000), Some(Event::Off));
        assert_eq!(b.next_deadline(), None);
    }

    #[test]
    fn wake_button_is_ignored_until_release() {
        let mut b = Buttons::default();
        b.ignore_until_release(Button::Ok, 0);
        assert_eq!(b.next_deadline(), None);
        assert_eq!(b.poll(2000), None);
        b.input(Button::Ok, false, 2000);
        assert_eq!(b.poll(2100), None);
        b.input(Button::Ok, true, 3000);
        b.input(Button::Ok, false, 3100);
        assert_eq!(b.poll(3200), Some(Event::Select));
    }

    #[test]
    fn edges_without_polls_in_between() {
        // An edge-driven caller: press, hold past the talk threshold, release, then one
        // poll. Nothing is lost and the order is kept.
        let mut b = Buttons::default();
        b.input(Button::Ok, true, 0);
        b.input(Button::Ok, false, 600);
        b.input(Button::Down, true, 700);
        b.input(Button::Down, false, 800);
        assert_eq!(b.poll(900), Some(Event::TalkStart));
        assert_eq!(b.poll(900), Some(Event::TalkEnd));
        assert_eq!(b.poll(900), Some(Event::Down));
        assert_eq!(b.poll(900), None);
    }

    #[test]
    fn two_buttons_at_once() {
        let mut b = Buttons::default();
        b.input(Button::Ok, true, 0);
        b.input(Button::Up, true, 0);
        // Both settle and pass their hold threshold in the same poll.
        assert_eq!(b.poll(1000), Some(Event::TalkStart));
        assert_eq!(b.poll(1000), Some(Event::Back));
        assert_eq!(b.poll(1000), None);
    }
}
