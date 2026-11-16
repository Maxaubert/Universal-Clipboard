//! File copies: the manifest, its path rules, NFC names and hashes (file sync spec 2.5 and 3).
//!
//! A file copy travels as a small offer (`FilesOffer`) and, when a peer wants it, a data
//! stream: a short head, the manifest, then the bytes of each file in manifest order. This
//! module holds the pure parts of that: the manifest and its checks. Walking, hashing and
//! writing files is the agent's job.
//!
//! Manifest rules (checked on both sides, before anything touches the disk):
//! - Paths are relative, `/`-separated and NFC. No `..` or `.`, no absolute or drive paths, no
//!   empty parts, no NUL, no backslash. At most [`MAX_NAME`] bytes per part, [`MAX_PATH`] per
//!   path, [`MAX_DEPTH`] parts and [`MAX_ENTRIES`] entries.
//! - Entries are sorted by path with no duplicates, and every parent folder is listed before
//!   its children. A folder has size 0, a zero hash and no exec bit.
//! - The encoded manifest is at most [`MANIFEST_MAX`] bytes.

use std::collections::{BTreeSet, HashSet};

use serde::{Deserialize, Serialize};
use unicode_normalization::{UnicodeNormalization, is_nfc};

use crate::hlc::Stamp;
use crate::ids::{ContentHash, FilesId};

/// Largest encoded manifest (16 MiB).
pub const MANIFEST_MAX: usize = 16 << 20;
/// Most entries (files and folders) in one copy.
pub const MAX_ENTRIES: usize = 100_000;
/// Deepest path, in parts.
pub const MAX_DEPTH: usize = 64;
/// Longest path part, in bytes.
pub const MAX_NAME: usize = 255;
/// Longest path, in bytes.
pub const MAX_PATH: usize = 4096;
/// Paths longer than this (in UTF-16 units, with the receive folder) are too long for Windows
/// Explorer. The copy is still placed; the note says so (spec 2.2.8).
pub const WINDOWS_PATH_MAX: usize = 259;
/// The receiver needs this much free space: the total plus 10 % (spec 2.2.2).
pub fn space_needed(total: u64) -> u64 {
    total.saturating_add(total / 10)
}

/// Stream error code: the transfer was superseded by a newer copy (spec 3).
pub const CODE_SUPERSEDED: u8 = 1;
/// Stream error code: the transfer was aborted (stop, failure, setting off).
pub const CODE_ABORTED: u8 = 2;

/// File types that also go on the clipboard as an image when the copy is one such file
/// (spec 2.2.5).
const IMAGE_EXTENSIONS: [&str; 4] = ["png", "jpg", "jpeg", "gif"];

/// A file or a folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EntryKind {
    /// A regular file.
    File,
    /// A folder (listed so that empty folders arrive too).
    Folder,
}

/// One entry of a manifest.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Entry {
    /// Relative, `/`-separated, NFC path. The first part is the top-level item's name.
    pub path: String,
    /// File or folder.
    pub kind: EntryKind,
    /// Size in bytes (0 for a folder).
    pub size: u64,
    /// BLAKE3 of the bytes (all zeros for a folder).
    pub hash: [u8; 32],
    /// The file is executable (set as mode `0o755` on macOS and Linux receivers).
    pub exec: bool,
}

impl Entry {
    /// A file entry.
    pub fn file(path: impl Into<String>, size: u64, hash: [u8; 32], exec: bool) -> Self {
        Self {
            path: path.into(),
            kind: EntryKind::File,
            size,
            hash,
            exec,
        }
    }

    /// A folder entry.
    pub fn folder(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            kind: EntryKind::Folder,
            size: 0,
            hash: [0; 32],
            exec: false,
        }
    }

    /// The number of parts in the path.
    pub fn depth(&self) -> usize {
        self.path.split('/').count()
    }
}

