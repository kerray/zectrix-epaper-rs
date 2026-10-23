//! When to use a partial refresh and when a full one (`docs/INTERACTIVE.md`, "Ghosting").
//!
//! * A new screen is always shown with a full refresh (the vendor UI does the same for
//!   page changes).
//! * A change on the same screen is a partial refresh while the shadow is valid and fewer
//!   than `max_partials` (default 8) partial refreshes happened since the last full one;
//!   otherwise it is a full refresh.
//! * A failed refresh invalidates the shadow, so the next update is a full refresh.
//! * Before deep sleep, if any partial refresh was attempted since the last good full
//!   refresh (or the last full refresh failed), do one full refresh.
//!
//! The policy only counts; the caller calls the driver and reports back.

/// Partial refreshes allowed between two full refreshes, as in the vendor UI.
pub const DEFAULT_MAX_PARTIALS: u16 = 8;

/// What caused an update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Update {
    /// A different screen (a new bundle).
    NewScreen,
    /// A change on the current screen (cursor move, patch).
    Change,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefreshKind {
    Full,
    Partial,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefreshPolicy {
    max_partials: u16,
    partials: u16,
    shadow_valid: bool,
    /// The panel may not show a clean image: a partial was attempted or a full failed.
    unclean: bool,
}

impl Default for RefreshPolicy {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_PARTIALS)
    }
}

impl RefreshPolicy {
    /// After a wake the shadow is unknown, so the first update is a full refresh.
    /// `max_partials` = 0 means never partial.
    pub const fn new(max_partials: u16) -> Self {
        Self {
            max_partials,
            partials: 0,
            shadow_valid: false,
            unclean: false,
        }
    }

    /// The caller has a shadow that matches the panel (e.g. restored from RTC memory) and
    /// the panel was last left after a clean full refresh.
    pub fn shadow_restored(&mut self) {
        self.shadow_valid = true;
        self.partials = 0;
        self.unclean = false;
    }

    /// How to show the next update.
    pub fn plan(&self, update: Update) -> RefreshKind {
        match update {
            Update::Change if self.shadow_valid && self.partials < self.max_partials => {
                RefreshKind::Partial
            }
            _ => RefreshKind::Full,
        }
    }

    /// Report a partial refresh call (one per rectangle).
    pub fn partial_done(&mut self, ok: bool) {
        self.unclean = true;
        if ok {
            self.partials = self.partials.saturating_add(1);
        } else {
            self.shadow_valid = false;
        }
    }

    /// Report a full refresh.
    pub fn full_done(&mut self, ok: bool) {
        self.partials = 0;
        self.shadow_valid = ok;
        self.unclean = !ok;
    }

    /// The shadow no longer matches the panel (e.g. something else drew on it).
    pub fn invalidate(&mut self) {
        self.shadow_valid = false;
    }

    /// Partial refreshes since the last full refresh.
    pub fn partials(&self) -> u16 {
        self.partials
    }

    pub fn shadow_valid(&self) -> bool {
        self.shadow_valid
    }

    /// Whether to do one full refresh of the final screen before deep sleep.
    pub fn full_before_sleep(&self) -> bool {
        self.unclean
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_update_after_wake_is_full() {
        let p = RefreshPolicy::default();
        assert_eq!(p.plan(Update::Change), RefreshKind::Full);
        assert!(!p.full_before_sleep());
    }

    #[test]
    fn full_after_max_partials() {
        let mut p = RefreshPolicy::default();
        p.full_done(true);
        for _ in 0..8 {
            assert_eq!(p.plan(Update::Change), RefreshKind::Partial);
            p.partial_done(true);
        }
        assert_eq!(p.partials(), 8);
        assert_eq!(p.plan(Update::Change), RefreshKind::Full);
        assert!(p.full_before_sleep());
        p.full_done(true);
        assert_eq!(p.partials(), 0);
        assert_eq!(p.plan(Update::Change), RefreshKind::Partial);
        assert!(!p.full_before_sleep());
    }

    #[test]
    fn new_screen_is_always_full() {
        let mut p = RefreshPolicy::new(100);
        p.full_done(true);
        assert_eq!(p.plan(Update::NewScreen), RefreshKind::Full);
    }

    #[test]
    fn failures_force_full() {
        let mut p = RefreshPolicy::default();
        p.full_done(true);
        p.partial_done(false);
        assert_eq!(p.plan(Update::Change), RefreshKind::Full);
        assert!(p.full_before_sleep());
        p.full_done(false);
        assert_eq!(p.plan(Update::Change), RefreshKind::Full);
        assert!(p.full_before_sleep());
        p.full_done(true);
        assert!(!p.full_before_sleep());
    }

    #[test]
    fn zero_means_never_partial_and_restore() {
        let mut p = RefreshPolicy::new(0);
        p.full_done(true);
        assert_eq!(p.plan(Update::Change), RefreshKind::Full);
        let mut p = RefreshPolicy::default();
        p.shadow_restored();
        assert_eq!(p.plan(Update::Change), RefreshKind::Partial);
        p.invalidate();
        assert_eq!(p.plan(Update::Change), RefreshKind::Full);
    }
}
