//! An in-memory clipboard for tests. It never touches the OS.

use std::path::PathBuf;

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

    /// A file manager copies a file reference that is not a real path (a virtual file), with
    /// a file name text next to it.
    pub fn copy_virtual_files(&mut self, name_text: &str) -> u64 {
        let seq = self.set(
            vec![(crate::Format::TextPlain, name_text.as_bytes().to_vec())],
            false,
            false,
            true,
        );
        self.state.files_virtual = true;
        seq
    }

    /// A file manager copies `paths`, with a file name text next to them (as Finder does).
    pub fn copy_paths(&mut self, paths: &[PathBuf]) -> u64 {
        let name = paths
            .first()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let seq = self.set(
            vec![(crate::Format::TextPlain, name.into_bytes())],
            false,
            false,
            true,
        );
        self.state.files = paths.to_vec();
        seq
    }

    /// Set the whole state from outside, as any program could. Returns the new counter.
    pub fn set(&mut self, parts: Vec<RawPart>, marker: bool, private: bool, files: bool) -> u64 {
        self.state.seq += 1;
        self.state.parts = parts;
        self.state.has_marker = marker;
        self.state.private = private;
        self.state.has_files = files;
        self.state.files = Vec::new();
        self.state.files_virtual = false;
        self.state.remote = false;
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

    /// The file paths on the clipboard (empty if it holds no file copy).
    pub fn files(&self) -> &[PathBuf] {
        &self.state.files
    }

    /// How many times [`Clipboard::write`] or [`Clipboard::write_files`] was called.
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

    fn write_files(
        &mut self,
        paths: &[PathBuf],
        image: Option<&[u8]>,
    ) -> Result<u64, ClipboardError> {
        crate::check_file_paths(paths)?;
        self.writes += 1;
        let parts = image
            .map(|png| vec![(crate::Format::ImagePng, png.to_vec())])
            .unwrap_or_default();
        let seq = self.set(parts, true, false, true);
        self.state.files = paths.to_vec();
        Ok(seq)
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
        c.copy_virtual_files("report.pdf");
        let s = c.read().unwrap();
        assert!(s.has_files && s.files_virtual && s.files.is_empty() && !s.private);
        c.copy_paths(&[PathBuf::from("/home/max/report.pdf")]);
        let s = c.read().unwrap();
        assert!(s.has_files && !s.files_virtual && !s.has_marker);
        assert_eq!(s.files, vec![PathBuf::from("/home/max/report.pdf")]);
        assert_eq!(c.text().as_deref(), Some("report.pdf"));
        // A later text copy clears the file list.
        c.copy_text("x");
        assert!(c.read().unwrap().files.is_empty());
    }

    #[test]
    fn write_files_carries_marker_and_optional_image() {
        let mut c = FakeClipboard::new();
        let abs = std::env::temp_dir().join("a.png");
        let seq = c
            .write_files(std::slice::from_ref(&abs), Some(b"png"))
            .unwrap();
        let s = c.read().unwrap();
        assert_eq!(s.seq, seq);
        assert!(s.has_marker && s.has_files);
        assert_eq!(s.files, vec![abs]);
        assert_eq!(c.part(Format::ImagePng), Some(&b"png"[..]));
        assert_eq!(c.write_count(), 1);
    }

    #[test]
    fn write_files_refuses_bad_paths_without_touching_the_clipboard() {
        let mut c = FakeClipboard::new();
        let seq = c.copy_text("keep");
        assert!(c.write_files(&[], None).is_err());
        assert!(c.write_files(&[PathBuf::from("relative/x")], None).is_err());
        assert_eq!(c.read().unwrap().seq, seq);
        assert_eq!(c.write_count(), 0);
    }

    #[test]
    fn default_write_files_is_unsupported() {
        struct Plain;
        impl Clipboard for Plain {
            fn read(&mut self) -> Result<Snapshot, ClipboardError> {
                Ok(Snapshot::default())
            }
            fn write(&mut self, _: &[RawPart]) -> Result<u64, ClipboardError> {
                Ok(0)
            }
        }
        let abs = std::env::temp_dir();
        assert_eq!(
            Plain.write_files(&[abs], None),
            Err(ClipboardError::Unsupported)
        );
    }
}