/// Why a manifest (or a path in it) is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestError {
    /// No entries.
    Empty,
    /// More than [`MAX_ENTRIES`] entries.
    TooManyEntries,
    /// A path deeper than [`MAX_DEPTH`].
    TooDeep,
    /// The encoded manifest is over [`MANIFEST_MAX`].
    TooLarge,
    /// A path breaks a rule (index of the entry).
    BadPath(usize),
    /// Entries are not sorted, or a path appears twice (index of the entry).
    Order(usize),
    /// An entry's parent folder is not listed before it (index of the entry).
    NoParent(usize),
    /// A folder with a size, hash or exec bit (index of the entry).
    BadFolder(usize),
    /// The sizes add up to more than `u64::MAX`.
    Overflow,
    /// The bytes do not match the expected manifest hash.
    HashMismatch,
    /// The bytes are not a manifest.
    Malformed,
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::Empty => f.write_str("no files"),
            ManifestError::TooManyEntries => write!(f, "more than {MAX_ENTRIES} entries"),
            ManifestError::TooDeep => write!(f, "a path is deeper than {MAX_DEPTH}"),
            ManifestError::TooLarge => f.write_str("manifest is too large"),
            ManifestError::BadPath(i) => write!(f, "entry {i}: bad path"),
            ManifestError::Order(i) => write!(f, "entry {i}: out of order or a duplicate"),
            ManifestError::NoParent(i) => write!(f, "entry {i}: parent folder is not listed"),
            ManifestError::BadFolder(i) => write!(f, "entry {i}: folder with data"),
            ManifestError::Overflow => f.write_str("sizes overflow"),
            ManifestError::HashMismatch => f.write_str("manifest does not match its hash"),
            ManifestError::Malformed => f.write_str("malformed manifest"),
        }
    }
}

impl std::error::Error for ManifestError {}

/// Is `path` a safe relative path under the rules in the module docs?
pub fn is_safe_path(path: &str) -> bool {
    if path.is_empty() || path.len() > MAX_PATH || !is_nfc(path) {
        return false;
    }
    if path.contains(['\0', '\\']) {
        return false;
    }
    let mut parts = 0;
    for (i, part) in path.split('/').enumerate() {
        parts += 1;
        if part.is_empty() || part == "." || part == ".." || part.len() > MAX_NAME {
            return false;
        }
        // A drive path ("C:" or "C:foo") as the first part.
        let b = part.as_bytes();
        if i == 0 && b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
            return false;
        }
    }
    parts <= MAX_DEPTH
}

/// Bring a name into NFC, the form every manifest uses.
pub fn nfc(s: &str) -> String {
    if is_nfc(s) {
        s.to_owned()
    } else {
        s.nfc().collect()
    }
}

/// The counts and hashes the engine needs from a manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FilesSummary {
    /// BLAKE3 of the encoded manifest (sent in the offer, checked on the stream).
    pub manifest_hash: [u8; 32],
    /// Content hash for echo layer 4.
    pub content: ContentHash,
    /// Total bytes of all files.
    pub total: u64,
    /// Number of files (folders not counted).
    pub count: u32,
}

/// The list of files and folders of one file copy. Always valid: it can only be made by
/// [`Manifest::new`] or [`Manifest::decode`], which check every rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    entries: Vec<Entry>,
    bytes: Vec<u8>,
    total: u64,
}

