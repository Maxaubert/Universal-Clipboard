//! The settings of the settings window (plan 2.5), as keys that `uc settings set` takes.

use serde::{Deserialize, Serialize};

/// The stored settings. Device name is not here: it lives in the membership chain (a
/// `rename` change), so every device sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Start at login (on).
    pub start_at_login: bool,
    /// Sync images (on).
    pub sync_images: bool,
    /// Largest image that is sent, in MB (1 to 25, default 25, plan 3.11).
    pub image_limit_mb: u32,
    /// Keep formatting (on). Off sends plain text only. HTML sync itself ships in M11.
    pub keep_formatting: bool,
    /// macOS only: also pass on what Apple's Universal Clipboard brings (plan decision 8).
    /// Off by default: decision 8 is still open and off is the safer choice.
    pub forward_apple_clips: bool,
    /// Copy smaller files automatically (on, file sync spec 5): files and folders copied
    /// with Ctrl+C / Cmd+C, up to `file_limit_mb`, go to the other devices by themselves.
    /// Off only stops these automatic copies; "Copy files...", `uc copy` and receiving go on.
    /// Stored as `sync_files` by 0.2.0 dev builds (2026-11-17 rename).
    #[serde(alias = "sync_files")]
    pub auto_file_copy: bool,
    /// Largest file copy that syncs by itself, in MB (1 to 10,240, default 100). Checked on
    /// the copying device only; "Copy files..." and `uc copy` have no limit.
    pub file_limit_mb: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            start_at_login: true,
            sync_images: true,
            image_limit_mb: 25,
            keep_formatting: true,
            forward_apple_clips: false,
            auto_file_copy: true,
            file_limit_mb: uc_core::caps::FILE_LIMIT_DEFAULT_MB as u32,
        }
    }
}

/// What kind of value a key takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
    /// `on` or `off`.
    OnOff,
    /// A whole number in a range.
    Number(u32, u32),
    /// Text (a name).
    Text,
}

/// One settings key.
#[derive(Clone, Copy, Debug)]
pub struct Key {
    /// The name used by `uc settings set`.
    pub name: &'static str,
    /// The label in the settings window.
    pub label: &'static str,
    /// The value kind.
    pub kind: ValueKind,
    /// Only on macOS.
    pub mac_only: bool,
}

/// Every key, in window order. `device-name` is handled by the agent (a rename change).
pub const KEYS: &[Key] = &[
    Key {
        name: "start-at-login",
        label: "Start at login",
        kind: ValueKind::OnOff,
        mac_only: false,
    },
    Key {
        name: "sync-images",
        label: "Sync images",
        kind: ValueKind::OnOff,
        mac_only: false,
    },
    Key {
        name: "image-limit-mb",
        label: "Largest image to sync (MB)",
        kind: ValueKind::Number(1, 25),
        mac_only: false,
    },
    Key {
        name: "keep-formatting",
        label: "Keep formatting",
        kind: ValueKind::OnOff,
        mac_only: false,
    },
    Key {
        name: "auto-file-copy",
        label: "Copy smaller files automatically",
        kind: ValueKind::OnOff,
        mac_only: false,
    },
    Key {
        name: "file-limit-mb",
        label: "Largest automatic file copy (MB), on this device",
        kind: ValueKind::Number(
            uc_core::caps::FILE_LIMIT_MIN_MB as u32,
            uc_core::caps::FILE_LIMIT_MAX_MB as u32,
        ),
        mac_only: false,
    },
    Key {
        name: "forward-apple-clips",
        label: "Also pass on what Apple's Universal Clipboard brings",
        kind: ValueKind::OnOff,
        mac_only: true,
    },
    Key {
        name: "device-name",
        label: "This device's name",
        kind: ValueKind::Text,
        mac_only: false,
    },
];

/// Find a key by name.
pub fn key(name: &str) -> Option<&'static Key> {
    KEYS.iter().find(|k| k.name == name)
}

fn on_off(v: bool) -> String {
    if v { "on" } else { "off" }.to_string()
}

fn parse_on_off(v: &str) -> Result<bool, String> {
    match v.to_ascii_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Ok(true),
        "off" | "false" | "no" | "0" => Ok(false),
        _ => Err(format!("Use on or off, not \"{v}\".")),
    }
}

impl Settings {
    /// The value of a key as text. `device-name` is not stored here.
    pub fn get(&self, name: &str) -> Option<String> {
        Some(match name {
            "start-at-login" => on_off(self.start_at_login),
            "sync-images" => on_off(self.sync_images),
            "image-limit-mb" => self.image_limit_mb.to_string(),
            "keep-formatting" => on_off(self.keep_formatting),
            "forward-apple-clips" => on_off(self.forward_apple_clips),
            "auto-file-copy" => on_off(self.auto_file_copy),
            "file-limit-mb" => self.file_limit_mb.to_string(),
            _ => return None,
        })
    }

