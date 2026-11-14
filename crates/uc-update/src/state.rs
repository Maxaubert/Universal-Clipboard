//! `update.json` in the agent's state folder: bad versions, failures in a row, last check.
//! No content, no keys.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// After this many failed updates in a row the agent shows "could not update itself" (plan
/// 2.7, 3.13 step 6) and stops trying until a newer version than the bad ones appears.
pub const GIVE_UP_AFTER: u32 = 3;

/// The updater's own small state.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateState {
    /// Versions that failed their health check. Never tried again.
    pub bad: Vec<String>,
    pub failures_in_row: u32,
    /// Unix ms of the last check.
    pub last_check_ms: u64,
    /// The last result in plain words, for the diagnostic report.
    pub last_result: Option<String>,
    /// The version installed by the last good update.
    pub last_good: Option<String>,
}

impl UpdateState {
    /// The file in `dir`.
    pub fn path(dir: &Path) -> PathBuf {
        dir.join("update.json")
    }

    /// Load, or the default if there is no file or it is broken.
    pub fn load(dir: &Path) -> UpdateState {
        std::fs::read(Self::path(dir))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Save (write a temp file, then rename).
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join("update.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, Self::path(dir))
    }

    /// True after [`GIVE_UP_AFTER`] failures in a row.
    pub fn gave_up(&self) -> bool {
        self.failures_in_row >= GIVE_UP_AFTER
    }

    /// True if `version` failed before.
    pub fn is_bad(&self, version: &str) -> bool {
        self.bad.iter().any(|v| v == version)
    }

    /// Record a failed attempt (`bad`: the version itself failed its health check).
    pub fn failed(&mut self, version: &str, why: &str, bad: bool) {
        self.failures_in_row += 1;
        if bad && !self.is_bad(version) {
            self.bad.push(version.to_string());
        }
        self.last_result = Some(format!("{version}: {why}"));
    }

    /// Record a good update.
    pub fn succeeded(&mut self, version: &str) {
        self.failures_in_row = 0;
        self.last_good = Some(version.to_string());
        self.last_result = Some(format!("{version}: updated"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_failures_give_up_and_success_resets() {
        let d = tempfile::tempdir().unwrap();
        let mut s = UpdateState::load(d.path());
        assert!(!s.gave_up());
        s.failed("0.0.2", "health check", true);
        s.failed("0.0.3", "download", false);
        assert!(!s.gave_up());
        s.failed("0.0.4", "health check", true);
        assert!(s.gave_up());
        assert!(s.is_bad("0.0.2") && !s.is_bad("0.0.3"));
        s.save(d.path()).unwrap();
        let mut l = UpdateState::load(d.path());
        assert_eq!(l, s);
        l.succeeded("0.0.5");
        assert!(!l.gave_up());
    }
}
