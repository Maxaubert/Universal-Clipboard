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
        let path = dir.join("agent.log");
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

/// Start logging to the state folder. `UCLIP_LOG` sets the level (default `info`).
pub fn init(dir: &Path, _test: bool) {
    let level = match std::env::var("UCLIP_LOG").as_deref() {
        Ok("debug") => tracing::Level::DEBUG,
        Ok("warn") => tracing::Level::WARN,
        _ => tracing::Level::INFO,
    };
    let _ = tracing_subscriber::fmt()
        .with_writer(Rotating::new(dir))
        .with_ansi(false)
        .with_max_level(level)
        .with_target(false)
        .try_init();
}

/// The last `n` lines of the log, for the diagnostic report.
pub fn tail(dir: &Path, n: usize) -> Vec<String> {
    let text = std::fs::read_to_string(dir.join("agent.log")).unwrap_or_default();
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
}
