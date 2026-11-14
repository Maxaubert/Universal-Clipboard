//! The clipboard the agent uses: the OS backend, or a fake one in test profiles.
//!
//! **Safety rule (plan 6.4, CLAUDE.md):** a test profile never touches the user's
//! clipboard. With `UCLIP_TEST_CLIPBOARD=os` it uses the OS backend only where that is
//! isolated: a private window station on Windows (`tools/winhost`), a named pasteboard on
//! macOS (`UCLIP_TEST_PASTEBOARD`), and on Linux a display that the test made (Docker,
//! Xvfb, headless Sway). Anything else is refused at start.

use uc_clip::{Clipboard, ClipboardError, FakeClipboard, Format, LineEnding, RawPart, Snapshot};

use crate::profile::Profile;

/// A change-event source the agent waits on.
pub enum Changes {
    /// The backend sends the change counter on this channel (Windows, Linux).
    Channel(std::sync::mpsc::Receiver<u64>),
    /// The agent polls `changeCount` (macOS).
    Poll,
    /// Changes come only from `debug.copy` (fake clipboard).
    Manual,
}

/// The clipboard backend.
pub enum Backend {
    /// In memory (tests).
    Fake(FakeClipboard),
    /// Windows.
    #[cfg(windows)]
    Win(uc_clip_win::WinClipboard),
    /// macOS.
    #[cfg(target_os = "macos")]
    Mac(uc_clip_mac::MacClipboard),
    /// Linux.
    #[cfg(target_os = "linux")]
    Linux(uc_clip_linux::LinuxClipboard),
}

impl std::fmt::Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name())
    }
}

/// Why no clipboard could be opened.
#[derive(Debug, Clone)]
pub struct OpenFailed(pub String);

impl Backend {
    /// Open the right backend for this profile.
    pub fn open(p: &Profile) -> Result<(Backend, Changes), OpenFailed> {
        if p.is_test() && p.test_env("UCLIP_TEST_CLIPBOARD").as_deref() != Some("os") {
            let fake = if cfg!(windows) {
                FakeClipboard::windows()
            } else {
                FakeClipboard::new()
            };
            return Ok((Backend::Fake(fake), Changes::Manual));
        }
        Self::open_os(p)
    }

    #[cfg(windows)]
    fn open_os(p: &Profile) -> Result<(Backend, Changes), OpenFailed> {
        if p.is_test() {
            let station = uc_winhost::current_station().unwrap_or_default();
            if station.eq_ignore_ascii_case("WinSta0") || station.is_empty() {
                return Err(OpenFailed(
                    "a test profile uses the Windows clipboard only inside tools/winhost".into(),
                ));
            }
        }
        let mut c = uc_clip_win::WinClipboard::new().map_err(|e| OpenFailed(e.to_string()))?;
        let rx = c.take_changes();
        Ok((Backend::Win(c), Changes::Channel(rx)))
    }

    #[cfg(target_os = "macos")]
    fn open_os(p: &Profile) -> Result<(Backend, Changes), OpenFailed> {
        let c = uc_clip_mac::MacClipboard::from_env();
        if p.is_test() && c.is_general() {
            return Err(OpenFailed(
                "a test profile needs UCLIP_TEST_PASTEBOARD (a named pasteboard)".into(),
            ));
        }
        Ok((Backend::Mac(c), Changes::Poll))
    }

    #[cfg(target_os = "linux")]
    fn open_os(_p: &Profile) -> Result<(Backend, Changes), OpenFailed> {
        let c = uc_clip_linux::LinuxClipboard::open().map_err(|e| OpenFailed(e.to_string()))?;
        let rx = c.subscribe();
        Ok((Backend::Linux(c), Changes::Channel(rx)))
    }

    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    fn open_os(_p: &Profile) -> Result<(Backend, Changes), OpenFailed> {
        Err(OpenFailed("no clipboard backend for this OS".into()))
    }

    /// A short name for the diagnostic report.
    pub fn name(&self) -> String {
        match self {
            Backend::Fake(_) => "fake (test)".into(),
            #[cfg(windows)]
            Backend::Win(_) => "windows".into(),
            #[cfg(target_os = "macos")]
            Backend::Mac(m) => {
                if m.is_general() {
                    "macos".into()
                } else {
                    "macos (named test pasteboard)".into()
                }
            }
            #[cfg(target_os = "linux")]
            Backend::Linux(l) => format!("linux {}", l.kind().name()),
        }
    }

