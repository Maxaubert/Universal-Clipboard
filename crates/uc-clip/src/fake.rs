//! An in-memory clipboard for tests. It never touches the OS.

use crate::{Clipboard, ClipboardError, LineEnding, RawPart, Snapshot};

/// An in-memory clipboard that behaves like an OS clipboard: every change bumps a counter,
/// our own writes carry the origin marker, and other programs (the "user", a clipboard
/// manager) can change it from outside.
#[derive(Clone, Debug, Default)]
pub struct FakeClipboard {
    state: Snapshot,
    line_ending: LineEnding,
    writes: u64,
    failing_reads: u32,
}

impl FakeClipboard {
    /// An empty clipboard with LF line endings.
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty clipboard that behaves like Windows (CRLF text).
    pub fn windows() -> Self {
        Self {
            line_ending: LineEnding::CrLf,
            ..Self::default()
        }
    }

    /// The user (or another app) copies `parts`. No marker. Returns the new counter.
    pub fn copy(&mut self, parts: Vec<RawPart>) -> u64 {
        self.set(parts, false, false, false)
    }

    /// The user copies text.
    pub fn copy_text(&mut self, text: &str) -> u64 {
        self.copy(vec![(crate::Format::TextPlain, text.as_bytes().to_vec())])
    }

    /// A password manager copies `parts` with a private marker.
    pub fn copy_private(&mut self, parts: Vec<RawPart>) -> u64 {
        self.set(parts, false, true, false)
    }

    /// A file manager copies a file reference, with a file name text next to it.
    pub fn copy_files(&mut self, name_text: &str) -> u64 {
        self.set(
            vec![(crate::Format::TextPlain, name_text.as_bytes().to_vec())],
            false,
            false,
            true,
        )
    }

    /// Set the whole state from outside, as any program could. Returns the new counter.
    pub fn set(&mut self, parts: Vec<RawPart>, marker: bool, private: bool, files: bool) -> u64 {
        self.state.seq += 1;
        self.state.parts = parts;
        self.state.has_marker = marker;
        self.state.private = private;
        self.state.has_files = files;
        self.state.seq
    }

    /// The next `n` reads fail with [`ClipboardError::Busy`].
    pub fn fail_reads(&mut self, n: u32) {
        self.failing_reads = n;
    }

    /// The current state, for assertions.
    pub fn peek(&self) -> &Snapshot {
        &self.state
    }

    /// The current text part, if any.
    pub fn text(&self) -> Option<String> {
        self.part(crate::Format::TextPlain)
            .map(|b| String::from_utf8_lossy(b).into_owned())
    }

    /// The bytes of one format, if present.
    pub fn part(&self, format: crate::Format) -> Option<&[u8]> {
        self.state
            .parts
            .iter()
            .find(|(f, _)| *f == format)
            .map(|(_, b)| b.as_slice())
    }

    /// How many times [`Clipboard::write`] was called.
    pub fn write_count(&self) -> u64 {
        self.writes
    }
}

impl Clipboard for FakeClipboard {
    fn read(&mut self) -> Result<Snapshot, ClipboardError> {
        if self.failing_reads > 0 {
            self.failing_reads -= 1;
            return Err(ClipboardError::Busy);
        }
        Ok(self.state.clone())
    }

    fn write(&mut self, parts: &[RawPart]) -> Result<u64, ClipboardError> {
        self.writes += 1;
        Ok(self.set(parts.to_vec(), true, false, false))
    }

    fn line_ending(&self) -> LineEnding {
        self.line_ending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Format;

    #[test]
    fn user_copy_has_no_marker_and_bumps_seq() {
        let mut c = FakeClipboard::new();
        let s1 = c.copy_text("a");
        let snap = c.read().unwrap();
        assert_eq!(snap.seq, s1);
        assert!(!snap.has_marker);
        assert_eq!(c.text().as_deref(), Some("a"));
    }

    #[test]
    fn our_write_carries_marker() {
        let mut c = FakeClipboard::new();
        let seq = c.write(&[(Format::TextPlain, b"x".to_vec())]).unwrap();
        let snap = c.read().unwrap();
        assert!(snap.has_marker);
        assert_eq!(snap.seq, seq);
        assert_eq!(c.write_count(), 1);
    }

    #[test]
    fn failing_reads_then_recover() {
        let mut c = FakeClipboard::windows();
        assert_eq!(c.line_ending(), LineEnding::CrLf);
        c.fail_reads(2);
        assert_eq!(c.read(), Err(ClipboardError::Busy));
        assert_eq!(c.read(), Err(ClipboardError::Busy));
        assert!(c.read().is_ok());
    }

    #[test]
    fn private_and_files_flags() {
        let mut c = FakeClipboard::new();
        c.copy_private(vec![(Format::TextPlain, b"pw".to_vec())]);
        assert!(c.read().unwrap().private);
        c.copy_files("report.pdf");
        let s = c.read().unwrap();
        assert!(s.has_files && !s.private);
    }
}
