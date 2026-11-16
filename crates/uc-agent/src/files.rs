//! File copies in the agent (file sync spec 2, 3 and 5): walk and hash on the copying device,
//! the data stream of a send, the receive folder, and the files folder rules.
//!
//! The engine (`uc_core::sync`) decides what happens; this module does the file and stream
//! work. The walk and the hash are blocking and run in `spawn_blocking`; sends and receives
//! are tokio tasks. None of them touches the agent's state: each reports back to the main
//! loop, which owns the engine.
//!
//! Nothing here logs or returns a file name: notes and logs hold counts and sizes only.
//!
//! The files folder ([`crate::profile::Profile::files_dir`]) holds one folder per received
//! copy, named by the copy's short ID. A copy is written to `<id>.part` first and renamed when
//! every file is in and checked. The last [`KEEP`] copies are kept; at agent start `.part`
//! leftovers and folders older than [`MAX_AGE`] go.

use std::collections::{BTreeSet, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::watch;
use uc_core::files::{
    self as core_files, Entry, EntryKind, LocalRules, MAX_DEPTH, MAX_ENTRIES, Manifest,
    STREAM_HEAD_LEN,
};
use uc_core::{DeviceId, FilesId};
use uc_net::{Link, RecvStream, STREAM_ABORTED, SendStream, StreamError};

/// Files of this size and up are hashed with a memory map on all cores (spec 2.1.3).
pub const MMAP_MIN: u64 = 16 << 20;
/// Bytes per read and write on a data stream.
const CHUNK: usize = 1 << 20;
/// Received copies kept in the files folder (spec 2.5).
pub const KEEP: usize = 3;
/// Folders older than this are deleted at agent start (spec 2.5).
pub const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);
/// How long a rename of the finished folder is tried again on Windows, where Defender or the
/// indexer may hold the new files for a moment (spec 2.2.3).
const RENAME_RETRY: Duration = Duration::from_secs(2);
/// How long the head of an incoming data stream may take.
pub const HEAD_TIMEOUT: Duration = Duration::from_secs(10);

/// Progress of one walk, hash or transfer, shared with the main loop for the `transfer` event.
#[derive(Debug, Default)]
pub struct Progress {
    /// Bytes done.
    pub done: AtomicU64,
    /// Files done (walked, hashed or moved).
    pub files: AtomicU64,
}

impl Progress {
    fn add(&self, bytes: u64) {
        self.done.fetch_add(bytes, Ordering::Relaxed);
    }

    fn file_done(&self) {
        self.files.fetch_add(1, Ordering::Relaxed);
    }

    /// (bytes, files) done so far.
    pub fn get(&self) -> (u64, u64) {
        (
            self.done.load(Ordering::Relaxed),
            self.files.load(Ordering::Relaxed),
        )
    }
}

// ------------------------------------------------------------------ walk

/// One file or folder found by the walk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// The manifest path: relative, `/`-separated, NFC, starting with the top-level name.
    pub rel: String,
    /// Where it is on this device.
    pub abs: PathBuf,
    /// File or folder.
    pub kind: EntryKind,
    /// Size in bytes (0 for a folder).
    pub size: u64,
    /// Executable (macOS and Linux).
    pub exec: bool,
}

/// What a walk found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Walked {
    /// Every file and folder.
    pub items: Vec<Item>,
    /// Total bytes of all files.
    pub total: u64,
    /// Number of files.
    pub files: u32,
    /// Symlinks and junctions inside folders that were left out (spec 2.4).
    pub links: u32,
}

/// Why a walk or a hash ended without a manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrepError {
    /// A cloud placeholder that is not on disk: the whole copy is skipped (spec 2.4).
    CloudOnly,
    /// Over the entry, depth or manifest limits.
    TooMany,
    /// An item could not be read (gone, no permission, locked).
    Unreadable,
    /// A file changed while it was hashed.
    Changed,
    /// A newer copy made it stale.
    Cancelled,
}

/// Is this item a cloud placeholder whose data is not on this device?
#[allow(unused_variables)]
fn cloud_only(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        uc_clip_win::files::is_cloud_placeholder(meta.file_attributes())
    }
    #[cfg(target_os = "macos")]
    {
        // SF_DATALESS: an iCloud Drive file that was evicted.
        use std::os::macos::fs::MetadataExt;
        const SF_DATALESS: u32 = 0x4000_0000;
        meta.st_flags() & SF_DATALESS != 0
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        false
    }
}

