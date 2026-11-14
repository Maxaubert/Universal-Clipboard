//! Profiles (plan 3.2): `UCLIP_PROFILE=<name>` gives the agent, the CLI and the UI their own
//! state folder, IPC name, key store entry and UDP port. The empty name is the real app.
//!
//! Test profiles (any non-empty name) may use extra test switches. The real app never reads
//! them:
//!
//! | Variable | Effect (test profiles only) |
//! |---|---|
//! | `UCLIP_STATE_DIR` | state folder (tests use temp folders) |
//! | `UCLIP_TEST_KEYSTORE=file` | file key store (`uc-keys`) |
//! | `UCLIP_TEST_CLIPBOARD=os` | the OS backend, but only an isolated one: a private window station (Windows), a named pasteboard (`UCLIP_TEST_PASTEBOARD`, macOS), any display (Linux, for Docker). Else a fake clipboard. |
//! | `UCLIP_TEST_LOOPBACK=127.0.0.n` | bind only this loopback address, no mDNS |
//! | `UCLIP_TEST_PAIR_ADDRS=ip:port,...` | where a joiner looks for the inviter (no mDNS on loopback) |
//! | `UCLIP_TEST_FAST=1` | short timers |

use std::path::PathBuf;

/// The default UDP port of the real app.
pub const DEFAULT_PORT: u16 = 47390;

/// One profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    /// Empty for the real app.
    pub name: String,
    /// Test profiles: the state folder (`UCLIP_STATE_DIR`), read once.
    pub dir_override: Option<PathBuf>,
}

/// A bad profile name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadProfile;

impl std::fmt::Display for BadProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UCLIP_PROFILE must be 1 to 32 characters of a-z, 0-9 and -")
    }
}

impl Profile {
    /// A profile by name. Names are 1 to 32 characters of `a-z`, `0-9`, `-`, or empty.
    pub fn new(name: &str) -> Result<Profile, BadProfile> {
        let ok = name.len() <= 32
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if ok {
            Ok(Profile {
                name: name.to_string(),
                dir_override: None,
            })
        } else {
            Err(BadProfile)
        }
    }

    /// From `UCLIP_PROFILE` (empty or unset: the real app).
    pub fn from_env() -> Result<Profile, BadProfile> {
        let mut p = Profile::new(&std::env::var("UCLIP_PROFILE").unwrap_or_default())?;
        p.dir_override = p.test_env("UCLIP_STATE_DIR").map(PathBuf::from);
        Ok(p)
    }

    /// A test profile with its own state folder (tests that drive several agents).
    pub fn test(name: &str, dir: PathBuf) -> Result<Profile, BadProfile> {
        let mut p = Profile::new(name)?;
        if !p.is_test() {
            return Err(BadProfile);
        }
        p.dir_override = Some(dir);
        Ok(p)
    }

    /// True for every profile except the real app.
    pub fn is_test(&self) -> bool {
        !self.name.is_empty()
    }

    /// A test switch, or `None` in the real app.
    pub fn test_env(&self, var: &str) -> Option<String> {
        if !self.is_test() {
            return None;
        }
        std::env::var(var).ok().filter(|v| !v.is_empty())
    }

    /// True if `UCLIP_TEST_FAST=1` (test profiles only).
    pub fn fast(&self) -> bool {
        self.test_env("UCLIP_TEST_FAST").as_deref() == Some("1")
    }

    fn suffix(&self) -> String {
        if self.is_test() {
            format!("-{}", self.name)
        } else {
            String::new()
        }
    }

    /// The state folder (plan 3.12).
    pub fn state_dir(&self) -> PathBuf {
        if let Some(d) = self.dir_override.as_ref().filter(|_| self.is_test()) {
            return d.clone();
        }
        let home = || {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
        };
        if cfg!(windows) {
            let base = std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_default();
            base.join(format!("UniversalClipboard{}", self.suffix()))
        } else if cfg!(target_os = "macos") {
            home()
                .join("Library/Application Support")
                .join(format!("uclip{}", self.suffix()))
        } else {
            let base = std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home().join(".local/share"));
            base.join(format!("uclip{}", self.suffix()))
        }
    }

    /// The Unix socket path (plan 3.2): `$XDG_RUNTIME_DIR/uclip[-profile]/agent.sock` on
    /// Linux when that folder exists, else `agent.sock` in the state folder.
    #[cfg(unix)]
    pub fn socket_path(&self) -> PathBuf {
        if self.dir_override.is_none()
            && cfg!(target_os = "linux")
            && let Some(rt) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)
            && rt.is_absolute()
            && rt.is_dir()
        {
            return rt
                .join(format!("uclip{}", self.suffix()))
                .join("agent.sock");
        }
        self.state_dir().join("agent.sock")
    }

    /// The Windows pipe name (plan 3.2): `\\.\pipe\uclip-<user SID>[-profile]`.
    #[cfg(windows)]
    pub fn pipe_name(&self) -> std::io::Result<String> {
        let sid = crate::win::current_user_sid()?;
        Ok(format!(r"\\.\pipe\uclip-{sid}{}", self.suffix()))
    }

    /// The first UDP port to try. Profiles get their own (the agent moves to another one if
    /// it is taken, plan 3.2).
    pub fn default_port(&self) -> u16 {
        if !self.is_test() {
            return DEFAULT_PORT;
        }
        let mut h: u32 = 2166136261;
        for b in self.name.bytes() {
            h = (h ^ u32::from(b)).wrapping_mul(16777619);
        }
        DEFAULT_PORT + 1 + (h % 500) as u16
    }

    /// Service name of the key store entry (`uc-keys` adds the profile).
    pub fn keystore_name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(Profile::new("").is_ok());
        assert!(Profile::new("dev").is_ok());
        assert!(Profile::new("t-1").is_ok());
        assert!(Profile::new("Dev").is_err());
        assert!(Profile::new("a/b").is_err());
        assert!(Profile::new(&"a".repeat(33)).is_err());
    }

    #[test]
    fn real_app_ignores_test_switches() {
        let p = Profile::new("").unwrap();
        assert_eq!(p.test_env("PATH"), None);
        assert!(!p.fast());
        assert_eq!(p.default_port(), DEFAULT_PORT);
    }

    #[test]
    fn profiles_get_their_own_folder_and_port() {
        let a = Profile::new("dev").unwrap();
        let b = Profile::new("dev2").unwrap();
        assert_ne!(a.default_port(), DEFAULT_PORT);
        assert!(a.state_dir().to_string_lossy().ends_with("-dev"));
        assert_ne!(a.state_dir(), b.state_dir());
    }
}
