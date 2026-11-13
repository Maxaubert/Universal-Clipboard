//! Hybrid logical clock and the conflict rule (plan 3.8).
//!
//! A stamp is (wall time in ms, counter, device ID). Stamps compare in that order, so two
//! devices never make equal stamps and every device picks the same winner: the highest stamp.
//! The clock never goes below the highest stamp it has seen, so a clock that is behind does no
//! harm. A stamp more than [`FUTURE_LIMIT_MS`] ahead of this device's clock is refused, so a
//! device with a clock far ahead cannot win every conflict.

use serde::{Deserialize, Serialize};

use crate::ids::DeviceId;

/// Stamps more than 2 minutes ahead of the local clock are refused.
pub const FUTURE_LIMIT_MS: u64 = 2 * 60 * 1000;

/// A hybrid logical clock stamp. `Ord` is the conflict rule: higher wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Stamp {
    /// Wall time in milliseconds since the Unix epoch.
    pub wall_ms: u64,
    /// Counter for events in the same millisecond.
    pub counter: u32,
    /// The device that made the stamp (tie-break).
    pub device: DeviceId,
}

/// A stamp from a peer was too far in the future.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FutureStamp {
    /// How far ahead of our clock it was, in ms.
    pub ahead_ms: u64,
}

/// The hybrid logical clock of one device.
#[derive(Clone, Debug)]
pub struct Hlc {
    device: DeviceId,
    wall_ms: u64,
    counter: u32,
    future_limit_ms: u64,
}

impl Hlc {
    /// A new clock for `device`.
    pub fn new(device: DeviceId) -> Self {
        Self {
            device,
            wall_ms: 0,
            counter: 0,
            future_limit_ms: FUTURE_LIMIT_MS,
        }
    }

    /// Change the future limit (tests only use the default).
    pub fn with_future_limit(mut self, ms: u64) -> Self {
        self.future_limit_ms = ms;
        self
    }

    /// The clock's wall reading: the larger of the real time and the highest time seen.
    /// Ages are measured against this (plan 3.8: age uses the hybrid clock).
    pub fn now_wall(&self, now_ms: u64) -> u64 {
        self.wall_ms.max(now_ms)
    }

    /// Make a new stamp. It is higher than every stamp made or accepted before.
    pub fn tick(&mut self, now_ms: u64) -> Stamp {
        if now_ms > self.wall_ms {
            self.wall_ms = now_ms;
            self.counter = 0;
        } else if self.counter == u32::MAX {
            self.wall_ms += 1;
            self.counter = 0;
        } else {
            self.counter += 1;
        }
        Stamp {
            wall_ms: self.wall_ms,
            counter: self.counter,
            device: self.device,
        }
    }

    /// Check a peer's stamp against the future limit, without changing the clock.
    pub fn check(&self, stamp: &Stamp, now_ms: u64) -> Result<(), FutureStamp> {
        let limit = now_ms.saturating_add(self.future_limit_ms);
        if stamp.wall_ms > limit {
            Err(FutureStamp {
                ahead_ms: stamp.wall_ms - now_ms,
            })
        } else {
            Ok(())
        }
    }

    /// Accept a peer's stamp: refuse it if it is too far ahead, else move the clock up to it.
    pub fn observe(&mut self, stamp: &Stamp, now_ms: u64) -> Result<(), FutureStamp> {
        self.check(stamp, now_ms)?;
        if (stamp.wall_ms, stamp.counter) > (self.wall_ms, self.counter) {
            self.wall_ms = stamp.wall_ms;
            self.counter = stamp.counter;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn dev(n: u8) -> DeviceId {
        DeviceId([n; 32])
    }

    #[test]
    fn ticks_are_strictly_increasing_even_when_wall_goes_back() {
        let mut c = Hlc::new(dev(1));
        let a = c.tick(1000);
        let b = c.tick(1000);
        let d = c.tick(500);
        assert!(a < b && b < d);
        assert_eq!(d.wall_ms, 1000);
    }

    #[test]
    fn future_stamp_is_refused() {
        let mut c = Hlc::new(dev(1));
        let s = Stamp {
            wall_ms: 10_000 + FUTURE_LIMIT_MS + 1,
            counter: 0,
            device: dev(2),
        };
        assert_eq!(
            c.observe(&s, 10_000),
            Err(FutureStamp {
                ahead_ms: FUTURE_LIMIT_MS + 1
            })
        );
        // The clock did not move.
        assert_eq!(c.tick(10_000).wall_ms, 10_000);
        let ok = Stamp {
            wall_ms: 10_000 + FUTURE_LIMIT_MS,
            counter: 3,
            device: dev(2),
        };
        assert!(c.observe(&ok, 10_000).is_ok());
    }

    #[test]
    fn behind_clock_still_wins_after_observing() {
        // Device 1's wall clock is 1 minute behind. After it sees device 2's stamp, its next
        // copy still wins: the clock never goes below what it has seen.
        let mut slow = Hlc::new(dev(1));
        let mut fast = Hlc::new(dev(2));
        let s2 = fast.tick(5_000_000);
        slow.observe(&s2, 5_000_000 - 60_000).unwrap();
        let s1 = slow.tick(5_000_000 - 60_000);
        assert!(s1 > s2);
    }

    #[test]
    fn counter_overflow_moves_wall() {
        let mut c = Hlc::new(dev(1));
        let s = Stamp {
            wall_ms: 7,
            counter: u32::MAX,
            device: dev(2),
        };
        c.observe(&s, 7).unwrap();
        let t = c.tick(7);
        assert!(t > s);
        assert_eq!((t.wall_ms, t.counter), (8, 0));
    }

    #[test]
    fn device_id_breaks_ties() {
        let a = Stamp {
            wall_ms: 1,
            counter: 0,
            device: dev(1),
        };
        let b = Stamp {
            wall_ms: 1,
            counter: 0,
            device: dev(2),
        };
        assert!(b > a);
    }

    proptest! {
        /// Random events on 3 clocks: every new stamp is above everything its clock has seen.
        #[test]
        fn causality(events in proptest::collection::vec((0usize..3, 0usize..3, 0u64..10_000), 1..200)) {
            let mut clocks: Vec<Hlc> = (0..3).map(|i| Hlc::new(dev(i as u8))).collect();
            let mut seen: Vec<Option<Stamp>> = vec![None; 3];
            let mut last: Vec<Option<Stamp>> = vec![None; 3];
            for (a, b, now) in events {
                let s = clocks[a].tick(now);
                if let Some(p) = seen[a] { prop_assert!(s > p); }
                if let Some(p) = last[a] { prop_assert!(s > p); }
                last[a] = Some(s);
                // Deliver it to b with b's clock near a's.
                if clocks[b].observe(&s, now).is_ok() {
                    seen[b] = Some(seen[b].map_or(s, |p| p.max(s)));
                }
            }
        }
    }
}