#[allow(unused_variables)]
fn is_exec(meta: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// A name as it goes into a manifest: NFC, and no backslash (a legal name on Linux and macOS,
/// a separator everywhere else).
fn wire_name(name: &std::ffi::OsStr) -> String {
    core_files::nfc(&name.to_string_lossy().replace('\\', "_"))
}

/// Walk the copied items: folders recursively (symlinks inside them left out), sizes summed.
/// Stops early on a cloud placeholder or when the limits are passed. `cancel` ends it.
pub fn walk(
    paths: &[PathBuf],
    cancel: &AtomicBool,
    progress: &Progress,
) -> Result<Walked, PrepError> {
    let mut out = Walked::default();
    let mut top = BTreeSet::new();
    for p in paths {
        // The copied item itself is followed if it is a link: the user picked it.
        let meta = std::fs::metadata(p).map_err(|_| PrepError::Unreadable)?;
        let name = p.file_name().map(wire_name).ok_or(PrepError::Unreadable)?;
        let name = core_files::unique_name(&name, meta.is_file(), false, &mut top);
        walk_item(p, name, &meta, 1, &mut out, cancel, progress)?;
    }
    Ok(out)
}

fn walk_item(
    abs: &Path,
    rel: String,
    meta: &std::fs::Metadata,
    depth: usize,
    out: &mut Walked,
    cancel: &AtomicBool,
    progress: &Progress,
) -> Result<(), PrepError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(PrepError::Cancelled);
    }
    if depth > MAX_DEPTH || out.items.len() >= MAX_ENTRIES {
        return Err(PrepError::TooMany);
    }
    if cloud_only(meta) {
        return Err(PrepError::CloudOnly);
    }
    if meta.is_dir() {
        out.items.push(Item {
            rel: rel.clone(),
            abs: abs.to_path_buf(),
            kind: EntryKind::Folder,
            size: 0,
            exec: false,
        });
        let rd = std::fs::read_dir(abs).map_err(|_| PrepError::Unreadable)?;
        for e in rd {
            let e = e.map_err(|_| PrepError::Unreadable)?;
            let path = e.path();
            let m = std::fs::symlink_metadata(&path).map_err(|_| PrepError::Unreadable)?;
            if m.file_type().is_symlink() {
                out.links += 1;
                continue;
            }
            if !m.is_dir() && !m.is_file() {
                // Sockets, pipes, devices: never content a user copies.
                continue;
            }
            let child = format!("{rel}/{}", wire_name(&e.file_name()));
            walk_item(&path, child, &m, depth + 1, out, cancel, progress)?;
        }
    } else {
        out.total = out.total.saturating_add(meta.len());
        out.files += 1;
        progress.file_done();
        out.items.push(Item {
            rel,
            abs: abs.to_path_buf(),
            kind: EntryKind::File,
            size: meta.len(),
            exec: is_exec(meta),
        });
    }
    Ok(())
}

// ------------------------------------------------------------------ hash

/// Where a manifest file comes from on the sending device, and how it looked when hashed. A
/// send checks both again, so a file that changed since is never sent with a stale hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    /// The path.
    pub abs: PathBuf,
    /// Size when hashed.
    pub size: u64,
    /// Modification time when hashed.
    pub mtime: Option<SystemTime>,
}

/// A local file copy, ready to offer and serve.
#[derive(Debug)]
pub struct Prepared {
    /// The manifest.
    pub manifest: Manifest,
    /// One per manifest entry, in manifest order: the file's source, `None` for a folder.
    pub sources: Vec<Option<Source>>,
    /// Links left out by the walk (for the note).
    pub links: u32,
}

/// BLAKE3 of one file: memory-mapped on all cores from [`MMAP_MIN`] bytes, else read.
pub fn hash_file(path: &Path, size: u64) -> io::Result<[u8; 32]> {
    let mut h = blake3::Hasher::new();
    if size >= MMAP_MIN {
        h.update_mmap_rayon(path)?;
    } else {
        h.update_reader(std::fs::File::open(path)?)?;
    }
    Ok(*h.finalize().as_bytes())
}

