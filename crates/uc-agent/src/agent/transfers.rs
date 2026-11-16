//! File copies in the agent core (file sync spec 2 and 5): the walk and hash jobs, sends,
//! the fetch, the files folder, notes, and the `transfer` view for the window and `uc status`.
//!
//! The engine decides; this part runs the jobs it asks for and reports back. Each job runs off
//! the main loop (`spawn_blocking` for the walk and the hash, a tokio task for a stream) and
//! sends one [`Xfer`] when it ends. A job carries a serial (sends, fetch) or a generation
//! (walk): a result whose job was cancelled or replaced meanwhile is dropped.
//!
//! Notes and logs hold counts and sizes only, never a file name or a path.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};
use uc_core::files::space_needed;
use uc_core::sync::{CancelReason, Event as EngineEvent, SkipReason, TransferFailure};
use uc_core::{DeviceId, FilesId};
use uc_net::{RecvStream, STREAM_ABORTED};

use super::{Core, now_ms, short_id};
use crate::files::{
    self, PrepError, Prepared, Progress, Received, RecvError, RecvJob, SendError, Walked,
    files_kind,
};
use crate::ipc::IpcError;

/// A file job ended: sent from the job to the main loop.
pub(super) enum Xfer {
    /// A walk ended.
    Walked {
        generation: u64,
        result: Result<Walked, PrepError>,
    },
    /// A hash ended.
    Prepared {
        generation: u64,
        result: Result<Prepared, PrepError>,
    },
    /// A data stream came in and its head was read (`None`: no valid head in time).
    Head {
        peer: DeviceId,
        link: u64,
        head: Option<(FilesId, usize)>,
        stream: RecvStream,
    },
    /// A send ended.
    Served {
        peer: DeviceId,
        id: FilesId,
        serial: u64,
        result: Result<(), SendError>,
    },
    /// A receive ended.
    Received {
        serial: u64,
        result: Result<Received, RecvError>,
    },
}

/// A walk and then a hash of a local file copy.
struct WalkJob {
    generation: u64,
    /// The copied (or picked) items, absolute.
    paths: Vec<PathBuf>,
    picked: bool,
    cancel: Arc<AtomicBool>,
    progress: Arc<Progress>,
    /// Total bytes, once walked.
    total: u64,
    /// The walk's result, between the walk and the hash.
    walked: Option<Walked>,
}

/// One running send.
struct SendJob {
    serial: u64,
    /// Set to a stream code to cancel. Dropping it aborts the send too.
    cancel: watch::Sender<u32>,
    progress: Arc<Progress>,
    total: u64,
}

/// The fetch of the pending offer: asked for, then receiving once its stream came.
struct FetchJob {
    id: FilesId,
    peer: DeviceId,
    serial: u64,
    total: u64,
    count: u32,
    /// Set once the stream came and the receive runs.
    cancel: Option<watch::Sender<u32>>,
    progress: Arc<Progress>,
}

/// The local file copy that is the current copy: what this device offers and serves.
struct LocalCopy {
    id: FilesId,
    prepared: Arc<Prepared>,
    /// The copied items, to keep a received folder that is copied again (a send source).
    roots: Vec<PathBuf>,
}

/// File state of the agent core.
pub(super) struct FileState {
    /// The files folder (`Profile::files_dir`).
    pub(super) dir: PathBuf,
    tx: mpsc::UnboundedSender<Xfer>,
    walk: Option<WalkJob>,
    local: Option<LocalCopy>,
    sends: HashMap<(DeviceId, FilesId), SendJob>,
    fetch: Option<FetchJob>,
    serial: u64,
    /// The received copy that is the current copy: its folder is kept.
    current_folder: Option<String>,
    /// The last received copy had a path too long for Windows Explorer.
    long_paths: bool,
    /// A more exact note for the next `TransferFailed` (set by the agent, not the engine).
    fail_note: Option<String>,
    /// Cancels in this call are the agent's own clean-up: no notes.
    quiet: bool,
    /// The last transfer error: the note and when.
    pub(super) last_error: Option<(String, u64)>,
    /// The `transfer` view last sent to the window, so only changes go out.
    shown: Option<Value>,
    /// Something changed: send the view at the next progress tick.
    dirty: bool,
    /// Test profiles: sends go no faster than this many bytes per second
    /// (`UCLIP_TEST_SEND_RATE`), so a test can act while a transfer runs.
    pub(super) send_rate: Option<u64>,
}

