//! `state.json` (plan 3.12): the membership chain, remembered addresses, settings and a few
//! flags. **Never clipboard content, never a key.** The device key and the group secret live
//! in the key store (`uc-keys`), not here.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::settings::Settings;

/// File name in the state folder.
pub const FILE: &str = "state.json";

/// Pause state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Pause {
    /// Not paused.
    #[default]
    Off,
    /// Paused until this wall time (ms).
    Until(u64),
    /// Paused until the user turns sync on.
    Forever,
}

/// The answer to the Windows firewall row (plan 2.2). Other OSes keep `Unanswered`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Firewall {
    /// Not asked yet: on Windows the network does not start (order rule).
    #[default]
    Unanswered,
    /// The user allowed incoming connections.
    Allowed,
    /// The user refused: outbound-only mode.
    Refused,
}

/// A notice to show until the user acts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Notice {
    /// "Another device removed this one."
    Removed,
}

/// One line of "Last activity". Kind, direction, device and time. Never content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activity {
    /// "text", "image", "text and image", ...
    pub kind: String,
    /// "sent", "received", "not sent".
    pub direction: String,
    /// The other device's name, if any.
    pub device: Option<String>,
    /// Wall time in ms.
    pub time_ms: u64,
    /// A short reason for "not sent" (for example "Image was too large to sync").
    pub note: Option<String>,
}

/// The whole file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StateFile {
    /// Format version.
    pub version: u32,
    /// This device's name before it has a group (afterwards the chain holds it).
    pub device_name: Option<String>,
    /// The UDP port in use (0: the profile default).
    pub port: u16,
    /// Settings.
    pub settings: Settings,
    /// Pause.
    pub pause: Pause,
    /// The membership chain, canonical order, each `body || signature` in hex.
    pub group: Vec<String>,
    /// The address book (`LanBook::encode`), hex.
    pub book: String,
    /// A notice for the user.
    pub notice: Option<Notice>,
    /// The Windows firewall row.
    pub firewall: Firewall,
    /// `uc setup` or the window's setup finished once.
    pub setup_done: bool,
    /// Last activity, newest last, at most [`ACTIVITY_MAX`].
    pub activity: Vec<Activity>,
}

/// Lines of "Last activity" kept.
pub const ACTIVITY_MAX: usize = 20;

impl StateFile {
    /// Path in a state folder.
    pub fn path(dir: &Path) -> PathBuf {
        dir.join(FILE)
    }

    /// Load. A missing file gives the defaults; a broken file is kept aside as
    /// `state.json.bad` and the defaults are used (the device then pairs again).
    pub fn load(dir: &Path) -> StateFile {
        let p = Self::path(dir);
        match std::fs::read(&p) {
            Ok(b) => match serde_json::from_slice(&b) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, "state file broken, starting fresh");
                    let _ = std::fs::rename(&p, dir.join("state.json.bad"));
                    StateFile::default()
                }
            },
            Err(_) => StateFile::default(),
        }
    }

    /// Save atomically (write a temp file, then rename).
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let mut s = self.clone();
        s.version = 1;
        let body = serde_json::to_vec_pretty(&s).map_err(std::io::Error::other)?;
        let tmp = dir.join("state.json.tmp");
        std::fs::write(&tmp, body)?;
        std::fs::rename(&tmp, Self::path(dir))
    }

    /// Add one line of "Last activity".
    pub fn push_activity(&mut self, a: Activity) {
        self.activity.push(a);
        if self.activity.len() > ACTIVITY_MAX {
            let n = self.activity.len() - ACTIVITY_MAX;
            self.activity.drain(..n);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_broken_file() {
        let dir = std::env::temp_dir().join(format!("uc-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = StateFile::default();
        assert_eq!(StateFile::load(&dir), s);
        s.port = 1234;
        s.pause = Pause::Until(99);
        s.save(&dir).unwrap();
        assert_eq!(StateFile::load(&dir).port, 1234);
        std::fs::write(StateFile::path(&dir), b"{not json").unwrap();
        assert_eq!(StateFile::load(&dir), StateFile::default());
        assert!(dir.join("state.json.bad").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn activity_is_bounded() {
        let mut s = StateFile::default();
        for i in 0..50 {
            s.push_activity(Activity {
                kind: "text".into(),
                direction: "sent".into(),
                device: None,
                time_ms: i,
                note: None,
            });
        }
        assert_eq!(s.activity.len(), ACTIVITY_MAX);
        assert_eq!(s.activity.last().unwrap().time_ms, 49);
    }
}
