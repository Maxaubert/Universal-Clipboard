//! Downloads and unpacking.
//!
//! HTTPS goes through the system `curl` (Windows 10 1803 and newer and macOS ship it), so the
//! OS certificate store is used and no TLS or HTTP library is linked into the agent. The
//! agent never uses the GitHub API (plan 3.13): only static release files.
//!
//! `file://` URLs are for tests only: the caller passes `allow_file` only in test profiles.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::UpdateError;

/// The system `curl` by full path (no PATH lookup).
fn curl() -> PathBuf {
    if cfg!(windows) {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        PathBuf::from(root).join("System32").join("curl.exe")
    } else {
        PathBuf::from("/usr/bin/curl")
    }
}

/// The system `tar` by full path.
fn tar() -> PathBuf {
    if cfg!(windows) {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        PathBuf::from(root).join("System32").join("tar.exe")
    } else if Path::new("/usr/bin/tar").exists() {
        PathBuf::from("/usr/bin/tar")
    } else {
        PathBuf::from("/bin/tar")
    }
}

fn file_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    // file:///C:/x on Windows, file:///tmp/x on Unix.
    let rest = if cfg!(windows) {
        rest.trim_start_matches('/')
    } else {
        rest
    };
    Some(PathBuf::from(rest))
}

/// Fetch `url` into memory, at most `max_bytes`.
pub fn fetch(url: &str, max_bytes: u64, allow_file: bool) -> Result<Vec<u8>, UpdateError> {
    if let Some(p) = file_path(url) {
        if !allow_file {
            return Err(UpdateError::Fetch("file URLs are for tests only".into()));
        }
        let b =
            std::fs::read(&p).map_err(|e| UpdateError::Fetch(format!("{}: {e}", p.display())))?;
        if b.len() as u64 > max_bytes {
            return Err(UpdateError::Fetch("too big".into()));
        }
        return Ok(b);
    }
    if !url.starts_with("https://") {
        return Err(UpdateError::Fetch("only https".into()));
    }
    let mut c = Command::new(curl());
    c.args([
        "--fail",
        "--silent",
        "--show-error",
        "--location",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--max-time",
        "600",
        "--max-filesize",
        &max_bytes.to_string(),
        url,
    ]);
    no_window(&mut c);
    let o = c
        .output()
        .map_err(|e| UpdateError::Fetch(format!("curl: {e}")))?;
    if !o.status.success() {
        let err = String::from_utf8_lossy(&o.stderr);
        return Err(UpdateError::Fetch(err.trim().chars().take(200).collect()));
    }
    if o.stdout.len() as u64 > max_bytes {
        return Err(UpdateError::Fetch("too big".into()));
    }
    Ok(o.stdout)
}

/// Unpack a `.tar.gz` into `dest` (made empty first) with the system `tar`.
pub fn untar(archive: &Path, dest: &Path) -> Result<(), UpdateError> {
    if dest.exists() {
        std::fs::remove_dir_all(dest).map_err(|e| UpdateError::Install(e.to_string()))?;
    }
    std::fs::create_dir_all(dest).map_err(|e| UpdateError::Install(e.to_string()))?;
    let mut c = Command::new(tar());
    c.arg("-xzf").arg(archive).arg("-C").arg(dest);
    no_window(&mut c);
    let o = c
        .output()
        .map_err(|e| UpdateError::Install(format!("tar: {e}")))?;
    if !o.status.success() {
        return Err(UpdateError::Install(format!(
            "tar: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        )));
    }
    Ok(())
}

/// Make a `.tar.gz` of the contents of `dir` (release tool and tests).
pub fn tar_dir(dir: &Path, archive: &Path) -> Result<(), UpdateError> {
    let mut c = Command::new(tar());
    c.arg("-czf").arg(archive).arg("-C").arg(dir).arg(".");
    no_window(&mut c);
    let o = c
        .output()
        .map_err(|e| UpdateError::Install(format!("tar: {e}")))?;
    if !o.status.success() {
        return Err(UpdateError::Install(format!(
            "tar: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        )));
    }
    Ok(())
}

fn no_window(c: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    #[cfg(not(windows))]
    let _ = c;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_urls_only_when_allowed() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.json");
        std::fs::write(&p, b"{}").unwrap();
        let url = format!(
            "file:///{}",
            p.display()
                .to_string()
                .replace('\\', "/")
                .trim_start_matches('/')
        );
        assert_eq!(fetch(&url, 10, true).unwrap(), b"{}");
        assert!(fetch(&url, 10, false).is_err());
        assert!(fetch(&url, 1, true).is_err());
        assert!(fetch("http://example.invalid/x", 10, true).is_err());
    }

    #[test]
    fn tar_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src");
        std::fs::create_dir_all(src.join("a")).unwrap();
        std::fs::write(src.join("a/b.txt"), "hi").unwrap();
        let arc = d.path().join("x.tar.gz");
        tar_dir(&src, &arc).unwrap();
        let out = d.path().join("out");
        untar(&arc, &out).unwrap();
        assert_eq!(std::fs::read_to_string(out.join("a/b.txt")).unwrap(), "hi");
    }
}
