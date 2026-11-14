//! When to look at `changeCount` again (plan 3.9).
//!
//! macOS has no clipboard change event, so the agent polls `changeCount`. One poll is a cheap
//! call to the pasteboard server: it reads no content and shows no prompt. This module only
//! decides the wait between polls, so it has no OS calls and is tested on every OS.
//!
//! The rule:
//! - Normal wait: 250 ms.
//! - Just after a change (for 1.5 s): 100 ms, because apps often write in several steps and the
//!   agent waits for the clipboard to settle (150 ms) before it reads.
//! - After 10 minutes with no change: 500 ms. The first change after a long idle time is seen
//!   at most 500 ms late. The next ones use the fast wait again.

use std::time::Duration;

/// Wait between polls in normal use.
pub const NORMAL: Duration = Duration::from_millis(250);
/// Wait between polls just after a change.
pub const FAST: Duration = Duration::from_millis(100);
/// Wait between polls after a long time with no change.
pub const IDLE: Duration = Duration::from_millis(500);
/// How long the fast wait lasts after a change.
pub const FAST_FOR_MS: u64 = 1_500;
/// How long with no change before the idle wait starts.
pub const IDLE_AFTER_MS: u64 = 10 * 60 * 1_000;

/// The result of one poll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Poll {
    /// The new change counter, if it changed since the last poll.
    pub changed: Option<u64>,
    /// How long to wait before the next poll.
    pub next: Duration,
}

/// Remembers the last `changeCount` and picks the wait before the next poll.
///
/// The caller passes the time (`now_ms`, any monotonic clock in milliseconds) and the counter,
/// so the rule is the same in tests and in the agent.
#[derive(Clone, Debug, Default)]
pub struct Poller {
    last_count: Option<u64>,
    last_change_ms: Option<u64>,
}

impl Poller {
    /// A poller that has not seen a counter yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Start from a known counter, for example the one read at agent start. A clip that was
    /// on the clipboard before the start is then not reported as a change.
    pub fn starting_at(count: u64, now_ms: u64) -> Self {
        Self {
            last_count: Some(count),
            last_change_ms: Some(now_ms),
        }
    }

    /// The last counter seen.
    pub fn last_count(&self) -> Option<u64> {
        self.last_count
    }

    /// Record one poll of `changeCount` taken at `now_ms`.
    ///
    /// The first poll only sets the start point: it reports no change.
    pub fn on_poll(&mut self, now_ms: u64, count: u64) -> Poll {
        let changed = match self.last_count {
            Some(last) if last != count => Some(count),
            _ => None,
        };
        if self.last_count.is_none() || changed.is_some() {
            self.last_change_ms = Some(now_ms);
        }
        self.last_count = Some(count);
        let since = now_ms.saturating_sub(self.last_change_ms.unwrap_or(now_ms));
        let next = if changed.is_some() || since < FAST_FOR_MS {
            FAST
        } else if since >= IDLE_AFTER_MS {
            IDLE
        } else {
            NORMAL
        };
        Poll { changed, next }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_poll_is_not_a_change() {
        let mut p = Poller::new();
        assert_eq!(p.on_poll(0, 7).changed, None);
        assert_eq!(p.last_count(), Some(7));
    }

    #[test]
    fn change_is_reported_once_then_fast_then_normal() {
        let mut p = Poller::starting_at(1, 0);
        // Long after start, nothing changed: normal wait.
        assert_eq!(
            p.on_poll(5_000, 1),
            Poll {
                changed: None,
                next: NORMAL
            }
        );
        let r = p.on_poll(5_250, 2);
        assert_eq!(r.changed, Some(2));
        assert_eq!(r.next, FAST);
        // Same counter again: no new change, still fast for a short time.
        let r = p.on_poll(5_350, 2);
        assert_eq!(r.changed, None);
        assert_eq!(r.next, FAST);
        // After the fast window: normal.
        assert_eq!(p.on_poll(5_250 + FAST_FOR_MS, 2).next, NORMAL);
    }

    #[test]
    fn idle_after_ten_minutes_and_back_to_fast_on_change() {
        let mut p = Poller::starting_at(3, 0);
        assert_eq!(p.on_poll(IDLE_AFTER_MS - 1, 3).next, NORMAL);
        assert_eq!(p.on_poll(IDLE_AFTER_MS, 3).next, IDLE);
        let r = p.on_poll(IDLE_AFTER_MS + 500, 4);
        assert_eq!(r.changed, Some(4));
        assert_eq!(r.next, FAST);
    }

    #[test]
    fn counter_going_down_counts_as_change() {
        // The pasteboard server restarted: the counter starts again from a low number.
        let mut p = Poller::starting_at(900, 0);
        assert_eq!(p.on_poll(10_000, 3).changed, Some(3));
    }

    #[test]
    fn time_going_backwards_does_not_panic() {
        let mut p = Poller::starting_at(1, 10_000);
        assert_eq!(p.on_poll(5, 1).next, FAST);
    }
}
