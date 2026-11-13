//! Clipboard trait and a fake clipboard for tests.
//!
//! This crate is the boundary between the sync logic (`uc-core`) and the OS clipboard.
//! Each OS backend (`uc-clip-win`, `uc-clip-mac`, `uc-clip-linux`) implements [`Clipboard`].
//! Tests use [`FakeClipboard`], so no test ever touches a real clipboard.

use serde::{Deserialize, Serialize};

mod fake;

pub use fake::FakeClipboard;

/// The clip formats that are synced (plan 3.8). Everything else on the clipboard is ignored.
///
/// The order of the variants is the canonical part order on the wire. Do not reorder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Format {
    /// `text/plain`, UTF-8.
    TextPlain,
    /// `text/html`, UTF-8 fragment (on Windows: without the `CF_HTML` header).
    TextHtml,
    /// `image/png`.
    ImagePng,
}

impl Format {
    /// The MIME name, for logs and diagnostics.
    pub fn mime(self) -> &'static str {
        match self {
            Format::TextPlain => "text/plain",
            Format::TextHtml => "text/html",
            Format::ImagePng => "image/png",
        }
    }
}

/// One format and its bytes, as read from or written to a clipboard.
pub type RawPart = (Format, Vec<u8>);

/// The line ending that the OS clipboard uses for text (plan 3.8: the wire always uses LF).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LineEnding {
    /// macOS, Linux, phones.
    #[default]
    Lf,
    /// Windows.
    CrLf,
}

/// What a backend read from the clipboard after a change event.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// The clipboard's change counter at the time of the read (Windows sequence number, macOS
    /// `changeCount`, Linux: a counter of selection-owner changes). Echo layer 2 compares it.
    pub seq: u64,
    /// The synced formats that were present, in any order.
    pub parts: Vec<RawPart>,
    /// Our private origin marker format was present (echo layer 1).
    pub has_marker: bool,
    /// The source marked the copy as private, or a private marker could not be read (plan 3.10).
    pub private: bool,
    /// The clipboard holds a file or folder reference. The whole clip is skipped (plan 3.8).
    pub has_files: bool,
}

/// An error from a clipboard backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardError {
    /// Another program holds the clipboard open. The caller may retry later.
    Busy,
    /// The OS refused access (for example a privacy setting).
    Denied,
    /// Any other failure, with a short code for the diagnostic report (never content).
    Other(String),
}

impl std::fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClipboardError::Busy => f.write_str("clipboard is busy"),
            ClipboardError::Denied => f.write_str("clipboard access denied"),
            ClipboardError::Other(code) => write!(f, "clipboard error: {code}"),
        }
    }
}

impl std::error::Error for ClipboardError {}

/// One OS clipboard.
///
/// The agent calls [`Clipboard::read`] after a change event (after the settle time).
/// [`Clipboard::write`] replaces the whole clipboard with the given parts **and adds the
/// private origin marker**, in one step, and returns the change counter of that write.
pub trait Clipboard {
    /// Read the current clipboard.
    fn read(&mut self) -> Result<Snapshot, ClipboardError>;

    /// Replace the clipboard with `parts` plus the origin marker. Text arrives in the
    /// line ending of [`Clipboard::line_ending`]. Returns the change counter after the write.
    fn write(&mut self, parts: &[RawPart]) -> Result<u64, ClipboardError>;

    /// The line ending that this clipboard uses for text.
    fn line_ending(&self) -> LineEnding {
        LineEnding::Lf
    }
}

impl<C: Clipboard + ?Sized> Clipboard for &mut C {
    fn read(&mut self) -> Result<Snapshot, ClipboardError> {
        (**self).read()
    }
    fn write(&mut self, parts: &[RawPart]) -> Result<u64, ClipboardError> {
        (**self).write(parts)
    }
    fn line_ending(&self) -> LineEnding {
        (**self).line_ending()
    }
}