/// Hash every file of a walk and build the manifest.
pub fn prepare(
    walked: Walked,
    cancel: &AtomicBool,
    progress: &Progress,
) -> Result<Prepared, PrepError> {
    let mut entries = Vec::with_capacity(walked.items.len());
    let mut sources = std::collections::HashMap::with_capacity(walked.items.len());
    for it in walked.items {
        if cancel.load(Ordering::Relaxed) {
            return Err(PrepError::Cancelled);
        }
        match it.kind {
            EntryKind::Folder => entries.push(Entry::folder(it.rel)),
            EntryKind::File => {
                let before = std::fs::metadata(&it.abs).map_err(|_| PrepError::Unreadable)?;
                let hash = hash_file(&it.abs, before.len()).map_err(|_| PrepError::Unreadable)?;
                let after = std::fs::metadata(&it.abs).map_err(|_| PrepError::Unreadable)?;
                let mtime = after.modified().ok();
                if after.len() != before.len() || before.modified().ok() != mtime {
                    return Err(PrepError::Changed);
                }
                progress.add(after.len());
                progress.file_done();
                sources.insert(
                    it.rel.clone(),
                    Source {
                        abs: it.abs,
                        size: after.len(),
                        mtime,
                    },
                );
                entries.push(Entry::file(it.rel, after.len(), hash, it.exec));
            }
        }
    }
    // Any rule the walk could not see (a name over 255 bytes, a path over 4,096) is a limit
    // too: the copy cannot be described, so it is not sent.
    let manifest = Manifest::new(entries).map_err(|_| PrepError::TooMany)?;
    let sources = manifest
        .entries()
        .iter()
        .map(|e| sources.remove(&e.path))
        .collect();
    Ok(Prepared {
        manifest,
        sources,
        links: walked.links,
    })
}

// ------------------------------------------------------------------ cancel

/// Wait until a stream code is set on `rx` (or its sender is gone: aborted).
pub async fn cancelled(rx: &mut watch::Receiver<u32>) -> u32 {
    loop {
        let v = *rx.borrow_and_update();
        if v != 0 {
            return v;
        }
        if rx.changed().await.is_err() {
            return STREAM_ABORTED;
        }
    }
}

// ------------------------------------------------------------------ send

/// Why a send ended early.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    /// The agent cancelled it (the stream was reset with the code).
    Cancelled,
    /// A file could not be opened or read (locked, permission, gone).
    Unreadable,
    /// A file changed since it was hashed.
    Changed,
    /// The stream failed (the receiver stopped it, or the link broke).
    Stream(StreamError),
}

/// Write `buf`, unless the agent cancels first.
async fn put(
    s: &mut SendStream,
    buf: &[u8],
    cancel: &mut watch::Receiver<u32>,
) -> Result<(), Result<u32, StreamError>> {
    tokio::select! {
        r = s.write(buf) => r.map_err(Err),
        c = cancelled(cancel) => Err(Ok(c)),
    }
}

/// Send one file copy on a new data stream to the receiver of `link`: the head and the
/// manifest, then each file's bytes in manifest order (spec 3).
pub async fn serve(
    link: Link,
    id: FilesId,
    copy: Arc<Prepared>,
    progress: Arc<Progress>,
    mut cancel: watch::Receiver<u32>,
) -> Result<(), SendError> {
    let mut stream = tokio::select! {
        s = link.open_stream() => s.map_err(SendError::Stream)?,
        _ = cancelled(&mut cancel) => return Err(SendError::Cancelled),
    };
    let r = serve_on(&mut stream, id, &copy, &progress, &mut cancel).await;
    match r {
        Ok(()) => tokio::select! {
            r = stream.finish() => r.map_err(SendError::Stream),
            // Every byte is out already; the receiver ends it on its side.
            _ = cancelled(&mut cancel) => Err(SendError::Cancelled),
        },
        Err(Ok(code)) => {
            stream.reset(code);
            Err(SendError::Cancelled)
        }
        Err(Err(e)) => {
            stream.reset(STREAM_ABORTED);
            Err(e)
        }
    }
}

/// The body of [`serve`]. `Err(Ok(code))`: cancelled with that code.
async fn serve_on(
    stream: &mut SendStream,
    id: FilesId,
    copy: &Prepared,
    progress: &Progress,
    cancel: &mut watch::Receiver<u32>,
) -> Result<(), Result<u32, SendError>> {
    let stream_err = |e: Result<u32, StreamError>| e.map_err(SendError::Stream);
    let start = core_files::stream_start(&id, &copy.manifest);
    put(stream, &start, cancel).await.map_err(stream_err)?;
    let mut buf = vec![0u8; CHUNK];
    for (e, src) in copy.manifest.entries().iter().zip(&copy.sources) {
        let Some(src) = src else { continue };
        let meta = tokio::fs::metadata(&src.abs)
            .await
            .map_err(|_| Err(SendError::Unreadable))?;
        if meta.len() != src.size || meta.modified().ok() != src.mtime || src.size != e.size {
            return Err(Err(SendError::Changed));
        }
        let mut f = tokio::fs::File::open(&src.abs)
            .await
            .map_err(|_| Err(SendError::Unreadable))?;
        let mut left = e.size;
        while left > 0 {
            let n = left.min(CHUNK as u64) as usize;
            match f.read_exact(&mut buf[..n]).await {
                Ok(_) => {}
                Err(x) if x.kind() == io::ErrorKind::UnexpectedEof => {
                    return Err(Err(SendError::Changed));
                }
                Err(_) => return Err(Err(SendError::Unreadable)),
            }
            put(stream, &buf[..n], cancel).await.map_err(stream_err)?;
            left -= n as u64;
            progress.add(n as u64);
        }
        progress.file_done();
    }
    Ok(())
}

