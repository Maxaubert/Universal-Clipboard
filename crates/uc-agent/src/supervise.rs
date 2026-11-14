//! `uc-agent --supervise` (plan 3.2, Linux): a tiny parent that starts the real agent and
//! starts it again if it crashes, with backoff. It stops when the agent exits cleanly (exit
//! code 0, for example after `uc leave` or a log out) or because another agent already runs.

use std::process::Command;
use std::time::{Duration, Instant};

use crate::agent::EXIT_ALREADY_RUNNING;

/// Backoff rule: 1 s, doubling to 60 s; back to 1 s after the agent ran for 60 s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backoff {
    wait: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            wait: Duration::from_secs(1),
        }
    }
}

impl Backoff {
    /// The child ran for `ran` and then crashed: how long to wait before the next start.
    pub fn after_crash(&mut self, ran: Duration) -> Duration {
        if ran >= Duration::from_secs(60) {
            self.wait = Duration::from_secs(1);
        }
        let w = self.wait;
        self.wait = (self.wait * 2).min(Duration::from_secs(60));
        w
    }
}

/// What the supervisor does after the child exits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum After {
    /// Stop supervising.
    Stop,
    /// Start again after a wait.
    Restart,
}

/// Decide from the exit code (`None`: killed by a signal).
pub fn after_exit(code: Option<i32>) -> After {
    match code {
        Some(0) | Some(EXIT_ALREADY_RUNNING) => After::Stop,
        _ => After::Restart,
    }
}

/// Run `exe args` until it exits cleanly. `limit` caps the number of starts (tests).
pub fn supervise(exe: &std::path::Path, args: &[String], limit: Option<u32>) -> i32 {
    let mut backoff = Backoff::default();
    let mut starts = 0u32;
    loop {
        // Plan 8.5: after `apt remove` the binary is gone; the supervisor ends quietly.
        if !exe.exists() {
            eprintln!("uc-agent is not installed any more; the supervisor stops");
            return 0;
        }
        starts += 1;
        let t = Instant::now();
        let status = Command::new(exe).args(args).status();
        let code = match status {
            Ok(s) => s.code(),
            Err(_) => Some(1),
        };
        if after_exit(code) == After::Stop {
            return code.unwrap_or(0);
        }
        if limit.is_some_and(|l| starts >= l) {
            return code.unwrap_or(1);
        }
        let w = backoff.after_crash(t.elapsed());
        eprintln!(
            "uc-agent exited ({code:?}); starting it again in {} s",
            w.as_secs()
        );
        std::thread::sleep(w);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stops_when_the_binary_is_gone() {
        let gone = std::env::temp_dir().join("uclip-no-such-agent-binary");
        assert_eq!(supervise(&gone, &[], Some(3)), 0);
    }

    #[test]
    fn backoff_doubles_and_resets() {
        let mut b = Backoff::default();
        let s = Duration::from_secs(1);
        assert_eq!(b.after_crash(s), Duration::from_secs(1));
        assert_eq!(b.after_crash(s), Duration::from_secs(2));
        assert_eq!(b.after_crash(s), Duration::from_secs(4));
        for _ in 0..10 {
            b.after_crash(s);
        }
        assert_eq!(b.after_crash(s), Duration::from_secs(60));
        assert_eq!(
            b.after_crash(Duration::from_secs(61)),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn clean_exit_stops() {
        assert_eq!(after_exit(Some(0)), After::Stop);
        assert_eq!(after_exit(Some(EXIT_ALREADY_RUNNING)), After::Stop);
        assert_eq!(after_exit(Some(101)), After::Restart);
        assert_eq!(after_exit(None), After::Restart);
    }
}