impl Manifest {
    /// Build a manifest from the sender's walk: names are brought into NFC, entries sorted,
    /// then every rule is checked. Top-level names must already be unique (see
    /// [`unique_name`]).
    pub fn new(mut entries: Vec<Entry>) -> Result<Manifest, ManifestError> {
        if entries.len() > MAX_ENTRIES {
            return Err(ManifestError::TooManyEntries);
        }
        for e in &mut entries {
            e.path = nfc(&e.path);
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Self::checked(entries)
    }

    /// Read a manifest from the stream and check it against the hash in the offer, then
    /// check every rule. The entries must already be in canonical form.
    pub fn decode(bytes: &[u8], expected: &[u8; 32]) -> Result<Manifest, ManifestError> {
        if bytes.len() > MANIFEST_MAX {
            return Err(ManifestError::TooLarge);
        }
        if blake3::hash(bytes).as_bytes() != expected {
            return Err(ManifestError::HashMismatch);
        }
        let entries: Vec<Entry> = match postcard::take_from_bytes(bytes) {
            Ok((v, [])) => v,
            _ => return Err(ManifestError::Malformed),
        };
        Self::checked(entries)
    }

    fn checked(entries: Vec<Entry>) -> Result<Manifest, ManifestError> {
        if entries.is_empty() {
            return Err(ManifestError::Empty);
        }
        if entries.len() > MAX_ENTRIES {
            return Err(ManifestError::TooManyEntries);
        }
        let mut folders: HashSet<&str> = HashSet::new();
        let mut total: u64 = 0;
        for (i, e) in entries.iter().enumerate() {
            if e.depth() > MAX_DEPTH {
                return Err(ManifestError::TooDeep);
            }
            if !is_safe_path(&e.path) {
                return Err(ManifestError::BadPath(i));
            }
            if i > 0 && entries[i - 1].path >= e.path {
                return Err(ManifestError::Order(i));
            }
            if let Some((parent, _)) = e.path.rsplit_once('/')
                && !folders.contains(parent)
            {
                return Err(ManifestError::NoParent(i));
            }
            match e.kind {
                EntryKind::Folder => {
                    if e.size != 0 || e.hash != [0; 32] || e.exec {
                        return Err(ManifestError::BadFolder(i));
                    }
                    folders.insert(&e.path);
                }
                EntryKind::File => {
                    total = total.checked_add(e.size).ok_or(ManifestError::Overflow)?;
                }
            }
        }
        let bytes = postcard::to_allocvec(&entries).map_err(|_| ManifestError::Malformed)?;
        if bytes.len() > MANIFEST_MAX {
            return Err(ManifestError::TooLarge);
        }
        Ok(Manifest {
            entries,
            bytes,
            total,
        })
    }

    /// The entries, sorted by path. This is also the order of the file bytes on the stream.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The encoded manifest, as sent on the stream.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// BLAKE3 of [`Manifest::bytes`].
    pub fn hash(&self) -> [u8; 32] {
        *blake3::hash(&self.bytes).as_bytes()
    }

    /// Content hash for echo layer 4: BLAKE3 over `uclip/files/1` and every entry's path,
    /// kind, size and hash. The exec bit is left out, because Windows cannot see it: a copy
    /// received there and copied again must still count as the same content.
    pub fn content_hash(&self) -> ContentHash {
        let mut h = blake3::Hasher::new();
        h.update(b"uclip/files/1");
        for e in &self.entries {
            h.update(&(e.path.len() as u64).to_le_bytes());
            h.update(e.path.as_bytes());
            h.update(&[match e.kind {
                EntryKind::File => 1,
                EntryKind::Folder => 2,
            }]);
            h.update(&e.size.to_le_bytes());
            h.update(&e.hash);
        }
        ContentHash(*h.finalize().as_bytes())
    }

    /// Total bytes of all files.
    pub fn total(&self) -> u64 {
        self.total
    }

    /// Number of files (folders not counted).
    pub fn file_count(&self) -> u32 {
        let n = self
            .entries
            .iter()
            .filter(|e| e.kind == EntryKind::File)
            .count();
        u32::try_from(n).unwrap_or(u32::MAX)
    }

    /// What the engine needs to know.
    pub fn summary(&self) -> FilesSummary {
        FilesSummary {
            manifest_hash: self.hash(),
            content: self.content_hash(),
            total: self.total,
            count: self.file_count(),
        }
    }

    /// The top-level items (entries with one path part), in manifest order.
    pub fn top_level(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| !e.path.contains('/'))
    }