// ------------------------------------------------------------------ receive

/// Read the head of an incoming data stream: the copy ID and the manifest length.
pub async fn read_head(stream: &mut RecvStream) -> Option<(FilesId, usize)> {
    let mut head = [0u8; STREAM_HEAD_LEN];
    tokio::time::timeout(HEAD_TIMEOUT, stream.read_exact(&mut head))
        .await
        .ok()?
        .ok()?;
    core_files::parse_stream_head(&head).ok()
}

/// What a receive needs to know.
#[derive(Clone, Debug)]
pub struct RecvJob {
    /// The copy.
    pub id: FilesId,
    /// The origin.
    pub peer: DeviceId,
    /// From the offer: the manifest must match it.
    pub manifest_hash: [u8; 32],
    /// From the offer.
    pub total: u64,
    /// From the offer.
    pub count: u32,
    /// The files folder.
    pub dir: PathBuf,
    /// The largest image to put next to a single image file (spec 2.2.5), or `None` when
    /// "Sync images" is off.
    pub image_max: Option<u64>,
}

/// A received copy, in its final folder.
#[derive(Debug)]
pub struct Received {
    /// The checked manifest.
    pub manifest: Manifest,
    /// `<files dir>/<short id>`.
    pub folder: PathBuf,
    /// The top-level items, absolute: what goes on the clipboard.
    pub top: Vec<PathBuf>,
    /// Some path is too long for Windows Explorer (spec 2.2.8).
    pub long_paths: bool,
    /// The image to put next to a single image file, as PNG.
    pub image: Option<Vec<u8>>,
}

/// Why a receive ended without files. The partial folder is gone in every case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecvError {
    /// The agent cancelled it (the stream was stopped with the code).
    Cancelled,
    /// The data did not match: manifest hash or rules, a file hash, more bytes than declared,
    /// or a sender that ended early.
    BadData,
    /// Not enough disk space (also a quota).
    NoSpace,
    /// Writing failed for another reason.
    WriteFailed,
    /// The link broke.
    Interrupted,
    /// The sender reset the stream with this code.
    Reset(u32),
}

fn write_err(e: io::Error) -> RecvError {
    match e.kind() {
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => RecvError::NoSpace,
        _ => RecvError::WriteFailed,
    }
}

fn stream_err(e: StreamError) -> RecvError {
    match e {
        StreamError::Reset(c) => RecvError::Reset(c),
        StreamError::FinishedEarly => RecvError::BadData,
        _ => RecvError::Interrupted,
    }
}

/// Read some bytes, unless the agent cancels first. `Ok(None)` at the clean end.
async fn take(
    s: &mut RecvStream,
    buf: &mut [u8],
    cancel: &mut watch::Receiver<u32>,
) -> Result<Option<usize>, Result<u32, RecvError>> {
    tokio::select! {
        r = s.read(buf) => r.map_err(|e| Err(stream_err(e))),
        c = cancelled(cancel) => Err(Ok(c)),
    }
}

/// Fill `buf`, unless the agent cancels first. An early end is bad data.
async fn take_exact(
    s: &mut RecvStream,
    buf: &mut [u8],
    cancel: &mut watch::Receiver<u32>,
) -> Result<(), Result<u32, RecvError>> {
    let mut at = 0;
    while at < buf.len() {
        match take(s, &mut buf[at..], cancel).await? {
            Some(n) => at += n,
            None => return Err(Err(RecvError::BadData)),
        }
    }
    Ok(())
}

/// The UTF-16 length of a path, as Windows Explorer counts it.
fn utf16_len(p: &Path) -> usize {
    p.as_os_str().to_string_lossy().encode_utf16().count()
}