impl FileState {
    pub(super) fn new(dir: PathBuf) -> (FileState, mpsc::UnboundedReceiver<Xfer>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            FileState {
                dir,
                tx,
                walk: None,
                local: None,
                sends: HashMap::new(),
                fetch: None,
                serial: 0,
                current_folder: None,
                long_paths: false,
                fail_note: None,
                quiet: false,
                last_error: None,
                shown: None,
                dirty: false,
                send_rate: None,
            },
            rx,
        )
    }

    /// A walk, send or fetch runs, or the window has not seen the last change yet: the
    /// progress tick runs.
    pub(super) fn busy(&self) -> bool {
        self.dirty || self.walk.is_some() || !self.sends.is_empty() || self.fetch.is_some()
    }

    fn next_serial(&mut self) -> u64 {
        self.serial += 1;
        self.serial
    }

    fn cancel_walk(&mut self) {
        if let Some(w) = self.walk.take() {
            w.cancel.store(true, Ordering::Relaxed);
            self.dirty = true;
        }
    }
}

/// The note for a skipped file copy (spec 2.1, 2.4). `None` for skips that are not about files.
fn skip_note(r: &SkipReason, limit_mb: u32) -> Option<String> {
    Some(match r {
        SkipReason::FilesOff => "Files are not synced (setting is off).".into(),
        SkipReason::FilesOverLimit => format!(
            "Files over {limit_mb} MB are not sent automatically. Use Copy files... in the app."
        ),
        SkipReason::FilesVirtual => {
            "These files are not on the disk (an attachment or an item in a zip), so they are not synced."
                .into()
        }
        SkipReason::FilesCloudOnly => {
            "Some files are only in the cloud, so the copy is not synced. Make them available offline first."
                .into()
        }
        SkipReason::FilesTooMany => {
            "Too many files or folders to sync (at most 100,000 items, 64 levels deep).".into()
        }
        _ => return None,
    })
}

/// The note for a walk or hash that failed in the agent.
fn prep_note(e: PrepError) -> Option<&'static str> {
    match e {
        PrepError::Unreadable => Some("The files could not be read, so they were not sent."),
        PrepError::Changed => Some("A file changed while it was read. Copy it again."),
        PrepError::CloudOnly | PrepError::TooMany | PrepError::Cancelled => None,
    }
}

impl Core {
    // ------------------------------------------------------------ engine events

    /// Handle a file event of the engine. Returns false for events that are not about files.
    pub(super) fn on_file_event(&mut self, e: &EngineEvent) -> bool {
        match e {
            EngineEvent::WalkFiles {
                generation,
                paths,
                picked,
            } => self.start_walk(*generation, paths.clone(), *picked),
            EngineEvent::HashFiles { generation } => self.start_prepare(*generation),
            EngineEvent::FilesSent {
                id,
                total,
                count,
                peers,
            } => {
                tracing::info!(files = count, bytes = total, peers, "files offered");
                let links = self
                    .files
                    .local
                    .as_ref()
                    .filter(|l| l.id == *id)
                    .map_or(0, |l| l.prepared.links);
                let note = (links > 0).then(|| {
                    format!(
                        "{links} {} left out; apps and bundles may not work.",
                        if links == 1 { "link was" } else { "links were" }
                    )
                });
                self.activity(&files_kind(*count, *total), "sent", None, note);
            }
            EngineEvent::FetchFiles {
                peer,
                id,
                total,
                count,
            } => self.fetch_files(*peer, *id, *total, *count),
            EngineEvent::ServeFiles { peer, id } => self.serve_files(*peer, *id),
            EngineEvent::CancelTransfer {
                id,
                peer,
                reason,
                sending,
            } => self.cancel_transfer(*id, *peer, *reason, *sending),
            EngineEvent::FilesApplied {
                origin,
                total,
                count,
                ..
            } => {
                tracing::info!(files = count, bytes = total, "files received");
                let name = self.name_of(origin);
                let note = std::mem::take(&mut self.files.long_paths)
                    .then(|| "Some paths are too long for Windows Explorer.".to_string());
                self.activity(&files_kind(*count, *total), "received", Some(name), note);
            }
            EngineEvent::FilesHeldOnly { .. } => {
                tracing::info!("file copy remembered, not fetched (old, paused or setting off)")
            }
            EngineEvent::TransferFailed { id, peer, failure } => {
                self.fetch_failed(*id, *peer, *failure)
            }
            EngineEvent::Skipped(SkipReason::PeerNoFiles(peer)) => {
                let name = self.name_of(peer);
                tracing::info!(peer = %short_id(&peer.0), "peer cannot receive files");
                self.activity(
                    "Files",
                    "not sent",
                    Some(name.clone()),
                    Some(format!("{name} needs an update to receive files.")),
                );
            }
            EngineEvent::Skipped(r) => {
                let Some(note) = skip_note(r, self.state.settings.file_limit_mb) else {
                    return false;
                };
                tracing::info!(reason = ?r, "file copy skipped");
                self.activity("Files", "not sent", None, Some(note));
            }
            _ => return false,
        }
        true
    }