    /// Set a key from text. Errors are plain sentences for the user.
    pub fn set(&mut self, name: &str, value: &str) -> Result<(), String> {
        let k = key(name).ok_or_else(|| format!("There is no setting \"{name}\"."))?;
        if k.mac_only && !cfg!(target_os = "macos") {
            return Err(format!("\"{name}\" exists only on a Mac."));
        }
        match name {
            "start-at-login" => self.start_at_login = parse_on_off(value)?,
            "sync-images" => self.sync_images = parse_on_off(value)?,
            "keep-formatting" => self.keep_formatting = parse_on_off(value)?,
            "forward-apple-clips" => self.forward_apple_clips = parse_on_off(value)?,
            "auto-file-copy" => self.auto_file_copy = parse_on_off(value)?,
            "image-limit-mb" => {
                let n: u32 = value
                    .trim()
                    .parse()
                    .map_err(|_| "Use a whole number from 1 to 25.".to_string())?;
                if !(1..=25).contains(&n) {
                    return Err("Use a whole number from 1 to 25.".into());
                }
                self.image_limit_mb = n;
            }
            "file-limit-mb" => {
                let ValueKind::Number(lo, hi) = k.kind else {
                    return Err(format!("\"{name}\" is set by the agent."));
                };
                let text = format!("Use a whole number from {lo} to {hi}.");
                let n: u32 = value.trim().parse().map_err(|_| text.clone())?;
                if !(lo..=hi).contains(&n) {
                    return Err(text);
                }
                self.file_limit_mb = n;
            }
            _ => return Err(format!("\"{name}\" is set by the agent.")),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_get() {
        let mut s = Settings::default();
        s.set("sync-images", "off").unwrap();
        assert_eq!(s.get("sync-images").as_deref(), Some("off"));
        s.set("image-limit-mb", "10").unwrap();
        assert_eq!(s.image_limit_mb, 10);
        assert!(s.set("image-limit-mb", "26").is_err());
        assert!(s.set("sync-images", "maybe").is_err());
        assert!(s.set("nope", "on").is_err());
    }

    #[test]
    fn file_settings() {
        let mut s = Settings::default();
        assert!(s.auto_file_copy);
        assert_eq!(s.file_limit_mb, 100);
        s.set("auto-file-copy", "off").unwrap();
        assert_eq!(s.get("auto-file-copy").as_deref(), Some("off"));
        assert!(s.set("sync-files", "off").is_err());
        s.set("file-limit-mb", "10240").unwrap();
        assert_eq!(s.get("file-limit-mb").as_deref(), Some("10240"));
        s.set("file-limit-mb", " 1 ").unwrap();
        assert_eq!(s.file_limit_mb, 1);
        for bad in ["0", "10241", "-5", "lots", ""] {
            let e = s.set("file-limit-mb", bad).unwrap_err();
            assert_eq!(e, "Use a whole number from 1 to 10240.");
        }
        assert_eq!(s.file_limit_mb, 1);
    }

    #[test]
    fn old_state_files_get_the_file_defaults() {
        let s: Settings = serde_json::from_str(r#"{"sync_images": false}"#).unwrap();
        assert!(!s.sync_images);
        assert!(s.auto_file_copy);
        assert_eq!(s.file_limit_mb, 100);
    }

    #[test]
    fn a_0_2_0_dev_state_file_keeps_files_off() {
        // 0.2.0 dev builds stored the setting as "sync_files" (renamed 2026-11-17).
        let s: Settings =
            serde_json::from_str(r#"{"sync_files": false, "file_limit_mb": 7}"#).unwrap();
        assert!(!s.auto_file_copy);
        assert_eq!(s.get("auto-file-copy").as_deref(), Some("off"));
        assert_eq!(s.file_limit_mb, 7);
        // Saved again, it uses the new name.
        let out = serde_json::to_string(&s).unwrap();
        assert!(out.contains("\"auto_file_copy\":false"), "{out}");
        assert!(!out.contains("sync_files"), "{out}");
    }

    #[test]
    fn apple_forwarding_is_off_by_default() {
        assert!(!Settings::default().forward_apple_clips);
    }

    #[test]
    fn every_key_but_the_name_is_readable() {
        let s = Settings::default();
        for k in KEYS {
            if k.name != "device-name" {
                assert!(s.get(k.name).is_some(), "{}", k.name);
            }
        }
    }
}