/// Receive one file copy from a stream whose head was read: the manifest (checked against the
/// offer before any file byte is written), then each file, checked against its hash, into
/// `<dir>/<short id>.part`, which is renamed to `<dir>/<short id>` at the end (spec 2.2.3).
pub async fn receive(
    mut stream: RecvStream,
    manifest_len: usize,
    job: RecvJob,
    progress: Arc<Progress>,
    mut cancel: watch::Receiver<u32>,
) -> Result<Received, RecvError> {
    let short = job.id.short();
    let part = job.dir.join(format!("{short}.part"));
    let r = receive_into(
        &mut stream,
        manifest_len,
        &job,
        &part,
        &progress,
        &mut cancel,
    )
    .await;
    let r = match r {
        Ok(m) => finish(m, &job, &part).await,
        Err(Ok(code)) => {
            stream.stop(code);
            Err(RecvError::Cancelled)
        }
        Err(Err(e)) => Err(e),
    };
    if r.is_err() {
        // A stream still open is stopped (aborted) when it drops at the end.
        let p = part.clone();
        let _ = tokio::task::spawn_blocking(move || std::fs::remove_dir_all(p)).await;
    }
    r
}

async fn receive_into(
    stream: &mut RecvStream,
    manifest_len: usize,
    job: &RecvJob,
    part: &Path,
    progress: &Progress,
    cancel: &mut watch::Receiver<u32>,
) -> Result<Manifest, Result<u32, RecvError>> {
    let mut mbytes = vec![0u8; manifest_len];
    take_exact(stream, &mut mbytes, cancel).await?;
    let manifest =
        Manifest::decode(&mbytes, &job.manifest_hash).map_err(|_| Err(RecvError::BadData))?;
    if manifest.total() != job.total || manifest.file_count() != job.count {
        return Err(Err(RecvError::BadData));
    }
    drop(mbytes);
    let locals = manifest.local_paths(LocalRules::this_os());
    let we = |e: io::Error| Err(write_err(e));
    if tokio::fs::metadata(part).await.is_ok() {
        tokio::fs::remove_dir_all(part).await.map_err(we)?;
    }
    tokio::fs::create_dir_all(part).await.map_err(we)?;
    let mut buf = vec![0u8; CHUNK];
    for (e, local) in manifest.entries().iter().zip(&locals) {
        let path = local.split('/').fold(part.to_path_buf(), |p, s| p.join(s));
        if e.kind == EntryKind::Folder {
            tokio::fs::create_dir(&path).await.map_err(we)?;
            continue;
        }
        let mut f = tokio::fs::File::create(&path).await.map_err(we)?;
        let mut h = blake3::Hasher::new();
        let mut left = e.size;
        while left > 0 {
            let want = left.min(CHUNK as u64) as usize;
            let n = match take(stream, &mut buf[..want], cancel).await? {
                Some(n) => n,
                None => return Err(Err(RecvError::BadData)),
            };
            h.update(&buf[..n]);
            f.write_all(&buf[..n]).await.map_err(we)?;
            left -= n as u64;
            progress.add(n as u64);
        }
        f.flush().await.map_err(we)?;
        f.sync_all().await.map_err(we)?;
        drop(f);
        if h.finalize().as_bytes() != &e.hash {
            return Err(Err(RecvError::BadData));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if e.exec { 0o755 } else { 0o644 };
            tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
                .await
                .map_err(we)?;
        }
        progress.file_done();
    }
    // The stream must end here: a sender that sends more than it declared is refused.
    let mut extra = [0u8; 1];
    match take(stream, &mut extra, cancel).await? {
        None => Ok(manifest),
        Some(_) => Err(Err(RecvError::BadData)),
    }
}

async fn finish(manifest: Manifest, job: &RecvJob, part: &Path) -> Result<Received, RecvError> {
    let folder = job.dir.join(job.id.short());
    let (p, f) = (part.to_path_buf(), folder.clone());
    tokio::task::spawn_blocking(move || {
        if f.exists() {
            std::fs::remove_dir_all(&f)?;
        }
        rename_retry(&p, &f)
    })
    .await
    .map_err(|_| RecvError::WriteFailed)?
    .map_err(write_err)?;
    let locals = manifest.local_paths(LocalRules::this_os());
    let mut top = Vec::new();
    let mut long_paths = false;
    for (e, local) in manifest.entries().iter().zip(&locals) {
        let path = local.split('/').fold(folder.clone(), |p, s| p.join(s));
        if utf16_len(&path) > core_files::WINDOWS_PATH_MAX {
            long_paths = true;
        }
        if !e.path.contains('/') {
            top.push(path);
        }
    }
    let image = match job.image_max.and_then(|max| manifest.single_image(max)) {
        Some(e) => {
            let path = top.first().cloned();
            let max = job.image_max.unwrap_or(0);
            let ext = e.path.rsplit_once('.').map(|(_, x)| x.to_ascii_lowercase());
            tokio::task::spawn_blocking(move || {
                let bytes = std::fs::read(path?).ok()?;
                companion_png(&bytes, ext.as_deref()?, max)
            })
            .await
            .ok()
            .flatten()
        }
        None => None,
    };
    Ok(Received {
        manifest,
        folder,
        top,
        long_paths,
        image,
    })
}