    /// macOS: the change counter, for polling. Other backends: `None`.
    pub fn change_count(&self) -> Option<u64> {
        match self {
            #[cfg(target_os = "macos")]
            Backend::Mac(m) => Some(m.change_count()),
            _ => None,
        }
    }

    /// macOS "Paste from Other Apps" (plan 3.9): `Some("ask")`, `Some("deny")`, ... or `None`.
    pub fn paste_access(&self) -> Option<&'static str> {
        match self {
            #[cfg(target_os = "macos")]
            Backend::Mac(m) => {
                use uc_clip_mac::AccessBehavior as A;
                m.access_behavior().map(|a| match a {
                    A::Default => "default",
                    A::Ask => "ask",
                    A::AlwaysAllow => "allow",
                    A::AlwaysDeny => "deny",
                    A::Unknown(_) => "unknown",
                })
            }
            _ => None,
        }
    }

    /// macOS: the type list, read with no content and no prompt (the "detect first" step of
    /// plan 3.9). `None` elsewhere.
    pub fn types_without_content(&self) -> Option<Vec<String>> {
        match self {
            #[cfg(target_os = "macos")]
            Backend::Mac(m) => m.types(),
            _ => None,
        }
    }

    /// Plan decision 8: read the content of Apple Universal Clipboard clips only when they
    /// are passed on.
    pub fn set_forward_remote(&mut self, _on: bool) {
        #[cfg(target_os = "macos")]
        if let Backend::Mac(m) = self {
            m.set_read_remote_content(_on);
        }
    }

    /// Test profiles: put text on the clipboard as another app would (no marker). Refused on
    /// a user clipboard by construction (see the module rule).
    pub fn debug_copy(&mut self, text: &str) -> Result<u64, ClipboardError> {
        match self {
            Backend::Fake(f) => Ok(f.copy_text(text)),
            #[cfg(windows)]
            Backend::Win(w) => {
                let mut b: Vec<u8> = text
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .flat_map(u16::to_le_bytes)
                    .collect();
                b.shrink_to_fit();
                w.write_raw(&[(uc_clip_win::RawFormat::Id(13), b)])
            }
            #[cfg(target_os = "macos")]
            Backend::Mac(m) => m.write_foreign(&[(uc_clip_mac::types::TEXT, text.as_bytes())]),
            #[cfg(target_os = "linux")]
            Backend::Linux(l) => l.copy_raw(&[
                ("text/plain;charset=utf-8", text.as_bytes()),
                ("UTF8_STRING", text.as_bytes()),
            ]),
        }
    }

    /// Test profiles: the current text, if any.
    pub fn debug_text(&mut self) -> Option<String> {
        let snap = self.read().ok()?;
        snap.parts
            .into_iter()
            .find(|(f, _)| *f == Format::TextPlain)
            .map(|(_, b)| String::from_utf8_lossy(&b).replace("\r\n", "\n"))
    }
}

impl Clipboard for Backend {
    fn read(&mut self) -> Result<Snapshot, ClipboardError> {
        match self {
            Backend::Fake(f) => f.read(),
            #[cfg(windows)]
            Backend::Win(w) => w.read(),
            #[cfg(target_os = "macos")]
            Backend::Mac(m) => m.read(),
            #[cfg(target_os = "linux")]
            Backend::Linux(l) => l.read(),
        }
    }

    fn write(&mut self, parts: &[RawPart]) -> Result<u64, ClipboardError> {
        match self {
            Backend::Fake(f) => f.write(parts),
            #[cfg(windows)]
            Backend::Win(w) => w.write(parts),
            #[cfg(target_os = "macos")]
            Backend::Mac(m) => m.write(parts),
            #[cfg(target_os = "linux")]
            Backend::Linux(l) => l.write(parts),
        }
    }

    fn line_ending(&self) -> LineEnding {
        match self {
            Backend::Fake(f) => f.line_ending(),
            #[cfg(windows)]
            Backend::Win(w) => w.line_ending(),
            #[cfg(target_os = "macos")]
            Backend::Mac(m) => m.line_ending(),
            #[cfg(target_os = "linux")]
            Backend::Linux(l) => l.line_ending(),
        }
    }
}
