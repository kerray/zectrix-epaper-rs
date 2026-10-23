//! Getting a frame onto the panel: one partial refresh of whatever changed, or a full
//! refresh when the [`RefreshPolicy`] asks for one.
//!
//! On the NOTE4 a partial refresh costs about the same at any size (one 400×40 row 764 ms,
//! the whole screen 793 ms; `docs/INTERACTIVE.md`, "Measurements"). So [`Presenter::show`]
//! compares the new frame with the shadow (what the panel shows) and refreshes the one
//! rectangle that holds every change, never several small ones. The app just draws the
//! complete screen into its frame.

use ssd2683::{MonoFrame, Rect, Ssd2683};

use crate::policy::{RefreshKind, RefreshPolicy, Update};

/// A black/white panel with full and partial refresh. Implemented for [`Ssd2683`]; a mock
/// in tests.
pub trait MonoPanel {
    type Error;

    /// Show `frame` with a full refresh.
    fn full(&mut self, frame: &MonoFrame) -> Result<(), Self::Error>;

    /// Refresh `rect` from `shadow` (the panel's current image) to `frame`. On success the
    /// refreshed window has been copied into `shadow`; it is returned.
    fn partial(
        &mut self,
        shadow: &mut MonoFrame,
        frame: &MonoFrame,
        rect: Rect,
    ) -> Result<Rect, Self::Error>;
}

impl<SPI, CS, DC, RST, BUSY, PWR, D> MonoPanel for Ssd2683<SPI, CS, DC, RST, BUSY, PWR, D>
where
    SPI: embedded_hal::spi::SpiBus,
    CS: embedded_hal::digital::OutputPin,
    DC: embedded_hal::digital::OutputPin,
    RST: embedded_hal::digital::OutputPin,
    BUSY: embedded_hal::digital::InputPin,
    PWR: embedded_hal::digital::OutputPin,
    D: embedded_hal::delay::DelayNs,
{
    type Error = ssd2683::Error<SPI::Error>;

    fn full(&mut self, frame: &MonoFrame) -> Result<(), Self::Error> {
        self.show_mono(frame, None)
    }

    fn partial(
        &mut self,
        shadow: &mut MonoFrame,
        frame: &MonoFrame,
        rect: Rect,
    ) -> Result<Rect, Self::Error> {
        self.show_mono_partial(shadow, frame, rect)
    }
}

/// What [`Presenter::show`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shown {
    /// The panel already showed the frame.
    Unchanged,
    Partial(Rect),
    Full,
}

/// The refresh policy plus the logic around the driver calls.
#[derive(Clone, Debug, Default)]
pub struct Presenter {
    policy: RefreshPolicy,
}

impl Presenter {
    pub fn new(policy: RefreshPolicy) -> Self {
        Self { policy }
    }

    pub fn policy(&self) -> &RefreshPolicy {
        &self.policy
    }

    pub fn policy_mut(&mut self) -> &mut RefreshPolicy {
        &mut self.policy
    }

    /// Show `frame`. `shadow` must be the driver's shadow: what the panel shows now (it is
    /// only trusted while the policy says so).
    ///
    /// A partial refresh that fails is followed by a full refresh. An error is only
    /// returned if the full refresh fails; the next call then does a full refresh again.
    pub fn show<P: MonoPanel>(
        &mut self,
        panel: &mut P,
        frame: &MonoFrame,
        shadow: &mut MonoFrame,
        update: Update,
    ) -> Result<Shown, P::Error> {
        let dirty = frame.dirty_rect(shadow);
        if dirty.is_none() && self.policy.shadow_valid() {
            return Ok(Shown::Unchanged);
        }
        if let (RefreshKind::Partial, Some(rect)) = (self.policy.plan(update), dirty) {
            match panel.partial(shadow, frame, rect) {
                Ok(done) => {
                    self.policy.partial_done(true);
                    return Ok(Shown::Partial(done));
                }
                Err(_) => self.policy.partial_done(false),
            }
        }
        self.full(panel, frame, shadow)
    }

    /// A full refresh of `frame`, whatever the policy says (e.g. the user asked for one).
    pub fn full<P: MonoPanel>(
        &mut self,
        panel: &mut P,
        frame: &MonoFrame,
        shadow: &mut MonoFrame,
    ) -> Result<Shown, P::Error> {
        match panel.full(frame) {
            Ok(()) => {
                shadow.copy_from(frame);
                self.policy.full_done(true);
                Ok(Shown::Full)
            }
            Err(e) => {
                self.policy.full_done(false);
                Err(e)
            }
        }
    }