/// The PNG that goes on the clipboard next to a single image file (spec 2.2.5): a PNG as it is,
/// a JPEG or GIF converted. `None` if it cannot be read or is over `max` bytes as PNG.
pub fn companion_png(bytes: &[u8], ext: &str, max: u64) -> Option<Vec<u8>> {
    let png = if ext == "png" {
        uc_core::image::check_local(bytes).ok()?;
        bytes.to_vec()
    } else {
        let format = match ext {
            "jpg" | "jpeg" => image::ImageFormat::Jpeg,
            "gif" => image::ImageFormat::Gif,
            _ => return None,
        };
        let mut reader = image::ImageReader::with_format(io::Cursor::new(bytes), format);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(16_384);
        limits.max_image_height = Some(16_384);
        limits.max_alloc = Some(512 << 20);
        reader.limits(limits);
        let img = reader.decode().ok()?;
        let mut out = io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).ok()?;
        out.into_inner()
    };
    (png.len() as u64 <= max).then_some(png)
}

// ------------------------------------------------------------------ files folder

/// Rename a finished folder. On Windows a virus scanner or the indexer may hold the new files
/// for a moment: try again for up to [`RENAME_RETRY`].
pub fn rename_retry(from: &Path, to: &Path) -> io::Result<()> {
    let start = std::time::Instant::now();
    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if cfg!(windows) && start.elapsed() < RENAME_RETRY => {
                let _ = e;
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(e),
        }
    }
}

/// The folders of the files folder: (name, modified, is a `.part` folder).
fn folders(dir: &Path) -> Vec<(String, SystemTime, bool)> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    rd.filter_map(Result::ok)
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            if !m.is_dir() {
                return None;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            let t = m.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let part = name.ends_with(".part");
            Some((name, t, part))
        })
        .collect()
}

/// At agent start: delete `.part` leftovers and folders older than [`MAX_AGE`]. Returns how
/// many folders went.
pub fn sweep(dir: &Path, now: SystemTime) -> usize {
    let mut n = 0;
    for (name, t, part) in folders(dir) {
        let old = now.duration_since(t).is_ok_and(|age| age > MAX_AGE);
        if (part || old) && std::fs::remove_dir_all(dir.join(&name)).is_ok() {
            n += 1;
        }
    }
    n
}

/// Keep the newest [`KEEP`] received copies and every folder in `protected` (current, pending,
/// or the source of a running send). `.part` folders are left to their own receive. Returns
/// how many folders went.
pub fn keep_last(dir: &Path, protected: &HashSet<String>) -> usize {
    let mut list: Vec<(String, SystemTime)> = folders(dir)
        .into_iter()
        .filter(|(_, _, part)| !part)
        .map(|(n, t, _)| (n, t))
        .collect();
    list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut n = 0;
    for (i, (name, _)) in list.into_iter().enumerate() {
        if i >= KEEP
            && !protected.contains(&name)
            && std::fs::remove_dir_all(dir.join(&name)).is_ok()
        {
            n += 1;
        }
    }
    n
}

/// The files folder name that holds `path`, if `path` is inside `dir` (a received copy that is
/// copied again from there is the source of a send).
pub fn folder_of(dir: &Path, path: &Path) -> Option<String> {
    let rest = path.strip_prefix(dir).ok()?;
    let first = rest.components().next()?;
    Some(first.as_os_str().to_string_lossy().into_owned())
}