    /// If the copy is one image file (png, jpg, gif) of at most `max` bytes, that entry
    /// (spec 2.2.5: the image also goes on the clipboard).
    pub fn single_image(&self, max: u64) -> Option<&Entry> {
        let [e] = self.entries.as_slice() else {
            return None;
        };
        let ext = e.path.rsplit_once('.')?.1.to_ascii_lowercase();
        (e.kind == EntryKind::File && e.size <= max && IMAGE_EXTENSIONS.contains(&ext.as_str()))
            .then_some(e)
    }

    /// The local relative path of every entry on this receiver, in entry order, `/`-separated.
    /// Names are made safe for `rules` and duplicates within a folder get ` (2)`, ` (3)`, ...
    pub fn local_paths(&self, rules: LocalRules) -> Vec<String> {
        // Local path of each folder, and the names used in each local folder.
        let mut folder_map: std::collections::HashMap<&str, String> =
            std::collections::HashMap::new();
        let mut used: std::collections::HashMap<String, BTreeSet<String>> =
            std::collections::HashMap::new();
        let mut out = Vec::with_capacity(self.entries.len());
        for e in &self.entries {
            let (parent, name) = match e.path.rsplit_once('/') {
                Some((p, n)) => (Some(p), n),
                None => (None, e.path.as_str()),
            };
            let local_parent = parent
                .and_then(|p| folder_map.get(p).cloned())
                .unwrap_or_default();
            let name = if rules.windows {
                windows_name(name)
            } else {
                name.to_owned()
            };
            let taken = used.entry(local_parent.clone()).or_default();
            let name = unique_name(&name, e.kind == EntryKind::File, rules.fold_case, taken);
            let local = if local_parent.is_empty() {
                name
            } else {
                format!("{local_parent}/{name}")
            };
            if e.kind == EntryKind::Folder {
                folder_map.insert(&e.path, local.clone());
            }
            out.push(local);
        }
        out
    }
}

/// How names must look on the receiving device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalRules {
    /// Windows names: `< > : " | ? * \` and control characters become `_`, trailing dots and
    /// spaces are trimmed, reserved names (`CON`, `NUL`, `COM1`, ...) get `_`.
    pub windows: bool,
    /// The file system ignores case (Windows, macOS by default): names that differ only in
    /// case are duplicates.
    pub fold_case: bool,
}

impl LocalRules {
    /// The rules of the OS this build runs on.
    pub fn this_os() -> Self {
        Self {
            windows: cfg!(windows),
            fold_case: cfg!(any(windows, target_os = "macos")),
        }
    }
}

const WINDOWS_RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Make one name safe for Windows (see [`LocalRules::windows`]).
pub fn windows_name(name: &str) -> String {
    let mapped: String = name
        .chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\\') || c < ' ' {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = mapped.trim_end_matches(['.', ' ']);
    let mut s = if trimmed.is_empty() {
        "_".to_owned()
    } else {
        trimmed.to_owned()
    };
    // "CON", "con.txt" and "Com1.tar.gz" are all reserved: the part before the first dot.
    let stem_len = s.find('.').unwrap_or(s.len());
    if WINDOWS_RESERVED
        .iter()
        .any(|r| r.eq_ignore_ascii_case(&s[..stem_len]))
    {
        s.insert(stem_len, '_');
    }
    s
}