    /// After engine events: a walk whose generation the engine dropped (a newer copy, pause,
    /// setting off) is cancelled.
    pub(super) fn sync_walk(&mut self) {
        if self
            .files
            .walk
            .as_ref()
            .is_some_and(|w| self.engine.walk_gen() != Some(w.generation))
        {
            tracing::debug!("walk ended by a newer copy");
            self.files.cancel_walk();
        }
    }

    // ------------------------------------------------------------ local copy

    fn start_walk(&mut self, generation: u64, paths: Vec<PathBuf>, picked: bool) {
        self.files.cancel_walk();
        tracing::info!(items = paths.len(), picked, "preparing files");
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Progress::default());
        self.files.walk = Some(WalkJob {
            generation,
            paths: paths.clone(),
            picked,
            cancel: cancel.clone(),
            progress: progress.clone(),
            total: 0,
            walked: None,
        });
        self.files.dirty = true;
        let tx = self.files.tx.clone();
        tokio::task::spawn_blocking(move || {
            let result = files::walk(&paths, &cancel, &progress);
            let _ = tx.send(Xfer::Walked { generation, result });
        });
    }

    fn start_prepare(&mut self, generation: u64) {
        let Some(w) = self
            .files
            .walk
            .as_mut()
            .filter(|w| w.generation == generation)
        else {
            return;
        };
        let Some(walked) = w.walked.take() else {
            return;
        };
        // A fresh count: "Preparing" now counts hashed bytes and files.
        w.progress = Arc::new(Progress::default());
        let (cancel, progress) = (w.cancel.clone(), w.progress.clone());
        let tx = self.files.tx.clone();
        tokio::task::spawn_blocking(move || {
            let result = files::prepare(walked, &cancel, &progress);
            let _ = tx.send(Xfer::Prepared { generation, result });
        });
    }

    fn on_walked(&mut self, generation: u64, result: Result<Walked, PrepError>) {
        if self
            .files
            .walk
            .as_ref()
            .is_none_or(|w| w.generation != generation)
        {
            return;
        }
        match result {
            Ok(walked) => {
                tracing::info!(
                    files = walked.files,
                    bytes = walked.total,
                    links = walked.links,
                    "walked"
                );
                if let Some(w) = &mut self.files.walk {
                    w.total = walked.total;
                    w.walked = Some(walked);
                }
                let total = self.files.walk.as_ref().map_or(0, |w| w.total);
                let evs = self.engine.on_walked(generation, total);
                self.on_engine(evs);
            }
            Err(e) => self.prep_failed(generation, e),
        }
    }

    fn on_prepared(&mut self, generation: u64, result: Result<Prepared, PrepError>) {
        let Some(job) = self.files.walk.take_if(|w| w.generation == generation) else {
            return;
        };
        self.files.dirty = true;
        let prepared = match result {
            Ok(p) => p,
            Err(e) => {
                self.files.walk = Some(job);
                self.prep_failed(generation, e);
                return;
            }
        };
        let summary = prepared.manifest.summary();
        let evs = self.engine.on_local_files(generation, summary, now_ms());
        let sent = evs.iter().find_map(|e| match e {
            EngineEvent::FilesSent { id, .. } => Some(*id),
            _ => None,
        });
        if let Some(id) = sent {
            self.files.local = Some(LocalCopy {
                id,
                prepared: Arc::new(prepared),
                roots: job.paths.clone(),
            });
        }
        self.on_engine(evs);
        // A picked copy goes on this clipboard too, with our marker, recorded as our own write,
        // so the watcher does not take it for a new automatic copy (spec 2.1.4).
        if sent.is_some() && job.picked {
            match uc_clip::Clipboard::write_files(&mut self.clip, &job.paths, None) {
                Ok(seq) => self.engine.note_own_write(seq),
                Err(e) => tracing::warn!(error = %e, "picked files could not go on this clipboard"),
            }
        }
    }

    /// A walk or hash failed in the agent: tell the engine (if the walk is still current).
    fn prep_failed(&mut self, generation: u64, e: PrepError) {
        self.files.walk = None;
        self.files.dirty = true;
        tracing::info!(error = ?e, "files not prepared");
        let reason = match e {
            PrepError::CloudOnly => SkipReason::FilesCloudOnly,
            PrepError::TooMany => SkipReason::FilesTooMany,
            PrepError::Cancelled => return,
            // No engine reason fits; the agent writes its own note below.
            PrepError::Unreadable | PrepError::Changed => SkipReason::Empty,
        };
        let evs = self.engine.walk_failed(generation, reason);
        if evs.is_empty() {
            return;
        }
        match prep_note(e) {
            Some(note) => self.activity("Files", "not sent", None, Some(note.into())),
            None => self.on_engine(evs),
        }
    }

    // ------------------------------------------------------------ sends

    fn serve_files(&mut self, peer: DeviceId, id: FilesId) {
        let link = self
            .peers
            .get(&peer)
            .filter(|p| p.gate.data_allowed())
            .map(|p| p.link.clone());
        let copy = self
            .files
            .local
            .as_ref()
            .filter(|l| l.id == id)
            .map(|l| l.prepared.clone());
        let (Some(link), Some(copy)) = (link, copy) else {
            let evs = self.engine.serve_done(peer, id);
            self.on_engine(evs);
            return;
        };
        let serial = self.files.next_serial();
        let (cancel, rx) = watch::channel(0u32);
        let progress = Arc::new(Progress::default());
        let total = copy.manifest.total();
        tracing::info!(peer = %short_id(&peer.0), bytes = total, "sending files");
        self.files.sends.insert(
            (peer, id),
            SendJob {
                serial,
                cancel,
                progress: progress.clone(),
                total,
            },
        );
        self.files.dirty = true;
        let tx = self.files.tx.clone();
        let rate = self.files.send_rate;
        tokio::spawn(async move {
            let result = files::serve(link, id, copy, progress, rx, rate).await;
            let _ = tx.send(Xfer::Served {
                peer,
                id,
                serial,
                result,
            });
        });
    }

    fn on_served(
        &mut self,
        peer: DeviceId,
        id: FilesId,
        serial: u64,
        result: Result<(), SendError>,
    ) {
        if self
            .files
            .sends
            .get(&(peer, id))
            .is_none_or(|s| s.serial != serial)
        {
            // Cancelled or replaced meanwhile: the engine already knows.
            return;
        }
        let job = self.files.sends.remove(&(peer, id));
        self.files.dirty = true;
        let short = short_id(&peer.0);
        let note = match result {
            Ok(()) => {
                tracing::info!(peer = %short, bytes = job.map_or(0, |j| j.total), "files sent");
                None
            }
            Err(SendError::Unreadable) => Some(
                "A file could not be opened (in use or no access), so the files were not sent.",
            ),
            Err(SendError::Changed) => Some("A file changed while it was sent. Copy it again."),
            Err(SendError::Cancelled) => None,
            Err(SendError::Stream(e)) => {
                tracing::info!(peer = %short, error = %e, "send ended");
                None
            }
        };
        let evs = self.engine.serve_done(peer, id);
        self.on_engine(evs);
        if let Some(note) = note {
            tracing::warn!(peer = %short, "send aborted: {note}");
            self.files.last_error = Some((note.to_string(), now_ms()));
            self.activity("Files", "not sent", None, Some(note.into()));
            // The copy cannot be read as it was hashed: stop its other sends too (their
            // receivers learn it from the engine). No more notes for those.
            self.files.quiet = true;
            let evs = self.engine.stop_transfer();
            self.on_engine(evs);
            self.files.quiet = false;
        }
    }

    // ------------------------------------------------------------ fetch

    fn fetch_files(&mut self, peer: DeviceId, id: FilesId, total: u64, count: u32) {
        self.drop_fetch();
        let dir = self.files.dir.clone();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!(error = %e, "files folder could not be made");
        }
        let free = files::free_space(&dir);
        if free.is_some_and(|f| f < space_needed(total)) {
            tracing::info!(
                bytes = total,
                free = free.unwrap_or(0),
                "not enough disk space for files"
            );
            let evs = self.engine.transfer_failed(id, TransferFailure::NoSpace);
            self.on_engine(evs);
            return;
        }
        let serial = self.files.next_serial();
        self.files.fetch = Some(FetchJob {
            id,
            peer,
            serial,
            total,
            count,
            cancel: None,
            progress: Arc::new(Progress::default()),
        });
        self.files.dirty = true;
        tracing::info!(peer = %short_id(&peer.0), files = count, bytes = total, "asking for files");
        let evs = self.engine.files_want(id);
        self.on_engine(evs);
    }

    /// Forget the fetch job; a running receive stops (aborted) and deletes its `.part` folder.
    fn drop_fetch(&mut self) {
        if let Some(f) = self.files.fetch.take() {
            if let Some(c) = f.cancel {
                let _ = c.send(STREAM_ABORTED);
            }
            self.files.dirty = true;
        }
    }

    /// A data stream came in: read its head off the main loop.
    pub(super) fn on_stream(&mut self, peer: DeviceId, link: u64, mut stream: RecvStream) {
        let tx = self.files.tx.clone();
        tokio::spawn(async move {
            let head = files::read_head(&mut stream).await;
            let _ = tx.send(Xfer::Head {
                peer,
                link,
                head,
                stream,
            });
        });
    }

    fn on_head(
        &mut self,
        peer: DeviceId,
        link: u64,
        head: Option<(FilesId, usize)>,
        stream: RecvStream,
    ) {
        // Only the stream we asked this peer for, on its current member link (spec 3).
        let current_link = self.peers.get(&peer).is_some_and(|p| p.link.id == link);
        let job = self.files.fetch.as_ref().filter(|f| {
            f.peer == peer
                && f.cancel.is_none()
                && head.is_some_and(|(id, _)| id == f.id && self.engine.stream_allowed(peer, id))
        });
        let (Some(job), Some((id, manifest_len)), true) = (job, head, current_link) else {
            tracing::debug!(peer = %short_id(&peer.0), "data stream nobody asked for: stopped");
            stream.stop(STREAM_ABORTED);
            return;
        };
        let Some(offer) = self.engine.pending_files().filter(|o| o.id == id).cloned() else {
            stream.stop(STREAM_ABORTED);
            return;
        };
        let s = &self.state.settings;
        let rjob = RecvJob {
            id,
            peer,
            manifest_hash: offer.manifest_hash,
            total: offer.total,
            count: offer.count,
            dir: self.files.dir.clone(),
            image_max: s.sync_images.then(|| u64::from(s.image_limit_mb) << 20),
        };
        let (cancel, rx) = watch::channel(0u32);
        let (serial, progress) = (job.serial, job.progress.clone());
        if let Some(f) = &mut self.files.fetch {
            f.cancel = Some(cancel);
        }
        self.files.dirty = true;
        tracing::info!(peer = %short_id(&peer.0), "receiving files");
        let tx = self.files.tx.clone();
        tokio::spawn(async move {
            let result = files::receive(stream, manifest_len, rjob, progress, rx).await;
            let _ = tx.send(Xfer::Received { serial, result });
        });
    }

    fn on_received(&mut self, serial: u64, result: Result<Received, RecvError>) {
        let Some(job) = self.files.fetch.take_if(|f| f.serial == serial) else {
            // Cancelled meanwhile: a finished folder that nobody wants goes.
            if let Ok(r) = result {
                remove_later(r.folder);
            }
            return;
        };
        self.files.dirty = true;
        let (id, peer) = (job.id, job.peer);
        let r = match result {
            Ok(r) => r,
            Err(e) => {
                tracing::info!(peer = %short_id(&peer.0), error = ?e, "receive ended");
                let failure = match e {
                    RecvError::Cancelled => return,
                    RecvError::BadData => TransferFailure::BadData,
                    RecvError::NoSpace => TransferFailure::NoSpace,
                    RecvError::WriteFailed => TransferFailure::WriteFailed,
                    RecvError::Interrupted => TransferFailure::Interrupted,
                    // A sender that moved on: asking again gets its cancel, or the new copy.
                    RecvError::Reset(c) if c == u32::from(uc_core::files::CODE_SUPERSEDED) => {
                        TransferFailure::Interrupted
                    }
                    RecvError::Reset(_) => {
                        let name = self.name_of(&peer);
                        self.files.fail_note = Some(format!("{name} stopped sending the files."));
                        TransferFailure::BadData
                    }
                };
                let evs = self.engine.transfer_failed(id, failure);
                self.on_engine(evs);
                return;
            }
        };
        if !self.engine.stream_allowed(peer, id) {
            // A newer copy came while the last bytes landed.
            remove_later(r.folder);
            return;
        }
        let seq = match uc_clip::Clipboard::write_files(&mut self.clip, &r.top, r.image.as_deref())
        {
            Ok(seq) => seq,
            Err(e) => {
                tracing::warn!(error = %e, "received files could not go on the clipboard");
                remove_later(r.folder);
                self.files.fail_note = Some("The files could not be put on the clipboard.".into());
                let evs = self
                    .engine
                    .transfer_failed(id, TransferFailure::WriteFailed);
                self.on_engine(evs);
                return;
            }
        };
        let short = id.short();
        self.files.current_folder = Some(short);
        self.files.long_paths = r.long_paths;
        let evs = self.engine.files_applied(id, &r.manifest, seq);
        self.on_engine(evs);
        self.files.long_paths = false;
        self.clean_files_folder();
    }

    /// Keep the last [`files::KEEP`] received copies, and every folder still in use.
    fn clean_files_folder(&self) {
        let mut keep: HashSet<String> = HashSet::new();
        keep.extend(self.files.current_folder.clone());
        if let Some(l) = &self.files.local {
            keep.extend(
                l.roots
                    .iter()
                    .filter_map(|p| files::folder_of(&self.files.dir, p)),
            );
        }
        let dir = self.files.dir.clone();
        tokio::task::spawn_blocking(move || {
            let n = files::keep_last(&dir, &keep);
            if n > 0 {
                tracing::info!(folders = n, "old received files deleted");
            }
        });
    }

    fn fetch_failed(&mut self, id: FilesId, peer: DeviceId, failure: TransferFailure) {
        if self.files.fetch.as_ref().is_some_and(|f| f.id == id) {
            self.drop_fetch();
        }
        let name = self.name_of(&peer);
        let note = self
            .files
            .fail_note
            .take()
            .unwrap_or_else(|| match failure {
                TransferFailure::NoSpace => format!("Not enough disk space for files from {name}."),
                TransferFailure::BadData => format!("The files from {name} did not arrive intact."),
                TransferFailure::WriteFailed => {
                    format!("The files from {name} could not be saved.")
                }
                TransferFailure::Interrupted | TransferFailure::Retries => {
                    format!("The connection to {name} broke too often. Copy the files again.")
                }
            });
        tracing::warn!(peer = %short_id(&peer.0), failure = ?failure, "files not received");
        self.files.last_error = Some((note.clone(), now_ms()));
        self.activity("Files", "not received", Some(name), Some(note));
    }

    // ------------------------------------------------------------ cancel and stop

    fn cancel_transfer(
        &mut self,
        id: FilesId,
        peer: DeviceId,
        reason: CancelReason,
        sending: bool,
    ) {
        self.files.dirty = true;
        let code = u32::from(reason.code());
        if sending {
            if let Some(s) = self.files.sends.remove(&(peer, id)) {
                let _ = s.cancel.send(code);
                tracing::info!(peer = %short_id(&peer.0), reason = ?reason, "send cancelled");
            }
            return;
        }
        let Some(f) = self.files.fetch.take_if(|f| f.id == id) else {
            return;
        };
        if let Some(c) = &f.cancel {
            let _ = c.send(code);
        }
        tracing::info!(peer = %short_id(&peer.0), reason = ?reason, "fetch cancelled");
        if self.files.quiet {
            return;
        }
        let name = self.name_of(&peer);
        let note = match reason {
            // The sender's `FilesCancel` for a newer copy can come before that copy does.
            CancelReason::Superseded
            | CancelReason::Requested
            | CancelReason::ByPeer(uc_core::files::CODE_SUPERSEDED) => {
                "A newer copy replaced these files.".to_string()
            }
            CancelReason::Stopped => "Stopped.".into(),
            CancelReason::Paused => "Sync was paused.".into(),
            CancelReason::FilesOff => "Files are not synced (setting is off).".into(),
            CancelReason::ByPeer(_) => format!("{name} stopped sending the files."),
        };
        self.activity(
            &files_kind(f.count, f.total),
            "not received",
            Some(name),
            Some(note),
        );
    }

    /// IPC `transfer.stop` (the Stop link, `uc transfer stop`): end the walk, the fetch and
    /// every send. Returns whether anything ran.
    pub(super) fn transfer_stop(&mut self) -> Value {
        let was =
            self.files.walk.is_some() || !self.files.sends.is_empty() || self.files.fetch.is_some();
        self.files.cancel_walk();
        let evs = self.engine.stop_transfer();
        self.on_engine(evs);
        // A fetch only asked for (no stream yet) has no engine cancel left to come.
        self.drop_fetch();
        if was {
            tracing::info!("transfer stopped by the user");
        }
        json!({ "stopped": was })
    }

    /// The link to `peer` is gone or replaced: its sends and its receive die with it. The
    /// engine already ended them (and asks again for a fetch), so no serve_done follows.
    pub(super) fn files_peer_gone(&mut self, peer: DeviceId) {
        let before = self.files.sends.len();
        self.files.sends.retain(|(p, _), _| *p != peer);
        if self.files.sends.len() != before {
            self.files.dirty = true;
        }
        if self.files.fetch.as_ref().is_some_and(|f| f.peer == peer) {
            self.drop_fetch();
        }
    }

    /// A new engine: every file job ends.
    pub(super) fn files_reset(&mut self) {
        self.files.cancel_walk();
        self.files.sends.clear();
        self.drop_fetch();
        self.files.local = None;
        self.files.current_folder = None;
        self.files.dirty = true;
    }

    // ------------------------------------------------------------ IPC

    /// IPC `files.copy` ("Copy files..." and "Copy folder..." in the window, `uc copy`): make
    /// these items the latest copy, at any size (spec 2.1.4).
    pub(super) fn files_copy(&mut self, p: &Value) -> Result<Value, IpcError> {
        let list = p
            .get("paths")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if list.is_empty() {
            return Err(IpcError::new(
                "no-paths",
                "Give at least one file or folder.",
            ));
        }
        let mut paths = Vec::with_capacity(list.len());
        for v in list {
            let Some(s) = v.as_str() else {
                return Err(IpcError::new("bad-path", "Paths must be text."));
            };
            let path = PathBuf::from(s);
            if !path.is_absolute() {
                return Err(IpcError::new(
                    "bad-path",
                    format!("\"{s}\" is not a full path."),
                ));
            }
            if std::fs::symlink_metadata(&path).is_err() {
                return Err(IpcError::new(
                    "not-found",
                    format!("There is no file or folder \"{s}\"."),
                ));
            }
            paths.push(path);
        }
        let n = paths.len();
        let evs = self.engine.pick_files(paths);
        for e in &evs {
            match e {
                EngineEvent::Skipped(SkipReason::Paused) => {
                    return Err(IpcError::new(
                        "paused",
                        "Sync is paused. Turn it on first: uc resume",
                    ));
                }
                EngineEvent::Skipped(SkipReason::FilesOff) => {
                    return Err(IpcError::new(
                        "files-off",
                        "Files are not synced. Turn it on: uc settings set sync-files on",
                    ));
                }
                EngineEvent::Skipped(_) => {
                    return Err(IpcError::new(
                        "no-paths",
                        "Give at least one file or folder.",
                    ));
                }
                _ => {}
            }
        }
        self.on_engine(evs);
        Ok(json!({ "preparing": true, "items": n }))
    }

    // ------------------------------------------------------------ progress

    /// What moves now, for the window and `uc status`: `{id, phase, device, done, total,
    /// files}`, phase `preparing`, `sending` or `receiving`. `None` when nothing runs. A fetch
    /// comes first, then a walk, then the sends (summed).
    pub(super) fn transfer_view(&self) -> Option<Value> {
        if let Some(f) = &self.files.fetch {
            let (done, _) = f.progress.get();
            return Some(json!({
                "id": f.id.short(),
                "phase": "receiving",
                "device": self.name_of(&f.peer),
                "done": done,
                "total": f.total,
                "files": f.count,
            }));
        }
        if let Some(w) = &self.files.walk {
            let (done, files) = w.progress.get();
            return Some(json!({
                "id": null,
                "phase": "preparing",
                "device": null,
                "done": done,
                "total": w.total,
                "files": files,
            }));
        }
        if self.files.sends.is_empty() {
            return None;
        }
        let mut peers: Vec<DeviceId> = self.files.sends.keys().map(|(p, _)| *p).collect();
        peers.sort();
        peers.dedup();
        let device = match peers.as_slice() {
            [one] => self.name_of(one),
            many => format!("{} devices", many.len()),
        };
        let (done, total) = self.files.sends.values().fold((0u64, 0u64), |(d, t), s| {
            (d + s.progress.get().0, t + s.total)
        });
        let local = self.files.local.as_ref();
        Some(json!({
            "id": local.map(|l| l.id.short()),
            "phase": "sending",
            "device": device,
            "done": done,
            "total": total,
            "files": local.map_or(0, |l| l.prepared.manifest.file_count()),
        }))
    }

    /// The `transfer` event, at most 4 per second (the progress tick): only when it changed.
    /// When nothing runs any more it says `phase: "idle"` once.
    pub(super) fn emit_transfer(&mut self) {
        self.files.dirty = false;
        let v = self.transfer_view().unwrap_or_else(|| {
            json!({"id": null, "phase": "idle", "device": null, "done": 0, "total": 0, "files": 0})
        });
        if self.files.shown.as_ref() != Some(&v) {
            self.files.shown = Some(v.clone());
            self.emit("transfer", v);
        }
    }

    /// `last_transfer_error` of `uc status --json`.
    pub(super) fn last_transfer_error(&self) -> Value {
        self.files.last_error.as_ref().map_or(
            Value::Null,
            |(text, at)| json!({"text": text, "time_ms": at}),
        )
    }

    // ------------------------------------------------------------ jobs

    /// A file job ended.
    pub(super) fn on_xfer(&mut self, m: Xfer) {
        match m {
            Xfer::Walked { generation, result } => self.on_walked(generation, result),
            Xfer::Prepared { generation, result } => self.on_prepared(generation, result),
            Xfer::Head {
                peer,
                link,
                head,
                stream,
            } => self.on_head(peer, link, head, stream),
            Xfer::Served {
                peer,
                id,
                serial,
                result,
            } => self.on_served(peer, id, serial, result),
            Xfer::Received { serial, result } => self.on_received(serial, result),
        }
    }

    /// At start: make the files folder and sweep `.part` leftovers and old folders.
    pub(super) fn files_start(&self) {
        let dir = self.files.dir.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = std::fs::create_dir_all(&dir) {
                tracing::warn!(error = %e, "files folder could not be made");
                return;
            }
            let n = files::sweep(&dir, std::time::SystemTime::now());
            if n > 0 {
                tracing::info!(folders = n, "old or partial received files deleted");
            }
        });
    }
}

/// Delete a folder off the main loop.
fn remove_later(folder: PathBuf) {
    tokio::task::spawn_blocking(move || {
        let _ = std::fs::remove_dir_all(folder);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_for_skipped_file_copies() {
        assert_eq!(
            skip_note(&SkipReason::FilesOverLimit, 100).as_deref(),
            Some("Files over 100 MB are not sent automatically. Use Copy files... in the app.")
        );
        assert_eq!(
            skip_note(&SkipReason::FilesOff, 100).as_deref(),
            Some("Files are not synced (setting is off).")
        );
        for r in [
            SkipReason::FilesVirtual,
            SkipReason::FilesCloudOnly,
            SkipReason::FilesTooMany,
        ] {
            let n = skip_note(&r, 1).unwrap();
            assert!(n.ends_with('.'), "{n}");
        }
        assert_eq!(skip_note(&SkipReason::Private, 1), None);
        assert_eq!(skip_note(&SkipReason::TooLarge, 1), None);
        assert!(
            prep_note(PrepError::Changed)
                .unwrap()
                .contains("Copy it again")
        );
        assert_eq!(prep_note(PrepError::Cancelled), None);
    }
}