/// Free bytes for this user on the volume of `dir`.
pub fn free_space(dir: &Path) -> Option<u64> {
    #[cfg(windows)]
    {
        crate::win::free_space(dir).ok()
    }
    #[cfg(unix)]
    {
        unix_free_space(dir)
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = dir;
        None
    }
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn unix_free_space(dir: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid C string and `st` a writable statvfs.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    #[allow(clippy::unnecessary_cast)]
    Some((st.f_bavail as u64).saturating_mul(st.f_frsize as u64))
}

// ------------------------------------------------------------------ words

/// A size for notes: "512 bytes", "12 KB", "12 MB", "1.5 GB" (1 KB = 1024 bytes, as the file
/// limit setting counts).
pub fn size_text(bytes: u64) -> String {
    const K: u128 = 1024;
    // Rounded to the nearest unit, never down to 0.
    let round = |unit: u128| ((u128::from(bytes) + unit / 2) / unit).max(1);
    match u128::from(bytes) {
        1 => "1 byte".into(),
        n if n < K => format!("{n} bytes"),
        n if n < K * K => format!("{} KB", round(K)),
        n if n < K * K * K => format!("{} MB", round(K * K)),
        _ => {
            let tenths = round(K * K * K / 10);
            format!("{}.{} GB", tenths / 10, tenths % 10)
        }
    }
}

/// The "kind" of a file copy in Last activity: "1 file, 12 MB", "3 files, 12 MB", or "Folder"
/// for folders with no file in them. Counts and sizes only, never a name.
pub fn files_kind(count: u32, total: u64) -> String {
    match count {
        0 => "Folder".into(),
        1 => format!("1 file, {}", size_text(total)),
        n => format!("{n} files, {}", size_text(total)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        let d = t.path().join("Photos");
        std::fs::create_dir_all(d.join("empty")).unwrap();
        std::fs::create_dir_all(d.join("trip")).unwrap();
        std::fs::write(d.join("trip").join("a.jpg"), vec![1u8; 1000]).unwrap();
        std::fs::write(d.join("b.txt"), b"hello").unwrap();
        std::fs::write(t.path().join("run.sh"), b"#!/bin/sh\n").unwrap();
        t
    }

    fn no_cancel() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn walk_and_prepare_a_folder_and_a_file() {
        let t = tree();
        let paths = vec![t.path().join("Photos"), t.path().join("run.sh")];
        let p = Progress::default();
        let w = walk(&paths, &no_cancel(), &p).unwrap();
        assert_eq!(w.files, 3);
        assert_eq!(w.total, 1000 + 5 + 10);
        assert_eq!(w.links, 0);
        let prep = prepare(w, &no_cancel(), &p).unwrap();
        let paths: Vec<&str> = prep
            .manifest
            .entries()
            .iter()
            .map(|e| e.path.as_str())
            .collect();
        assert_eq!(
            paths,
            [
                "Photos",
                "Photos/b.txt",
                "Photos/empty",
                "Photos/trip",
                "Photos/trip/a.jpg",
                "run.sh"
            ]
        );
        // Sources line up with entries: none for folders, the file for files.
        for (e, s) in prep.manifest.entries().iter().zip(&prep.sources) {
            assert_eq!(e.kind == EntryKind::File, s.is_some(), "{}", e.path);
        }
        let b = &prep.manifest.entries()[1];
        assert_eq!(b.hash, *blake3::hash(b"hello").as_bytes());
        assert_eq!(p.get().0, 1015);
    }

    #[test]
    fn same_names_at_the_top_get_a_number() {
        let t = tempfile::tempdir().unwrap();
        for d in ["x", "y"] {
            std::fs::create_dir(t.path().join(d)).unwrap();
            std::fs::write(t.path().join(d).join("a.txt"), d).unwrap();
        }
        let paths = vec![t.path().join("x/a.txt"), t.path().join("y/a.txt")];
        let w = walk(&paths, &no_cancel(), &Progress::default()).unwrap();
        let names: Vec<&str> = w.items.iter().map(|i| i.rel.as_str()).collect();
        assert_eq!(names, ["a.txt", "a (2).txt"]);
    }

    #[test]
    fn a_missing_item_is_unreadable_and_cancel_stops() {
        let t = tree();
        let gone = vec![t.path().join("nope")];
        assert_eq!(
            walk(&gone, &no_cancel(), &Progress::default()),
            Err(PrepError::Unreadable)
        );
        let stop = AtomicBool::new(true);
        let paths = vec![t.path().join("Photos")];
        assert_eq!(
            walk(&paths, &stop, &Progress::default()),
            Err(PrepError::Cancelled)
        );
    }

    #[test]
    fn too_deep_is_too_many() {
        let t = tempfile::tempdir().unwrap();
        let mut p = t.path().join("d");
        // Short names keep the path well under Windows limits.
        for _ in 0..=MAX_DEPTH {
            p = p.join("d");
        }
        if std::fs::create_dir_all(&p).is_err() {
            return; // A file system that cannot nest this deep.
        }
        let w = walk(&[t.path().join("d")], &no_cancel(), &Progress::default());
        assert_eq!(w, Err(PrepError::TooMany));
    }

    #[cfg(unix)]
    #[test]
    fn links_inside_folders_are_left_out_and_exec_is_kept() {
        use std::os::unix::fs::PermissionsExt;
        let t = tree();
        let d = t.path().join("Photos");
        std::os::unix::fs::symlink("/etc", d.join("link")).unwrap();
        let sh = t.path().join("run.sh");
        std::fs::set_permissions(&sh, std::fs::Permissions::from_mode(0o755)).unwrap();
        let w = walk(&[d, sh], &no_cancel(), &Progress::default()).unwrap();
        assert_eq!(w.links, 1);
        assert!(w.items.iter().all(|i| !i.rel.ends_with("link")));
        assert!(w.items.iter().find(|i| i.rel == "run.sh").unwrap().exec);
    }

    #[test]
    fn prepare_records_the_hashed_size() {
        let t = tree();
        let f = t.path().join("run.sh");
        let w = walk(std::slice::from_ref(&f), &no_cancel(), &Progress::default()).unwrap();
        let prep = prepare(w, &no_cancel(), &Progress::default()).unwrap();
        // Hashing reads the size again, so the manifest has the size that was hashed.
        assert_eq!(prep.manifest.entries()[0].size, 10);
        assert_eq!(prep.sources[0].as_ref().unwrap().size, 10);
    }

    #[test]
    fn big_files_hash_the_same_with_mmap() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("big.bin");
        let data: Vec<u8> = (0..MMAP_MIN as usize + 7)
            .map(|i| (i % 251) as u8)
            .collect();
        std::fs::write(&f, &data).unwrap();
        assert_eq!(
            hash_file(&f, data.len() as u64).unwrap(),
            *blake3::hash(&data).as_bytes()
        );
        let small = t.path().join("small.bin");
        std::fs::write(&small, &data[..100]).unwrap();
        assert_eq!(
            hash_file(&small, 100).unwrap(),
            *blake3::hash(&data[..100]).as_bytes()
        );
    }

    #[test]
    fn sweep_and_keep_last() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path();
        for n in ["a1", "a2", "a3", "a4", "a5", "x.part"] {
            std::fs::create_dir(dir.join(n)).unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
        std::fs::write(dir.join("loose.txt"), b"x").unwrap();
        // At start: the .part leftover goes, recent folders stay.
        assert_eq!(sweep(dir, SystemTime::now()), 1);
        assert!(!dir.join("x.part").exists());
        // Eight days later, everything is old.
        let later = SystemTime::now() + Duration::from_secs(8 * 24 * 3600);
        // Keep 3 newest (a5, a4, a3) and the protected a1.
        let protected: HashSet<String> = ["a1".to_string()].into();
        assert_eq!(keep_last(dir, &protected), 1);
        for (n, there) in [
            ("a1", true),
            ("a2", false),
            ("a3", true),
            ("a4", true),
            ("a5", true),
        ] {
            assert_eq!(dir.join(n).exists(), there, "{n}");
        }
        assert_eq!(sweep(dir, later), 4);
        assert!(dir.join("loose.txt").exists(), "only folders are swept");
    }

    #[test]
    fn folder_of_a_received_copy() {
        let dir = Path::new("/c/files");
        assert_eq!(
            folder_of(dir, Path::new("/c/files/ab12cd34/Photos/a.jpg")).as_deref(),
            Some("ab12cd34")
        );
        assert_eq!(folder_of(dir, Path::new("/c/other/a.jpg")), None);
    }

    #[test]
    fn free_space_of_a_real_folder() {
        let t = tempfile::tempdir().unwrap();
        assert!(free_space(t.path()).is_some_and(|n| n > 0));
    }

    #[test]
    fn sizes_and_kinds_in_words() {
        assert_eq!(size_text(0), "0 bytes");
        assert_eq!(size_text(1), "1 byte");
        assert_eq!(size_text(1023), "1023 bytes");
        assert_eq!(size_text(1024), "1 KB");
        assert_eq!(size_text(1500), "1 KB");
        assert_eq!(size_text(1600), "2 KB");
        assert_eq!(size_text((3 << 20) + 1005), "3 MB");
        assert_eq!(size_text(12 << 20), "12 MB");
        assert_eq!(size_text(3 << 29), "1.5 GB");
        assert_eq!(files_kind(0, 0), "Folder");
        assert_eq!(files_kind(1, 12 << 20), "1 file, 12 MB");
        assert_eq!(files_kind(3, 12 << 20), "3 files, 12 MB");
    }

    #[test]
    fn companion_images() {
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([200, 10, 10]));
        let mut png = io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let png = png.into_inner();
        assert_eq!(
            companion_png(&png, "png", 1 << 20).as_deref(),
            Some(&png[..])
        );
        assert_eq!(companion_png(&png, "png", 10), None, "over the image limit");
        let mut jpg = io::Cursor::new(Vec::new());
        img.write_to(&mut jpg, image::ImageFormat::Jpeg).unwrap();
        let out = companion_png(jpg.get_ref(), "jpg", 1 << 20).unwrap();
        assert!(out.starts_with(b"\x89PNG"));
        assert_eq!(companion_png(b"not an image", "jpg", 1 << 20), None);
        assert_eq!(companion_png(&png, "svg", 1 << 20), None);
    }
}
