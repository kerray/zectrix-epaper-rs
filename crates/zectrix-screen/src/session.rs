//! The interactive session: Wi-Fi and the shadow stay up until no button was pressed for
//! `idle_ms` (default 60 s), then the device goes to deep sleep (after the full refresh
//! that [`RefreshPolicy::full_before_sleep`](crate::RefreshPolicy::full_before_sleep) asks
//! for). Times are milliseconds from any monotonic clock.

pub const DEFAULT_IDLE_MS: u64 = 60_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Session {
    idle_ms: u64,
    last: u64,
}

impl Session {
    /// A session that started (or last saw activity) at `now`.
    pub const fn new(now: u64, idle_ms: u64) -> Self {
        Self { idle_ms, last: now }
    }

    pub const fn with_default(now: u64) -> Self {
        Self::new(now, DEFAULT_IDLE_MS)
    }

    /// Activity (a button event, a server reply that changed the screen, talking).
    /// A timestamp older than the last activity is ignored.
    pub fn touch(&mut self, now: u64) {
        self.last = self.last.max(now);
    }

    /// When the session ends unless there is activity before.
    pub fn deadline(&self) -> u64 {
        self.last.saturating_add(self.idle_ms)
    }

    /// Milliseconds left, 0 when the session is over.
    pub fn remaining(&self, now: u64) -> u64 {
        self.deadline().saturating_sub(now)
    }

    /// Time to go to sleep.
    pub fn should_sleep(&self, now: u64) -> bool {
        now >= self.deadline()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_timeout() {
        let mut s = Session::with_default(1_000);
        assert!(!s.should_sleep(60_999));
        assert!(s.should_sleep(61_000));
        s.touch(30_000);
        assert_eq!(s.remaining(31_000), 59_000);
        assert!(!s.should_sleep(61_000));
        assert!(s.should_sleep(90_000));
        s.touch(10_000); // stale
        assert_eq!(s.deadline(), 90_000);
    }
}
