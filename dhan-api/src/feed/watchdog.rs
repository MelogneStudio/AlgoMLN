use crate::feed::candle::SessionWindow;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub trait FeedClock: Send + Sync + std::fmt::Debug {
    fn now_utc(&self) -> i64;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemFeedClock;

impl FeedClock for SystemFeedClock {
    fn now_utc(&self) -> i64 {
        chrono::Utc::now().timestamp()
    }
}

#[derive(Debug, Default)]
pub struct ManualFeedClock {
    now: AtomicI64,
}

impl ManualFeedClock {
    pub fn new(initial_utc: i64) -> Self {
        Self {
            now: AtomicI64::new(initial_utc),
        }
    }

    pub fn set(&self, now_utc: i64) {
        self.now.store(now_utc, Ordering::SeqCst);
    }

    pub fn advance(&self, seconds: i64) {
        self.now.fetch_add(seconds, Ordering::SeqCst);
    }
}

impl FeedClock for ManualFeedClock {
    fn now_utc(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchdogAction {
    /// Watchdog is disarmed (outside trading session window).
    Disarmed,
    /// Watchdog is armed and active, no timeout.
    Ok,
    /// Watchdog timed out (inactivity exceeded timeout while armed).
    TriggeredTimeout {
        elapsed_secs: i64,
        timeout_secs: i64,
    },
}

#[derive(Clone)]
pub struct FeedWatchdog {
    pub session_window: SessionWindow,
    pub timeout: Duration,
    pub clock: Arc<dyn FeedClock>,
    armed: bool,
    last_activity_utc: Option<i64>,
    window_entry_utc: Option<i64>,
}

impl std::fmt::Debug for FeedWatchdog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FeedWatchdog")
            .field("session_window", &self.session_window)
            .field("timeout", &self.timeout)
            .field("armed", &self.armed)
            .field("last_activity_utc", &self.last_activity_utc)
            .field("window_entry_utc", &self.window_entry_utc)
            .finish()
    }
}

impl FeedWatchdog {
    pub fn new(session_window: SessionWindow, timeout: Duration, clock: Arc<dyn FeedClock>) -> Self {
        Self {
            session_window,
            timeout,
            clock,
            armed: false,
            last_activity_utc: None,
            window_entry_utc: None,
        }
    }

    /// Notify watchdog that activity occurred at the given UTC time.
    pub fn on_activity(&mut self, now_utc: i64) {
        if self.session_window.is_in_session(now_utc) {
            if !self.armed {
                self.armed = true;
                self.window_entry_utc = Some(now_utc);
            }
            self.last_activity_utc = Some(now_utc);
        } else {
            // Activity occurred outside trading window: keep watchdog disarmed
            self.armed = false;
            self.window_entry_utc = None;
            self.last_activity_utc = None;
        }
    }

    /// Check watchdog status at given UTC time.
    pub fn check(&mut self, now_utc: i64) -> WatchdogAction {
        if !self.session_window.is_in_session(now_utc) {
            // Outside configured trading window: watchdog is disarmed
            self.armed = false;
            self.window_entry_utc = None;
            self.last_activity_utc = None;
            return WatchdogAction::Disarmed;
        }

        // Inside trading session window: arm if not already armed
        if !self.armed {
            self.armed = true;
            self.window_entry_utc = Some(now_utc);
        }

        // Inactivity baseline starts at window entry or last in-window activity
        let baseline = match self.last_activity_utc {
            Some(act) => act.max(self.window_entry_utc.unwrap_or(act)),
            None => self.window_entry_utc.unwrap_or(now_utc),
        };

        let elapsed_secs = (now_utc - baseline).max(0);
        let timeout_secs = self.timeout.as_secs() as i64;

        if elapsed_secs >= timeout_secs {
            WatchdogAction::TriggeredTimeout {
                elapsed_secs,
                timeout_secs,
            }
        } else {
            WatchdogAction::Ok
        }
    }

    pub fn record_activity(&mut self) {
        let now = self.clock.now_utc();
        self.on_activity(now);
    }

    pub fn check_now(&mut self) -> WatchdogAction {
        let now = self.clock.now_utc();
        self.check(now)
    }

    pub fn is_armed(&self) -> bool {
        self.armed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_watchdog_disarmed_outside_trading_window() {
        let clock = Arc::new(ManualFeedClock::new(1795140000)); // 07:30 IST
        let mut wd = FeedWatchdog::new(
            SessionWindow::for_instrument(true),
            Duration::from_secs(40),
            clock.clone(),
        );

        // Pre-open check
        assert_eq!(wd.check_now(), WatchdogAction::Disarmed);
        assert!(!wd.is_armed());

        // Advance 10 minutes without activity
        clock.advance(600);
        assert_eq!(wd.check_now(), WatchdogAction::Disarmed);
    }

    #[test]
    fn test_watchdog_arms_at_window_start_without_immediate_fire() {
        // 2026-11-20 09:14:55 IST: 1795146295 UTC
        let clock = Arc::new(ManualFeedClock::new(1795146295));
        let mut wd = FeedWatchdog::new(
            SessionWindow::for_instrument(true),
            Duration::from_secs(40),
            clock.clone(),
        );

        assert_eq!(wd.check_now(), WatchdogAction::Disarmed);

        // Advance to 09:15:00 IST (1795146300 UTC)
        clock.set(1795146300);
        assert_eq!(wd.check_now(), WatchdogAction::Ok);
        assert!(wd.is_armed());

        // At 09:15:05 IST, still within 40s timeout
        clock.advance(5);
        assert_eq!(wd.check_now(), WatchdogAction::Ok);

        // Activity arrives at 09:15:06 IST
        clock.advance(1);
        wd.record_activity();
        assert_eq!(wd.check_now(), WatchdogAction::Ok);

        // Inactivity up to 09:15:45 IST (39s since last activity at 06) -> Ok
        clock.set(1795146306 + 39);
        assert_eq!(wd.check_now(), WatchdogAction::Ok);

        // At 09:15:46 IST (40s elapsed) -> Timeout!
        clock.set(1795146306 + 40);
        assert_eq!(
            wd.check_now(),
            WatchdogAction::TriggeredTimeout {
                elapsed_secs: 40,
                timeout_secs: 40,
            }
        );
    }

    #[test]
    fn test_watchdog_disarms_at_session_end() {
        // F&O session ends at 15:15:00 IST (54,900s).
        // 2026-11-18 15:14:50 IST: 1794995090 UTC
        let clock = Arc::new(ManualFeedClock::new(1794995090));
        let mut wd = FeedWatchdog::new(
            SessionWindow::for_instrument(true),
            Duration::from_secs(40),
            clock.clone(),
        );

        wd.record_activity();
        assert_eq!(wd.check_now(), WatchdogAction::Ok);

        // Advance to 15:15:00 IST (10 seconds later)
        clock.advance(10);
        // At 15:15:00, session is over for F&O -> Disarmed!
        assert_eq!(wd.check_now(), WatchdogAction::Disarmed);
        assert!(!wd.is_armed());

        // Advance to 15:28:00 IST -> Still Disarmed, does NOT fire!
        clock.advance(780);
        assert_eq!(wd.check_now(), WatchdogAction::Disarmed);
    }
}
