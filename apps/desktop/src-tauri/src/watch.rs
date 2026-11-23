//! The window process's own log (`ui.log` in the state folder) and its watchdog (2026-11-23).
//!
//! On 2026-11-23 the macOS menu bar icon stopped opening its panel, and opening the app showed
//! a Dock icon with no window, after two days of running. Nothing was logged: this process only
//! wrote to stderr, which goes nowhere when the agent starts it. So:
//!
//! - `ui.log` (rotates like `agent.log`, same rule: kinds and times, never clipboard content)
//!   gets the clicks, panel and window steps, failures and panics.
//! - A watchdog thread asks the main thread for a ping every [`PING_EVERY`]. When no answer
//!   came for [`HUNG_AFTER`], the process logs it and starts a fresh copy with `--tray`
//!   (the icon only), then ends.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tauri::AppHandle;

pub const PING_EVERY: Duration = Duration::from_secs(10);
pub const HUNG_AFTER: Duration = Duration::from_secs(30);
/// A copy started by the watchdog that hangs again this soon is not restarted again (no
/// restart loop): it aborts, which leaves a crash report.
pub const MIN_RUN_BEFORE_RESTART: Duration = Duration::from_secs(120);
/// The argument a restarted copy gets, so it waits for the old one to go first.
pub const RESTARTED_ARG: &str = "--restarted";

/// Start logging to `<state>/ui.log`.
pub fn init_log() {
    let Ok(p) = crate::profile() else { return };
    let dir = p.state_dir();
    let _ = std::fs::create_dir_all(&dir);
    uc_agent::log::init_named(&dir, "ui.log");
}

/// The flags this process was started with (only known flags, nothing else from the
/// command line).
pub fn flags(args: &[String]) -> Vec<&str> {
    args.iter()
        .skip(1)
        .map(String::as_str)
        .filter(|a| matches!(*a, "--tray" | "--route" | "update" | RESTARTED_ARG))
        .collect()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// What the watchdog does after one check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Fine,
    /// Start a fresh copy, then end this one.
    Restart,
    /// A restarted copy hung again soon: end without a new copy.
    GiveUp,
}

/// `silent`: how long the main thread has not answered. `restarted`: this copy was started by
/// the watchdog. `ran`: how long this copy has run.
pub fn verdict(silent: Duration, restarted: bool, ran: Duration) -> Verdict {
    if silent < HUNG_AFTER {
        Verdict::Fine
    } else if restarted && ran < MIN_RUN_BEFORE_RESTART {
        Verdict::GiveUp
    } else {
        Verdict::Restart
    }
}

/// Start the watchdog thread.
pub fn start(app: &AppHandle, restarted: bool) {
    let app = app.clone();
    let started = Instant::now();
    let last = std::sync::Arc::new(AtomicU64::new(now_ms()));
    let _ = std::thread::Builder::new()
        .name("ui-watchdog".into())
        .spawn(move || {
            loop {
                std::thread::sleep(PING_EVERY);
                let l = last.clone();
                if let Err(e) = app.run_on_main_thread(move || l.store(now_ms(), Ordering::SeqCst))
                {
                    tracing::warn!(error = %e, "watchdog: cannot reach the main thread");
                }
                let silent =
                    Duration::from_millis(now_ms().saturating_sub(last.load(Ordering::SeqCst)));
                match verdict(silent, restarted, started.elapsed()) {
                    Verdict::Fine => {}
                    Verdict::Restart => {
                        tracing::error!(
                            silent_s = silent.as_secs(),
                            "main thread hung: starting a fresh copy"
                        );
                        restart();
                    }
                    Verdict::GiveUp => {
                        tracing::error!(
                            silent_s = silent.as_secs(),
                            ran_s = started.elapsed().as_secs(),
                            "main thread hung again soon after a restart: aborting"
                        );
                        std::process::abort();
                    }
                }
            }
        });
}

/// Start a new copy with `--tray`, then end this one. The new copy waits a moment for this
/// one to go (the single-instance lock), see [`wait_if_restarted`].
fn restart() -> ! {
    match std::env::current_exe() {
        Ok(exe) => match std::process::Command::new(exe)
            .args(["--tray", RESTARTED_ARG])
            .spawn()
        {
            Ok(c) => tracing::info!(pid = c.id(), "fresh copy started"),
            Err(e) => tracing::error!(error = %e, "fresh copy did not start"),
        },
        Err(e) => tracing::error!(error = %e, "no path to this program"),
    }
    // A hung main thread may hold locks that a normal exit waits for: abort if the exit has
    // not finished in 3 s.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(3));
        std::process::abort();
    });
    std::process::exit(3);
}

/// A copy started by the watchdog: give the old copy time to end, so the single-instance
/// check does not hand this start to the old, hung copy.
pub fn wait_if_restarted(args: &[String]) -> bool {
    let restarted = args.iter().any(|a| a == RESTARTED_ARG);
    if restarted {
        std::thread::sleep(Duration::from_secs(4));
    }
    restarted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restarts_only_after_a_long_silence() {
        let s = Duration::from_secs;
        assert_eq!(verdict(s(0), false, s(10)), Verdict::Fine);
        assert_eq!(verdict(s(29), false, s(1000)), Verdict::Fine);
        assert_eq!(verdict(s(30), false, s(5)), Verdict::Restart);
        assert_eq!(verdict(s(30), true, s(1000)), Verdict::Restart);
        assert_eq!(verdict(s(40), true, s(60)), Verdict::GiveUp);
    }

    #[test]
    fn only_known_flags_are_logged() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            flags(&a(&[
                "/x/uc",
                "--tray",
                "/Users/someone/secret",
                RESTARTED_ARG
            ])),
            vec!["--tray", RESTARTED_ARG]
        );
    }
}
