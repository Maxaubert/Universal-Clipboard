//! The agent's log (plan 3.12): kinds, sizes, times and error codes. **Never clipboard
//! content, never keys.** The code only logs such facts; no log line takes a clip, a name
//! of a pasteboard type with content, a code or a secret. The file rotates at 256 KB
//! (`agent.log`, then `agent.log.1`).

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Rotation size.
pub const MAX_BYTES: u64 = 256 * 1024;

/// A file that rotates at [`MAX_BYTES`].
#[derive(Clone)]
pub struct Rotating {
    inner: Arc<Mutex<(PathBuf, Option<File>, u64)>>,
}

impl Rotating {
    /// Log to `dir/agent.log`.
    pub fn new(dir: &Path) -> Rotating {
        Rotating::named(dir, "agent.log")
    }

    /// Log to `dir/<file>` (the window process uses `ui.log`). `file` ends in `.log`; the
    /// rotated copy is `<file>.1`.
    pub fn named(dir: &Path, file: &str) -> Rotating {
        let path = dir.join(file);
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok();
        Rotating {
            inner: Arc::new(Mutex::new((path, f, size))),
        }
    }
}

impl Write for Rotating {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if g.2 + buf.len() as u64 > MAX_BYTES {
            g.1 = None;
            let old = g.0.with_extension("log.1");
            let _ = std::fs::rename(&g.0, old);
            g.1 = OpenOptions::new().create(true).append(true).open(&g.0).ok();
            g.2 = 0;
        }
        if g.1.is_none() {
            // The open failed before (folder missing, file locked): try again, so a passing
            // problem does not lose every later line.
            g.1 = OpenOptions::new().create(true).append(true).open(&g.0).ok();
        }
        g.2 += buf.len() as u64;
        match g.1.as_mut() {
            Some(f) => f.write(buf),
            None => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match g.1.as_mut() {
            Some(f) => f.flush(),
            None => Ok(()),
        }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Rotating {
    type Writer = Rotating;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// The level from `UCLIP_LOG` (any case): `trace`, `debug`, `info` (default), `warn`, `error`.
fn level_from(v: &str) -> tracing::Level {
    match v.trim().to_ascii_lowercase().as_str() {
        "trace" => tracing::Level::TRACE,
        "debug" => tracing::Level::DEBUG,
        "warn" => tracing::Level::WARN,
        "error" => tracing::Level::ERROR,
        _ => tracing::Level::INFO,
    }
}

/// Start logging to the state folder. `UCLIP_LOG` sets the level (default `info`). A panic
/// is written to the log too (the Windows agent has no console), then the old hook runs.
pub fn init(dir: &Path, _test: bool) {
    init_named(dir, "agent.log");
}

/// [`init`] for another log file in the state folder: the window and tray process writes
/// `ui.log` (it has no console on Windows or when the agent starts it on macOS).
pub fn init_named(dir: &Path, file: &str) {
    let level = level_from(&std::env::var("UCLIP_LOG").unwrap_or_default());
    let set = tracing_subscriber::fmt()
        .with_writer(Rotating::named(dir, file))
        .with_ansi(false)
        .with_max_level(level)
        .with_target(false)
        .try_init()
        .is_ok();
    if set {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // The message and place come from our code, never from a clip.
            tracing::error!(
                panic = %info,
                thread = std::thread::current().name().unwrap_or("?"),
                "panic"
            );
            prev(info);
        }));
    }
}

/// The last `n` lines of the log (`agent.log.1` then `agent.log`), for the diagnostic report.
pub fn tail(dir: &Path, n: usize) -> Vec<String> {
    let mut text = std::fs::read_to_string(dir.join("agent.log.1")).unwrap_or_default();
    text.push_str(&std::fs::read_to_string(dir.join("agent.log")).unwrap_or_default());
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_is_read_in_any_case() {
        assert_eq!(level_from("DEBUG"), tracing::Level::DEBUG);
        assert_eq!(level_from(" Trace "), tracing::Level::TRACE);
        assert_eq!(level_from("error"), tracing::Level::ERROR);
        assert_eq!(level_from(""), tracing::Level::INFO);
        assert_eq!(level_from("nonsense"), tracing::Level::INFO);
    }

    #[test]
    fn tail_reaches_into_the_rotated_file() {
        let dir = std::env::temp_dir().join(format!("uc-log-tail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("agent.log.1"),
            "a
b
",
        )
        .unwrap();
        std::fs::write(
            dir.join("agent.log"),
            "c
",
        )
        .unwrap();
        assert_eq!(tail(&dir, 2), vec!["b", "c"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotates_at_256_kb() {
        let dir = std::env::temp_dir().join(format!("uc-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut r = Rotating::new(&dir);
        let line = vec![b'x'; 1000];
        for _ in 0..300 {
            r.write_all(&line).unwrap();
        }
        r.flush().unwrap();
        assert!(dir.join("agent.log.1").exists());
        assert!(std::fs::metadata(dir.join("agent.log")).unwrap().len() <= MAX_BYTES);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn named_logs_rotate_to_their_own_file() {
        let dir = std::env::temp_dir().join(format!("uc-log-ui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut r = Rotating::named(&dir, "ui.log");
        let line = vec![b'x'; 1000];
        for _ in 0..300 {
            r.write_all(&line).unwrap();
        }
        r.flush().unwrap();
        assert!(dir.join("ui.log.1").exists());
        assert!(!dir.join("agent.log").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