/// `name`, or `name (2)`, `name (3)`, ... (before the extension for a file), whichever is not
/// in `taken` yet. The chosen name is added to `taken`. With `fold_case`, names that differ
/// only in case count as the same. The sender uses it for top-level names, the receiver for
/// every folder.
pub fn unique_name(
    name: &str,
    is_file: bool,
    fold_case: bool,
    taken: &mut BTreeSet<String>,
) -> String {
    let key = |s: &str| {
        if fold_case {
            s.to_lowercase()
        } else {
            s.to_owned()
        }
    };
    if taken.insert(key(name)) {
        return name.to_owned();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if is_file && i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 2u64.. {
        let candidate = format!("{stem} ({n}){ext}");
        if taken.insert(key(&candidate)) {
            return candidate;
        }
    }
    unreachable!("the counter never runs out")
}

/// The ID of a file copy: BLAKE3 over `uclip/files-id/1`, the manifest hash and the stamp.
pub fn files_id(manifest_hash: &[u8; 32], stamp: &Stamp) -> FilesId {
    let mut h = blake3::Hasher::new();
    h.update(b"uclip/files-id/1");
    h.update(manifest_hash);
    h.update(&stamp.wall_ms.to_le_bytes());
    h.update(&stamp.counter.to_le_bytes());
    h.update(&stamp.device.0);
    FilesId(*h.finalize().as_bytes())
}

/// Version byte of the stream head.
const STREAM_VERSION: u8 = 1;
/// Length of the stream head: version, copy ID, manifest length (u32, big endian).
pub const STREAM_HEAD_LEN: usize = 1 + 32 + 4;

/// The start of a data stream: the head, then the manifest (spec 3). The file bytes follow.
pub fn stream_start(id: &FilesId, manifest: &Manifest) -> Vec<u8> {
    let m = manifest.bytes();
    let mut out = Vec::with_capacity(STREAM_HEAD_LEN + m.len());
    out.push(STREAM_VERSION);
    out.extend_from_slice(&id.0);
    // The manifest is at most MANIFEST_MAX, far below u32::MAX.
    out.extend_from_slice(&(m.len() as u32).to_be_bytes());
    out.extend_from_slice(m);
    out
}

/// Read a stream head: the copy ID and the manifest length that follows. The length is
/// checked against [`MANIFEST_MAX`] before the caller allocates anything.
pub fn parse_stream_head(head: &[u8; STREAM_HEAD_LEN]) -> Result<(FilesId, usize), ManifestError> {
    if head[0] != STREAM_VERSION {
        return Err(ManifestError::Malformed);
    }
    let mut id = [0u8; 32];
    id.copy_from_slice(&head[1..33]);
    let len = u32::from_be_bytes([head[33], head[34], head[35], head[36]]) as usize;
    if len > MANIFEST_MAX {
        return Err(ManifestError::TooLarge);
    }
    Ok((FilesId(id), len))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn h(n: u8) -> [u8; 32] {
        [n; 32]
    }

    fn sample() -> Manifest {
        Manifest::new(vec![
            Entry::file("photos/b.jpg", 20, h(2), false),
            Entry::folder("photos"),
            Entry::file("photos/a.jpg", 10, h(1), false),
            Entry::folder("photos/empty"),
            Entry::file("run.sh", 5, h(3), true),
        ])
        .unwrap()
    }

    #[test]
    fn new_sorts_and_sums() {
        let m = sample();
        let paths: Vec<&str> = m.entries().iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "photos",
                "photos/a.jpg",
                "photos/b.jpg",
                "photos/empty",
                "run.sh"
            ]
        );
        assert_eq!(m.total(), 35);
        assert_eq!(m.file_count(), 3);
        let top: Vec<&str> = m.top_level().map(|e| e.path.as_str()).collect();
        assert_eq!(top, ["photos", "run.sh"]);
    }

    #[test]
    fn decode_round_trips_and_checks_the_hash() {
        let m = sample();
        let back = Manifest::decode(m.bytes(), &m.hash()).unwrap();
        assert_eq!(back, m);
        assert_eq!(
            Manifest::decode(m.bytes(), &h(0)),
            Err(ManifestError::HashMismatch)
        );
        let mut extra = m.bytes().to_vec();
        extra.push(0);
        let hash = *blake3::hash(&extra).as_bytes();
        assert_eq!(
            Manifest::decode(&extra, &hash),
            Err(ManifestError::Malformed)
        );
    }

    #[test]
    fn nfc_names_hash_the_same_on_every_os() {
        // "é" composed (Windows, Linux) and decomposed (older macOS file systems).
        let a = Manifest::new(vec![Entry::file("caf\u{e9}.txt", 1, h(1), false)]).unwrap();
        let b = Manifest::new(vec![Entry::file("cafe\u{301}.txt", 1, h(1), false)]).unwrap();
        assert_eq!(a.hash(), b.hash());
        assert_eq!(a.content_hash(), b.content_hash());
        assert_eq!(a.entries()[0].path, "caf\u{e9}.txt");
    }

    #[test]
    fn content_hash_ignores_the_exec_bit_but_not_names_or_data() {
        let base = Manifest::new(vec![Entry::file("a", 1, h(1), false)]).unwrap();
        let exec = Manifest::new(vec![Entry::file("a", 1, h(1), true)]).unwrap();
        let renamed = Manifest::new(vec![Entry::file("b", 1, h(1), false)]).unwrap();
        let changed = Manifest::new(vec![Entry::file("a", 1, h(2), false)]).unwrap();
        assert_eq!(base.content_hash(), exec.content_hash());
        assert_ne!(base.hash(), exec.hash());
        assert_ne!(base.content_hash(), renamed.content_hash());
        assert_ne!(base.content_hash(), changed.content_hash());
    }

    #[test]
    fn hostile_paths_are_refused() {
        for bad in [
            "",
            "..",
            "a/../b",
            "./a",
            "/etc/passwd",
            "a//b",
            "a/",
            "C:",
            "c:\\x",
            "C:foo",
            "a\\..\\b",
            "nul\0byte",
            "cafe\u{301}",
        ] {
            assert!(!is_safe_path(bad), "{bad:?} must be refused");
            let r = Manifest::decode_unchecked_for_test(vec![Entry::file(bad, 1, h(1), false)]);
            assert!(r.is_err(), "{bad:?} must be refused in a manifest");
        }
        assert!(!is_safe_path(&"a".repeat(MAX_NAME + 1)));
        assert!(is_safe_path(&"a".repeat(MAX_NAME)));
        let deep = vec!["d"; MAX_DEPTH + 1].join("/");
        assert!(!is_safe_path(&deep));
        let long = vec!["x".repeat(200); 21].join("/");
        assert!(long.len() > MAX_PATH && !is_safe_path(&long));
        assert!(is_safe_path("a b/c.d/ é"));
    }

    #[test]
    fn hostile_manifests_are_refused() {
        use ManifestError as E;
        let enc = |entries: &Vec<Entry>| {
            let b = postcard::to_allocvec(entries).unwrap();
            let hash = *blake3::hash(&b).as_bytes();
            Manifest::decode(&b, &hash)
        };
        assert_eq!(enc(&vec![]), Err(E::Empty));
        // Child without its folder, folder after its child, a file used as a folder.
        assert_eq!(
            enc(&vec![Entry::file("a/b", 1, h(1), false)]),
            Err(E::NoParent(0))
        );
        assert_eq!(
            enc(&vec![
                Entry::file("a", 1, h(1), false),
                Entry::file("a/b", 1, h(1), false)
            ]),
            Err(E::NoParent(1))
        );
        // Duplicates and unsorted entries.
        assert_eq!(
            enc(&vec![
                Entry::file("a", 1, h(1), false),
                Entry::file("a", 1, h(1), false)
            ]),
            Err(E::Order(1))
        );
        assert_eq!(
            enc(&vec![
                Entry::file("b", 1, h(1), false),
                Entry::file("a", 1, h(1), false)
            ]),
            Err(E::Order(1))
        );
        // A folder that claims data.
        let mut f = Entry::folder("d");
        f.size = 9;
        assert_eq!(enc(&vec![f]), Err(E::BadFolder(0)));
        // Sizes that overflow.
        assert_eq!(
            enc(&vec![
                Entry::file("a", u64::MAX, h(1), false),
                Entry::file("b", 1, h(1), false)
            ]),
            Err(E::Overflow)
        );
        // A traversal path.
        assert_eq!(
            enc(&vec![Entry::file("../x", 1, h(1), false)]),
            Err(E::BadPath(0))
        );
        // Too many entries and too large.
        let many: Vec<Entry> = (0..=MAX_ENTRIES)
            .map(|i| Entry::file(format!("{i:06}"), 0, h(0), false))
            .collect();
        assert_eq!(enc(&many), Err(E::TooManyEntries));
        assert_eq!(
            Manifest::decode(&vec![0; MANIFEST_MAX + 1], &h(0)),
            Err(E::TooLarge)
        );
        // Garbage.
        let junk = [0xff, 0xff, 0xff, 0xff, 0x0f];
        assert_eq!(
            Manifest::decode(&junk, blake3::hash(&junk).as_bytes()),
            Err(E::Malformed)
        );
    }

    #[test]
    fn deep_paths_are_refused_with_too_deep() {
        let mut entries = Vec::new();
        let mut p = String::from("d");
        for _ in 0..MAX_DEPTH {
            entries.push(Entry::folder(p.clone()));
            p.push_str("/d");
        }
        entries.push(Entry::folder(p));
        assert_eq!(Manifest::new(entries), Err(ManifestError::TooDeep));
    }

    #[test]
    fn windows_names() {
        assert_eq!(windows_name("a:b?.txt"), "a_b_.txt");
        assert_eq!(windows_name("x\u{1}y"), "x_y");
        assert_eq!(windows_name("trail. . "), "trail");
        assert_eq!(windows_name("..."), "_");
        assert_eq!(windows_name("CON"), "CON_");
        assert_eq!(windows_name("nul.txt"), "nul_.txt");
        assert_eq!(windows_name("Com1.tar.gz"), "Com1_.tar.gz");
        assert_eq!(windows_name("CONSOLE.txt"), "CONSOLE.txt");
        assert_eq!(windows_name("back\\slash"), "back_slash");
    }

    #[test]
    fn unique_names_get_a_number() {
        let mut taken = BTreeSet::new();
        assert_eq!(unique_name("a.txt", true, true, &mut taken), "a.txt");
        assert_eq!(unique_name("A.TXT", true, true, &mut taken), "A (2).TXT");
        assert_eq!(unique_name("a.txt", true, true, &mut taken), "a (3).txt");
        assert_eq!(unique_name("dir.v2", false, true, &mut taken), "dir.v2");
        assert_eq!(unique_name("dir.v2", false, true, &mut taken), "dir.v2 (2)");
        assert_eq!(unique_name(".bashrc", true, true, &mut taken), ".bashrc");
        assert_eq!(
            unique_name(".bashrc", true, true, &mut taken),
            ".bashrc (2)"
        );
        let mut cs = BTreeSet::new();
        assert_eq!(unique_name("a", true, false, &mut cs), "a");
        assert_eq!(unique_name("A", true, false, &mut cs), "A");
    }

    #[test]
    fn local_paths_for_windows_rename_folders_and_their_children() {
        let m = Manifest::new(vec![
            Entry::folder("Dir"),
            Entry::file("Dir/x", 1, h(1), false),
            Entry::folder("dir"),
            Entry::file("dir/y:z", 1, h(1), false),
            Entry::file("CON", 1, h(1), false),
        ])
        .unwrap();
        let win = m.local_paths(LocalRules {
            windows: true,
            fold_case: true,
        });
        assert_eq!(win, ["CON_", "Dir", "Dir/x", "dir (2)", "dir (2)/y_z"]);
        let linux = m.local_paths(LocalRules {
            windows: false,
            fold_case: false,
        });
        assert_eq!(linux, ["CON", "Dir", "Dir/x", "dir", "dir/y:z"]);
    }

    #[test]
    fn single_image_rule() {
        let one = |p: &str, size| Manifest::new(vec![Entry::file(p, size, h(1), false)]).unwrap();
        assert!(one("shot.PNG", 10).single_image(10).is_some());
        assert!(one("a.jpeg", 10).single_image(10).is_some());
        assert!(one("a.gif", 11).single_image(10).is_none());
        assert!(one("a.svg", 1).single_image(10).is_none());
        assert!(one("png", 1).single_image(10).is_none());
        assert!(sample().single_image(u64::MAX).is_none());
    }

    #[test]
    fn files_id_depends_on_manifest_and_stamp() {
        let s = |c| Stamp {
            wall_ms: 1,
            counter: c,
            device: crate::DeviceId([1; 32]),
        };
        assert_eq!(files_id(&h(1), &s(0)), files_id(&h(1), &s(0)));
        assert_ne!(files_id(&h(1), &s(0)), files_id(&h(1), &s(1)));
        assert_ne!(files_id(&h(1), &s(0)), files_id(&h(2), &s(0)));
        assert_eq!(files_id(&h(1), &s(0)).short().len(), 8);
    }

    #[test]
    fn stream_head_round_trip_and_limits() {
        let m = sample();
        let id = FilesId([7; 32]);
        let start = stream_start(&id, &m);
        let head: [u8; STREAM_HEAD_LEN] = start[..STREAM_HEAD_LEN].try_into().unwrap();
        let (got, len) = parse_stream_head(&head).unwrap();
        assert_eq!(got, id);
        assert_eq!(&start[STREAM_HEAD_LEN..], m.bytes());
        assert_eq!(len, m.bytes().len());
        let mut bad = head;
        bad[33..].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(parse_stream_head(&bad), Err(ManifestError::TooLarge));
        bad[0] = 9;
        assert_eq!(parse_stream_head(&bad), Err(ManifestError::Malformed));
    }

    #[test]
    fn space_needed_adds_ten_percent() {
        assert_eq!(space_needed(1000), 1100);
        assert_eq!(space_needed(u64::MAX), u64::MAX);
    }

    impl Manifest {
        /// Encode raw entries (no normalizing or sorting) and decode them, as a receiver would.
        fn decode_unchecked_for_test(entries: Vec<Entry>) -> Result<Manifest, ManifestError> {
            let b = postcard::to_allocvec(&entries).unwrap();
            Manifest::decode(&b, blake3::hash(&b).as_bytes())
        }
    }

    proptest! {
        #[test]
        fn decode_never_panics(b in proptest::collection::vec(any::<u8>(), 0..512)) {
            let hash = *blake3::hash(&b).as_bytes();
            let _ = Manifest::decode(&b, &hash);
        }

        #[test]
        fn local_paths_are_safe_and_unique(names in proptest::collection::vec("[a-zA-Z.:<> ?*]{1,6}", 1..12)) {
            let mut taken = BTreeSet::new();
            let entries: Vec<Entry> = names
                .iter()
                .filter(|n| is_safe_path(n))
                .map(|n| unique_name(n, true, false, &mut taken))
                .map(|n| Entry::file(n, 1, h(1), false))
                .collect();
            prop_assume!(!entries.is_empty());
            let m = Manifest::new(entries).unwrap();
            let rules = LocalRules { windows: true, fold_case: true };
            let local = m.local_paths(rules);
            let folded: BTreeSet<String> = local.iter().map(|s| s.to_lowercase()).collect();
            prop_assert_eq!(folded.len(), local.len());
            for p in &local {
                prop_assert!(is_safe_path(p));
                prop_assert!(!p.contains(['<', '>', ':', '?', '*']));
                prop_assert!(!p.ends_with(['.', ' ']));
            }
        }
    }
}