    /// Before deep sleep: one full refresh of `frame` if partial refreshes (or a failed
    /// refresh) may have left the panel unclean. Returns whether it refreshed.
    pub fn before_sleep<P: MonoPanel>(
        &mut self,
        panel: &mut P,
        frame: &MonoFrame,
        shadow: &mut MonoFrame,
    ) -> Result<bool, P::Error> {
        if !self.policy.full_before_sleep() {
            return Ok(false);
        }
        self.full(panel, frame, shadow).map(|_| true)
    }
}

#[cfg(test)]
mod tests {
    use embedded_graphics_core::pixelcolor::BinaryColor;

    use super::*;

    #[derive(Default)]
    struct Mock {
        calls: Vec<&'static str>,
        fail_partial: bool,
        fail_full: bool,
    }

    impl MonoPanel for Mock {
        type Error = ();

        fn full(&mut self, _: &MonoFrame) -> Result<(), ()> {
            self.calls.push("full");
            if self.fail_full { Err(()) } else { Ok(()) }
        }

        fn partial(
            &mut self,
            shadow: &mut MonoFrame,
            frame: &MonoFrame,
            rect: Rect,
        ) -> Result<Rect, ()> {
            self.calls.push("partial");
            if self.fail_partial {
                return Err(());
            }
            shadow.copy_rect(frame, rect);
            Ok(rect)
        }
    }

    fn frame_with(x: u32, y: u32) -> MonoFrame {
        let mut f = MonoFrame::new();
        f.set(x, y, BinaryColor::On);
        f
    }

    #[test]
    fn first_full_then_one_partial_per_change() {
        let mut p = Presenter::new(RefreshPolicy::default());
        let mut panel = Mock::default();
        let mut shadow = MonoFrame::new();
        let a = frame_with(10, 10);
        assert_eq!(
            p.show(&mut panel, &a, &mut shadow, Update::Change),
            Ok(Shown::Full)
        );
        assert_eq!(
            p.show(&mut panel, &a, &mut shadow, Update::Change),
            Ok(Shown::Unchanged)
        );
        let mut b = frame_with(10, 10);
        b.set(300, 200, BinaryColor::On);
        assert!(matches!(
            p.show(&mut panel, &b, &mut shadow, Update::Change),
            Ok(Shown::Partial(r)) if r.y == 200 && r.h == 1
        ));
        assert_eq!(panel.calls, ["full", "partial"]);
        assert_eq!(shadow.dirty_rect(&b), None);
        // Before sleep: one full refresh because a partial happened.
        assert_eq!(p.before_sleep(&mut panel, &b, &mut shadow), Ok(true));
        assert_eq!(p.before_sleep(&mut panel, &b, &mut shadow), Ok(false));
    }

    #[test]
    fn failed_partial_falls_back_to_full() {
        let mut p = Presenter::new(RefreshPolicy::default());
        let mut panel = Mock::default();
        let mut shadow = MonoFrame::new();
        p.show(&mut panel, &frame_with(1, 1), &mut shadow, Update::Change)
            .unwrap();
        panel.fail_partial = true;
        let b = frame_with(2, 2);
        assert_eq!(
            p.show(&mut panel, &b, &mut shadow, Update::Change),
            Ok(Shown::Full)
        );
        assert_eq!(panel.calls, ["full", "partial", "full"]);
        assert_eq!(shadow.dirty_rect(&b), None);
    }

    #[test]
    fn failed_full_keeps_forcing_full() {
        let mut p = Presenter::new(RefreshPolicy::default());
        let mut panel = Mock {
            fail_full: true,
            ..Mock::default()
        };
        let mut shadow = MonoFrame::new();
        let a = frame_with(1, 1);
        assert_eq!(p.show(&mut panel, &a, &mut shadow, Update::Change), Err(()));
        panel.fail_full = false;
        // Same frame, but the shadow is not trusted: full again.
        assert_eq!(
            p.show(&mut panel, &a, &mut shadow, Update::Change),
            Ok(Shown::Full)
        );
    }

    #[test]
    fn new_screens_are_full_unless_allowed() {
        let mut panel = Mock::default();
        let mut shadow = MonoFrame::new();
        let mut p = Presenter::new(RefreshPolicy::default());
        p.full(&mut panel, &frame_with(1, 1), &mut shadow).unwrap();
        let b = frame_with(5, 5);
        assert_eq!(
            p.show(&mut panel, &b, &mut shadow, Update::NewScreen),
            Ok(Shown::Full)
        );

        let mut p = Presenter::new(RefreshPolicy::default().with_partial_screens(true));
        p.full(&mut panel, &frame_with(1, 1), &mut shadow).unwrap();
        assert!(matches!(
            p.show(&mut panel, &b, &mut shadow, Update::NewScreen),
            Ok(Shown::Partial(_))
        ));
    }
}
