//! The sync state machine (plan 3.8), in both roles: full member and pull-only client.
//!
//! The engine does no I/O of its own. The caller (the agent, the mobile bindings or the test
//! harness) feeds it events and gives it a [`Clipboard`] to read and write:
//!
//! - [`Engine::connect`] when a connection to a peer is up, [`Engine::disconnect`] when it ends;
//! - [`Engine::on_local_change`] when the OS clipboard changed (after the settle time);
//! - [`Engine::on_frame`] for every frame a peer sent;
//! - [`Engine::fetch`] for an explicit fetch (app opened on a phone, `uclip fetch`).
//!
//! Every call returns [`Event`]s for "Last activity", logs and error states. Frames to peers go
//! out through the [`Peer`] trait.
//!
//! Rules in short:
//! - Each device holds only the newest clip, in memory. Highest stamp wins.
//! - Clips up to [`caps::PUSH_MAX`] are pushed; larger ones are announced with `Latest`, and the
//!   peer asks with `Want`. Pull-only peers always get `Latest` and ask.
//! - On connect both sides send `Hello` and `Latest`.
//! - Automatic apply only if the clip is less than 1 hour old (full member). A pull-only client
//!   and an explicit fetch take the newest clip with no age limit.
//! - Four echo and loop layers, see [`SkipReason`].
//!
//! File copies (file sync spec 4). The engine decides; the agent walks, hashes and moves bytes:
//! - The current copy is a clip or a file copy ([`Current`]); both carry a clock stamp and the
//!   highest stamp wins, whatever the kind.
//! - A local file copy starts a walk ([`Event::WalkFiles`], with a generation number). Any
//!   newer local copy or applied remote copy ends the walk, so a stale result is dropped. Only
//!   when the walk and hashes are done ([`Engine::on_local_files`]) does the copy get a stamp.
//! - Only the origin offers and serves a file copy, and only to peers whose engine `Hello` has
//!   the files bit. A receiver keeps one pending offer and fetches it if it is the newest copy.
//! - At most [`SERVE_MAX`] sends run at once; more wait in a FIFO queue.
//! - Any newer copy cancels the pending fetch and the running sends
//!   ([`Event::CancelTransfer`]). A fetch cut by a lost link is asked again from the start
//!   when the origin is back, at most [`FETCH_RETRIES`] times.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;

use uc_clip::{Clipboard, Format, RawPart};

use crate::caps::{self, Dropped};
use crate::clip::{Clip, ClipError, ClipKind};
use crate::files::{self, FilesSummary, Manifest};
use crate::hlc::{FUTURE_LIMIT_MS, Hlc, Stamp};
use crate::ids::{ClipId, ContentHash, DeviceId, FilesId};
use crate::msg::{
    self, ClipHeader, CodecError, DeviceRecord, FilesOffer, Hello, HelloExt, Message, Role,
};
use crate::text;

/// Most file sends that run at once; more wait in a FIFO queue (file sync spec 4).
pub const SERVE_MAX: usize = 2;
/// How often a fetch cut by a lost link is asked again from the start (file sync spec 2.2.7).
pub const FETCH_RETRIES: u32 = 3;

/// Automatic apply limit: 1 hour.
pub const AUTO_APPLY_MAX_AGE_MS: u64 = 60 * 60 * 1000;

/// One connection to a peer. The transport (iroh in M2, the fake network in tests) implements
/// it. `send` must keep frame order on one connection.
pub trait Peer: Send {
    /// The peer's device ID (its public key, already checked to be a group member).
    fn id(&self) -> DeviceId;
    /// Send one frame. An error means the connection is gone.
    fn send(&mut self, frame: Vec<u8>) -> Result<(), PeerGone>;
}

/// The connection to a peer is gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeerGone;

/// Settings of one engine.
#[derive(Clone, Debug)]
pub struct Config {
    /// This device.
    pub device: DeviceRecord,
    /// Full member or pull-only client.
    pub role: Role,
    /// App version sent in `Hello`.
    pub app_version: String,
    /// Membership chain heads sent in `Hello` (M3 keeps them up to date).
    pub chain_heads: Vec<[u8; 32]>,
    /// Group secret epoch sent in `Hello`.
    pub epoch: u64,
    /// Automatic apply age limit.
    pub auto_apply_max_age_ms: u64,
    /// Future stamp limit.
    pub future_limit_ms: u64,
    /// Push limit.
    pub push_max: usize,
    /// "Sync images" setting.
    pub sync_images: bool,
    /// Largest image part that is sent, in bytes (setting `image-limit-mb`; at most
    /// [`caps::IMAGE_MAX`]).
    pub image_max: usize,
    /// macOS: pass on clips that came through Apple's Universal Clipboard (plan decision 8).
    /// Off by default (M7: decision 8 is still open, off is the safer choice).
    pub forward_remote: bool,
    /// Group ID sent in `Hello` (protocol 5.9).
    pub group: [u8; 32],
    /// "Copy smaller files automatically" setting (file sync spec 5). Off only stops automatic
    /// file copies made on this device (Ctrl+C / Cmd+C of files): picked copies, receiving and
    /// serving a copy already offered go on.
    pub auto_file_copy: bool,
    /// Largest file copy that syncs automatically, in bytes (setting `file-limit-mb`).
    /// Picked copies have no byte limit.
    pub file_max: u64,
    /// Feature bits sent in the `Hello` extension (tests turn them off to act as a 0.1.x peer).
    pub features: u32,
}

impl Config {
    /// Defaults from the plan.
    pub fn new(device: DeviceRecord, role: Role) -> Self {
        Self {
            device,
            role,
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            chain_heads: Vec::new(),
            epoch: 0,
            auto_apply_max_age_ms: AUTO_APPLY_MAX_AGE_MS,
            future_limit_ms: FUTURE_LIMIT_MS,
            push_max: caps::PUSH_MAX,
            sync_images: true,
            image_max: caps::IMAGE_MAX,
            forward_remote: false,
            group: [0; 32],
            auto_file_copy: true,
            file_max: caps::file_limit_bytes(caps::FILE_LIMIT_DEFAULT_MB),
            features: msg::FEATURES,
        }
    }
}

/// Why a local clipboard change was not sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// Sync is paused.
    Paused,
    /// The clipboard could not be read.
    ReadFailed,
    /// Echo layer 1: our private marker is on the clipboard.
    Marker,
    /// Echo layer 2: the change counter is the one of our own write.
    OwnWrite,
    /// Echo layer 3: these exact bytes are the current clip (the one held, sent or received).
    SeenId,
    /// Echo layer 4: this content is the current clip, re-encoded (for example by a clipboard
    /// manager). An older clip copied again is a new copy and is sent.
    SeenContent,
    /// The source marked it as private (password manager).
    Private,
    /// An automatic file copy (Ctrl+C / Cmd+C of files), and "Copy smaller files
    /// automatically" is off.
    FilesOff,
    /// A file copy over the automatic limit ("Use Copy files... in the app").
    FilesOverLimit,
    /// Virtual files without a path (Outlook attachment, zip item, promised file).
    FilesVirtual,
    /// Cloud placeholders that are not on disk: reading them would download them.
    FilesCloudOnly,
    /// Over the entry, depth or manifest limits.
    FilesTooMany,
    /// This peer cannot receive files (its `Hello` has no files bit): "needs an update".
    PeerNoFiles(DeviceId),
    /// macOS: it came from another Apple device through Apple's Universal Clipboard, and
    /// passing such clips on is off (plan decision 8).
    Remote,
    /// No synced format on the clipboard (or only images with "Sync images" off).
    Empty,
    /// Every part was over its cap.
    TooLarge,
    /// Invalid content (not UTF-8, bad image).
    Invalid(ClipError),
}

/// Result of [`Engine::fetch`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchOutcome {
    /// No full member is connected: "No computer is on right now on this network."
    NoMemberOnline,
    /// A newer clip was asked for. It is applied when it arrives, with no age limit.
    Requested {
        /// The peer that was asked.
        from: DeviceId,
        /// The clip.
        id: ClipId,
    },
    /// The newest clip was already held and is now on the clipboard.
    Applied(ClipId),
    /// The clipboard already has the group's newest clip.
    AlreadyCurrent,
    /// Nobody holds a clip.
    NothingHeld,
}

/// Why a file transfer is cancelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelReason {
    /// A newer copy (of anything) replaced it.
    Superseded,
    /// The user pressed Stop (or `uc transfer stop`).
    Stopped,
    /// Sync was paused.
    Paused,
    /// The same peer asked for the same copy again: its old stream is reset first.
    Requested,
    /// The other side cancelled, with this code.
    ByPeer(u8),
}

impl CancelReason {
    /// The stream error code and `FilesCancel` reason for it (file sync spec 3).
    pub fn code(self) -> u8 {
        match self {
            CancelReason::Superseded | CancelReason::Requested => files::CODE_SUPERSEDED,
            CancelReason::Stopped | CancelReason::Paused => files::CODE_ABORTED,
            CancelReason::ByPeer(c) => c,
        }
    }
}

/// Why a fetch failed on the receiving side. The agent reports it with
/// [`Engine::transfer_failed`]; the engine reports it back in [`Event::TransferFailed`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferFailure {
    /// The link or stream broke (Wi-Fi blip, wake, network change). Asked again when the
    /// origin is back, at most [`FETCH_RETRIES`] times.
    Interrupted,
    /// The data did not match: manifest hash or rules, a file hash, more bytes than declared.
    BadData,
    /// Not enough disk space (also a quota).
    NoSpace,
    /// Writing the files failed for another reason.
    WriteFailed,
    /// Only from the engine: the link broke more than [`FETCH_RETRIES`] times.
    Retries,
}

/// Something that happened, for "Last activity", logs and error states. Never content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A local copy was sent.
    Sent {
        /// Wire ID.
        id: ClipId,
        /// Kind.
        kind: ClipKind,
        /// Total size.
        size: usize,
        /// To how many peers.
        peers: usize,
    },
    /// A local copy was not sent.
    Skipped(SkipReason),
    /// One part of a local copy was over its cap and left out.
    PartDropped(Dropped),
    /// A part could not be read (for example a broken image) and was left out. The other parts
    /// were still sent.
    PartInvalid {
        /// Which part.
        format: Format,
    },
    /// A clip from a peer was put on the clipboard.
    Applied {
        /// Wire ID.
        id: ClipId,
        /// Kind.
        kind: ClipKind,
        /// The device where the copy happened.
        origin: DeviceId,
    },
    /// A newer clip arrived but was only held, not applied (too old for automatic apply, or
    /// paused). An explicit fetch applies it.
    HeldOnly {
        /// Wire ID.
        id: ClipId,
    },
    /// Writing the clipboard failed.
    WriteFailed,
    /// A peer's stamp was more than 2 minutes ahead of our clock, so its clip was refused.
    RefusedFutureStamp {
        /// The peer.
        peer: DeviceId,
    },
    /// Our clock is more than 2 minutes ahead of this peer's, so it refuses our clips.
    /// The UI shows "The date and time on this device are wrong".
    OwnClockAhead {
        /// The peer.
        peer: DeviceId,
    },
    /// A frame could not be decoded. The caller should close the connection.
    BadFrame {
        /// The peer.
        peer: DeviceId,
        /// Why.
        error: CodecError,
    },
    /// A clip from a peer was invalid and dropped.
    BadClip {
        /// The peer.
        peer: DeviceId,
        /// Why.
        error: ClipError,
    },
    /// A peer said hello.
    PeerHello {
        /// The peer.
        peer: DeviceId,
        /// Its role.
        role: Role,
        /// Its protocol version.
        protocol: u8,
    },
    /// A connection failed while sending and was dropped.
    PeerLost {
        /// The peer.
        peer: DeviceId,
    },
    /// A peer told us we were removed from the group.
    Removed {
        /// The peer.
        by: DeviceId,
    },
    /// Membership changes for M3 to process.
    Membership {
        /// The peer.
        peer: DeviceId,
        /// Encoded changes.
        changes: Vec<Vec<u8>>,
    },
    /// An address update for M2 to process.
    AddrUpdate {
        /// The peer.
        peer: DeviceId,
        /// Encoded, signed address list.
        signed: Vec<u8>,
    },
    /// A pull-only client tried to sync, but no full member is connected.
    NoMemberOnline,
    /// A file copy: walk these paths (folders recursively, sizes summed) off the main loop,
    /// then call [`Engine::on_walked`]. "Preparing files..."
    WalkFiles {
        /// The walk's generation. A newer copy ends it; a stale result is dropped.
        generation: u64,
        /// The copied (or picked) items, absolute.
        paths: Vec<PathBuf>,
        /// Picked with "Copy files..." or `uc copy`: no byte limit.
        picked: bool,
    },
    /// The walk passed the limits: hash the files, build the manifest, then call
    /// [`Engine::on_local_files`].
    HashFiles {
        /// The walk's generation.
        generation: u64,
    },
    /// A local file copy became the current copy and was offered.
    FilesSent {
        /// The copy.
        id: FilesId,
        /// Total bytes.
        total: u64,
        /// Number of files.
        count: u32,
        /// To how many peers.
        peers: usize,
    },
    /// Fetch this file copy from its origin: check the free space, then call
    /// [`Engine::files_want`] (or [`Engine::transfer_failed`] with `NoSpace`).
    FetchFiles {
        /// The origin.
        peer: DeviceId,
        /// The copy.
        id: FilesId,
        /// Total bytes.
        total: u64,
        /// Number of files.
        count: u32,
    },
    /// Open a data stream to `peer` and send this file copy. Call [`Engine::serve_done`] when
    /// the send ends, however it ends.
    ServeFiles {
        /// The receiver.
        peer: DeviceId,
        /// The copy.
        id: FilesId,
    },
    /// Stop a transfer: reset the stream (sending) or stop it and delete the partial folder
    /// (receiving), with [`CancelReason::code`].
    CancelTransfer {
        /// The copy.
        id: FilesId,
        /// The other side.
        peer: DeviceId,
        /// Why.
        reason: CancelReason,
        /// True for a send, false for a fetch.
        sending: bool,
    },
    /// A received file copy is the current copy (the agent already wrote the clipboard).
    FilesApplied {
        /// The copy.
        id: FilesId,
        /// The device where the copy happened.
        origin: DeviceId,
        /// Total bytes.
        total: u64,
        /// Number of files.
        count: u32,
    },
    /// A newer file copy was offered but is only remembered, not fetched (older than 1 hour,
    /// or paused).
    FilesHeldOnly {
        /// The copy.
        id: FilesId,
    },
    /// A fetch failed for good; the partial folder must be gone.
    TransferFailed {
        /// The copy.
        id: FilesId,
        /// The origin.
        peer: DeviceId,
        /// Why.
        failure: TransferFailure,
    },
}

struct PeerState {
    link: Box<dyn Peer>,
    role: Role,
    latest: Option<ClipHeader>,
    /// The `Hello` extension of the peer's engine `Hello`; `None` until it arrived.
    ext: Option<HelloExt>,
}

struct Held {
    clip: Clip,
    header: ClipHeader,
    /// `clip.content_hash()`, kept so each local change hashes only the new clip.
    content: ContentHash,
}

/// A file copy that is the current copy.
struct HeldFiles {
    offer: FilesOffer,
    /// Content hash of the manifest, for echo layer 4.
    content: ContentHash,
    /// The user stopped its sends: no more offers or sends of it.
    stopped: bool,
}

/// The current copy: the newest one this device knows and holds.
enum Current {
    Clip(Held),
    Files(HeldFiles),
}

impl Current {
    fn stamp(&self) -> Stamp {
        match self {
            Current::Clip(h) => h.header.stamp,
            Current::Files(f) => f.offer.stamp,
        }
    }
}

/// A local walk in progress.
#[derive(Clone, Copy)]
struct Walk {
    generation: u64,
    picked: bool,
}

/// Where a pending fetch stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fetch {
    /// Only remembered: too old, paused, stopped or failed. Never fetched.
    Remembered,
    /// `FetchFiles` was emitted; waiting for the agent's free space check.
    Asking,
    /// `FilesWant` was sent; the stream may come.
    Wanted,
    /// The link broke; waiting for the origin to come back.
    Waiting,
}

/// A file copy offered by a peer, newer than the current copy.
struct Pending {
    offer: FilesOffer,
    fetch: Fetch,
    /// Fetches cut by a lost link so far.
    interruptions: u32,
    /// A break was counted and no new link or stream came since. The broken stream and the
    /// link going down are often both reported for one blip (in either order), and the ask
    /// in between goes out on the dying link: that is still one break.
    charged: bool,
}

/// The sync state machine of one device.
pub struct Engine {
    cfg: Config,
    hlc: Hlc,
    current: Option<Current>,
    applied: Option<ClipId>,
    peers: BTreeMap<DeviceId, PeerState>,
    last_write_seq: Option<u64>,
    explicit: BTreeSet<ClipId>,
    paused: bool,
    walk: Option<Walk>,
    next_gen: u64,
    pending: Option<Pending>,
    /// Sends that run, as (receiver, copy).
    running: Vec<(DeviceId, FilesId)>,
    /// Sends that wait, oldest first.
    queue: VecDeque<(DeviceId, FilesId)>,
}

impl Engine {
    /// A new engine with nothing held.
    pub fn new(cfg: Config) -> Self {
        let hlc = Hlc::new(cfg.device.id).with_future_limit(cfg.future_limit_ms);
        Self {
            cfg,
            hlc,
            current: None,
            applied: None,
            peers: BTreeMap::new(),
            last_write_seq: None,
            explicit: BTreeSet::new(),
            paused: false,
            walk: None,
            next_gen: 0,
            pending: None,
            running: Vec::new(),
            queue: VecDeque::new(),
        }
    }

    /// This device's ID.
    pub fn id(&self) -> DeviceId {
        self.cfg.device.id
    }

    /// This device's role.
    pub fn role(&self) -> Role {
        self.cfg.role
    }

    /// The header of the newest clip held, if any (`None` while the current copy is files).
    pub fn held(&self) -> Option<&ClipHeader> {
        self.held_ref().map(|h| &h.header)
    }

    /// The newest clip held, if any (`None` while the current copy is files).
    pub fn held_clip(&self) -> Option<&Clip> {
        self.held_ref().map(|h| &h.clip)
    }

    /// The current file copy, if the current copy is files.
    pub fn current_files(&self) -> Option<&FilesOffer> {
        match &self.current {
            Some(Current::Files(f)) => Some(&f.offer),
            _ => None,
        }
    }

    /// The pending file copy offered by a peer (being fetched, or only remembered).
    pub fn pending_files(&self) -> Option<&FilesOffer> {
        self.pending.as_ref().map(|p| &p.offer)
    }

    /// The copy being fetched right now (asked for or wanted), if any.
    pub fn fetching(&self) -> Option<FilesId> {
        self.pending
            .as_ref()
            .filter(|p| matches!(p.fetch, Fetch::Asking | Fetch::Wanted))
            .map(|p| p.offer.id)
    }

    /// The sends that run, as (receiver, copy).
    pub fn running_sends(&self) -> &[(DeviceId, FilesId)] {
        &self.running
    }

    /// The sends that wait, oldest first.
    pub fn queued_sends(&self) -> Vec<(DeviceId, FilesId)> {
        self.queue.iter().copied().collect()
    }

    /// The generation of the walk in progress, if any.
    pub fn walk_gen(&self) -> Option<u64> {
        self.walk.map(|w| w.generation)
    }

    /// The `Hello` extension this device sends. The agent's gate `Hello` uses it too, so both
    /// `Hello`s carry the same features (file sync spec 3).
    pub fn hello_ext(&self) -> HelloExt {
        HelloExt {
            features: self.cfg.features,
        }
    }

    /// The wire ID of the last clip this engine put on, or sent from, the clipboard.
    pub fn applied(&self) -> Option<ClipId> {
        self.applied
    }

    /// Connected peers.
    pub fn peers(&self) -> impl Iterator<Item = DeviceId> + '_ {
        self.peers.keys().copied()
    }

    /// Pause or resume. While paused nothing is sent and received clips are held, not applied.
    /// Pausing ends a walk and cancels file transfers.
    pub fn set_paused(&mut self, paused: bool) -> Vec<Event> {
        let mut out = Vec::new();
        self.paused = paused;
        if paused {
            self.walk = None;
            self.halt_transfers(CancelReason::Paused, &mut out);
        }
        out
    }

    /// Change the "Copy smaller files automatically" setting. Off ends a running automatic
    /// (not picked) walk; picked walks, fetches and sends go on.
    pub fn set_auto_file_copy(&mut self, on: bool) {
        self.cfg.auto_file_copy = on;
        if !on && self.walk.is_some_and(|w| !w.picked) {
            self.walk = None;
        }
    }

    /// Change the largest file copy that syncs automatically, in bytes.
    pub fn set_file_max(&mut self, bytes: u64) {
        self.cfg.file_max = bytes;
    }

    /// Is sync paused?
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Change the "Sync images" setting.
    pub fn set_sync_images(&mut self, on: bool) {
        self.cfg.sync_images = on;
    }

    /// Change the largest image that is sent (capped at [`caps::IMAGE_MAX`]).
    pub fn set_image_max(&mut self, bytes: usize) {
        self.cfg.image_max = bytes.min(caps::IMAGE_MAX);
    }

    /// Change plan decision 8 (pass on Apple Universal Clipboard clips).
    pub fn set_forward_remote(&mut self, on: bool) {
        self.cfg.forward_remote = on;
    }

    /// Update the membership values sent in `Hello`.
    pub fn set_group(&mut self, group: [u8; 32], heads: Vec<[u8; 32]>, epoch: u64) {
        self.cfg.group = group;
        self.cfg.chain_heads = heads;
        self.cfg.epoch = epoch;
    }

    /// This device's record (name and kind), as sent in `Hello`.
    pub fn set_device(&mut self, device: DeviceRecord) {
        self.cfg.device = device;
    }

    /// The settings in use.
    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// A connection to a peer is up. Sends `Hello` and `Latest`. A second connection with the
    /// same peer replaces the first.
    pub fn connect(&mut self, link: Box<dyn Peer>, now_ms: u64) -> Vec<Event> {
        let mut out = Vec::new();
        let id = link.id();
        // A replaced link takes its transfers with it.
        if self.peers.remove(&id).is_some() {
            self.link_lost(id, &mut out);
        }
        // A new link to the origin: the next break of a fetch from it counts again.
        if let Some(p) = self.pending.as_mut().filter(|p| p.offer.origin == id) {
            p.charged = false;
        }
        self.peers.insert(
            id,
            PeerState {
                link,
                role: Role::Full,
                latest: None,
                ext: None,
            },
        );
        let hello = Message::Hello(Hello {
            protocol: msg::PROTOCOL_VERSION,
            app_version: self.cfg.app_version.clone(),
            device: self.cfg.device.clone(),
            role: self.cfg.role,
            chain_heads: self.cfg.chain_heads.clone(),
            epoch: self.cfg.epoch,
            wall_ms: now_ms,
            group: self.cfg.group,
            ext: self.hello_ext(),
        });
        self.send(id, &hello, &mut out);
        let latest = self.latest_msg();
        self.send(id, &latest, &mut out);
        out
    }

    /// The connection to a peer ended. Its sends end; a fetch from it waits for it to come
    /// back.
    pub fn disconnect(&mut self, peer: DeviceId) -> Vec<Event> {
        let mut out = Vec::new();
        if self.peers.remove(&peer).is_some() {
            self.link_lost(peer, &mut out);
        }
        out
    }

    /// The local clipboard changed. Read it and, if it passes every check, send it.
    pub fn on_local_change(&mut self, cb: &mut dyn Clipboard, now_ms: u64) -> Vec<Event> {
        let mut out = Vec::new();
        if self.paused {
            out.push(Event::Skipped(SkipReason::Paused));
            return out;
        }
        let snap = match cb.read() {
            Ok(s) => s,
            Err(_) => {
                out.push(Event::Skipped(SkipReason::ReadFailed));
                return out;
            }
        };
        // Layer 2: the change counter of our own write.
        if self.last_write_seq == Some(snap.seq) {
            out.push(Event::Skipped(SkipReason::OwnWrite));
            return out;
        }
        // Layer 1: our marker.
        if snap.has_marker {
            out.push(Event::Skipped(SkipReason::Marker));
            return out;
        }
        // A new copy by the user: it ends any walk still running.
        self.walk = None;
        if snap.private {
            out.push(Event::Skipped(SkipReason::Private));
            return out;
        }
        // Decision 8 holds for file copies too: what Apple's Universal Clipboard brought is
        // not forwarded unless the setting says so.
        if snap.remote && !self.cfg.forward_remote {
            out.push(Event::Skipped(SkipReason::Remote));
            return out;
        }
        // A file copy: its text and image parts (a file name, an icon) are ignored.
        if snap.has_files || snap.files_virtual || !snap.files.is_empty() {
            if !self.cfg.auto_file_copy {
                out.push(Event::Skipped(SkipReason::FilesOff));
            } else if snap.files.is_empty() {
                out.push(Event::Skipped(SkipReason::FilesVirtual));
            } else {
                out.push(self.start_walk(snap.files, false));
            }
            return out;
        }
        let mut parts: Vec<RawPart> = Vec::with_capacity(snap.parts.len());
        for (format, data) in snap.parts {
            if parts.iter().any(|(f, _)| *f == format) {
                continue;
            }
            match format {
                Format::TextPlain | Format::TextHtml => match text::to_wire(&data) {
                    // Empty text is no copy: sending it would wipe every other clipboard.
                    Some(s) if s.is_empty() => {}
                    Some(s) => parts.push((format, s.into_bytes())),
                    // HTML in another encoding (UTF-16 from some apps) is only extra: leave it
                    // out and still send the plain text.
                    None if format == Format::TextHtml => {}
                    None => {
                        out.push(Event::Skipped(SkipReason::Invalid(ClipError::NotUtf8)));
                        return out;
                    }
                },
                Format::ImagePng if !self.cfg.sync_images => {}
                // A local image that would be too costly to check is left out; the rest goes.
                Format::ImagePng if crate::image::check_local(&data).is_err() => {
                    out.push(Event::PartInvalid { format });
                }
                Format::ImagePng if data.len() > self.cfg.image_max => {
                    out.push(Event::PartDropped(Dropped {
                        format,
                        size: data.len(),
                    }));
                }
                Format::ImagePng => parts.push((format, data)),
            }
        }
        if parts.is_empty() {
            out.push(Event::Skipped(SkipReason::Empty));
            return out;
        }
        let (parts, dropped) = caps::fit(parts);
        let over_cap = dropped.iter().any(|d| d.size > caps::cap(d.format));
        out.extend(dropped.into_iter().map(Event::PartDropped));
        if parts.is_empty() {
            // Only "too large" when a part really was over its cap. HTML with no text twin
            // is left out by rule, which is an empty copy, not a large one.
            out.push(Event::Skipped(if over_cap {
                SkipReason::TooLarge
            } else {
                SkipReason::Empty
            }));
            return out;
        }
        // If the image turns out broken, the text must still go: keep the other parts aside
        // (text is small; the image is not copied).
        let without_image: Vec<RawPart> = if parts.len() > 1 {
            parts
                .iter()
                .filter(|(f, _)| *f != Format::ImagePng)
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        let clip = match Clip::from_parts(parts) {
            Ok(c) => c,
            Err(ClipError::Image(_)) if !without_image.is_empty() => {
                match Clip::from_parts(without_image) {
                    Ok(c) => {
                        out.push(Event::PartInvalid {
                            format: Format::ImagePng,
                        });
                        c
                    }
                    Err(e) => {
                        out.push(Event::Skipped(SkipReason::Invalid(e)));
                        return out;
                    }
                }
            }
            Err(e) => {
                out.push(Event::Skipped(SkipReason::Invalid(e)));
                return out;
            }
        };
        // Layers 3 and 4 compare with the current clip only. A clip received (or sent) and then
        // seen again on this clipboard, as is or re-encoded, is never sent back. An older clip
        // that the user copies again is a new copy: it is sent, so the other devices get it
        // (bug 2026-11-15: a link copied a second time never reached the Mac).
        if self.held_id() == Some(clip.id()) {
            out.push(Event::Skipped(SkipReason::SeenId));
            return out;
        }
        let content = clip.content_hash();
        if self.held_ref().is_some_and(|h| h.content == content) {
            out.push(Event::Skipped(SkipReason::SeenContent));
            return out;
        }

        let stamp = self.hlc.tick(now_ms);
        let header = ClipHeader {
            id: clip.id(),
            origin: self.id(),
            stamp,
            parts: clip.metas().to_vec(),
        };
        self.applied = Some(clip.id());
        let (id, kind, size) = (clip.id(), clip.kind(), clip.size());
        self.set_current(
            Current::Clip(Held {
                clip,
                header,
                content,
            }),
            &mut out,
        );

        let targets: Vec<DeviceId> = self.peers.keys().copied().collect();
        if self.cfg.role == Role::PullOnly && !self.has_full_peer() {
            out.push(Event::NoMemberOnline);
        }
        let mut sent = 0;
        for peer in targets {
            if self.offer_held(peer, &mut out) {
                sent += 1;
            }
        }
        out.push(Event::Sent {
            id,
            kind,
            size,
            peers: sent,
        });
        out
    }

    /// A frame arrived from `from`.
    pub fn on_frame(
        &mut self,
        from: DeviceId,
        frame: &[u8],
        cb: &mut dyn Clipboard,
        now_ms: u64,
    ) -> Vec<Event> {
        let mut out = Vec::new();
        if !self.peers.contains_key(&from) {
            return out;
        }
        let msg = match msg::decode(frame) {
            Ok(Some(m)) => m,
            Ok(None) => return out,
            Err(error) => {
                out.push(Event::BadFrame { peer: from, error });
                return out;
            }
        };
        match msg {
            Message::Hello(h) => self.on_hello(from, h, now_ms, &mut out),
            Message::Latest { header } => self.on_latest(from, header, now_ms, &mut out),
            Message::Clip { header, data } => {
                self.on_clip(from, header, data, cb, now_ms, &mut out)
            }
            Message::Want { id } => {
                if self.held_id() == Some(id) {
                    if let Some(m) = self.clip_msg() {
                        self.send(from, &m, &mut out);
                    }
                } else {
                    // We moved on: tell the peer what we hold now.
                    let m = self.latest_msg();
                    self.send(from, &m, &mut out);
                }
            }
            Message::Removed => out.push(Event::Removed { by: from }),
            // The agent routes these before the engine (protocol 5.8).
            Message::WantSecret { .. } | Message::Secret { .. } => {}
            Message::Membership { changes } => out.push(Event::Membership {
                peer: from,
                changes,
            }),
            Message::AddrUpdate { signed } => out.push(Event::AddrUpdate { peer: from, signed }),
            Message::FilesOffer(o) => self.on_files_offer(from, o, now_ms, &mut out),
            Message::FilesWant { id } => self.on_files_want(from, id, &mut out),
            Message::FilesCancel { id, reason } => self.on_files_cancel(from, id, reason, &mut out),
        }
        out
    }

    /// "Copy files..." in the window or `uc copy`: make `paths` the latest copy, at any size.
    /// Starts a walk like a file copy on the clipboard does.
    pub fn pick_files(&mut self, paths: Vec<PathBuf>) -> Vec<Event> {
        let reason = if self.paused {
            Some(SkipReason::Paused)
        } else if paths.is_empty() {
            Some(SkipReason::Empty)
        } else {
            None
        };
        match reason {
            Some(r) => vec![Event::Skipped(r)],
            None => vec![self.start_walk(paths, true)],
        }
    }

    /// The walk of generation `generation` found `total` bytes. Returns [`Event::HashFiles`] if the
    /// copy may go on, a skip if it is over the automatic limit, and nothing if the walk is
    /// stale. The limit is checked on the copying device only.
    pub fn on_walked(&mut self, generation: u64, total: u64) -> Vec<Event> {
        let Some(w) = self.walk.filter(|w| w.generation == generation) else {
            return Vec::new();
        };
        if !w.picked && total > self.cfg.file_max {
            self.walk = None;
            return vec![Event::Skipped(SkipReason::FilesOverLimit)];
        }
        vec![Event::HashFiles { generation }]
    }

    /// The walk of generation `generation` failed (cloud placeholders, too many entries, ...).
    /// Returns the skip, or nothing if the walk is stale.
    pub fn walk_failed(&mut self, generation: u64, reason: SkipReason) -> Vec<Event> {
        if self.walk.is_some_and(|w| w.generation == generation) {
            self.walk = None;
            vec![Event::Skipped(reason)]
        } else {
            Vec::new()
        }
    }

    /// The walk of generation `generation` is hashed and its manifest built. If the walk is still
    /// current, the copy gets a stamp, becomes the current copy and is offered.
    pub fn on_local_files(
        &mut self,
        generation: u64,
        summary: FilesSummary,
        now_ms: u64,
    ) -> Vec<Event> {
        let mut out = Vec::new();
        let Some(w) = self.walk.filter(|w| w.generation == generation) else {
            return out;
        };
        self.walk = None;
        let skip = if self.paused {
            Some(SkipReason::Paused)
        } else if !w.picked && !self.cfg.auto_file_copy {
            Some(SkipReason::FilesOff)
        } else if !w.picked && summary.total > self.cfg.file_max {
            Some(SkipReason::FilesOverLimit)
        } else if matches!(&self.current,
            Some(Current::Files(f)) if f.content == summary.content && !f.stopped)
        {
            // Layer 4: the same files as the current copy (received here, or copied twice).
            Some(SkipReason::SeenContent)
        } else {
            None
        };
        if let Some(r) = skip {
            out.push(Event::Skipped(r));
            return out;
        }
        let stamp = self.hlc.tick(now_ms);
        let offer = FilesOffer {
            id: files::files_id(&summary.manifest_hash, &stamp),
            origin: self.id(),
            stamp,
            total: summary.total,
            count: summary.count,
            manifest_hash: summary.manifest_hash,
            picked: w.picked,
        };
        self.applied = None;
        self.set_current(
            Current::Files(HeldFiles {
                offer: offer.clone(),
                content: summary.content,
                stopped: false,
            }),
            &mut out,
        );
        let targets: Vec<DeviceId> = self.peers.keys().copied().collect();
        let mut sent = 0;
        for peer in targets {
            if self.offer_files(peer, &mut out) {
                sent += 1;
            }
        }
        out.push(Event::FilesSent {
            id: offer.id,
            total: offer.total,
            count: offer.count,
            peers: sent,
        });
        out
    }

    /// The agent wrote the clipboard itself (the picked items after "Copy files..."). Echo
    /// layer 2 then skips that change.
    pub fn note_own_write(&mut self, seq: u64) {
        self.last_write_seq = Some(seq);
    }

    /// The free space check for [`Event::FetchFiles`] passed: ask the origin for the copy.
    pub fn files_want(&mut self, id: FilesId) -> Vec<Event> {
        let mut out = Vec::new();
        let Some(p) = self
            .pending
            .as_mut()
            .filter(|p| p.offer.id == id && p.fetch == Fetch::Asking)
        else {
            return out;
        };
        let origin = p.offer.origin;
        if !self.peers.contains_key(&origin) {
            p.fetch = Fetch::Waiting;
            return out;
        }
        p.fetch = Fetch::Wanted;
        self.send(origin, &Message::FilesWant { id }, &mut out);
        out
    }

    /// May a data stream from `peer` for `id` be accepted? Only for the outstanding
    /// `FilesWant` to that peer; any other stream is stopped at once (file sync spec 3).
    pub fn stream_allowed(&self, peer: DeviceId, id: FilesId) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|p| p.offer.id == id && p.offer.origin == peer && p.fetch == Fetch::Wanted)
    }

    /// The fetched files are in place and on the clipboard (written with `seq`). The copy
    /// becomes the current copy, and echo layer 2 skips the write.
    pub fn files_applied(&mut self, id: FilesId, manifest: &Manifest, seq: u64) -> Vec<Event> {
        let mut out = Vec::new();
        if !self
            .pending
            .as_ref()
            .is_some_and(|p| p.offer.id == id && p.fetch == Fetch::Wanted)
        {
            return out;
        }
        let Some(p) = self.pending.take() else {
            return out;
        };
        if manifest.hash() != p.offer.manifest_hash {
            self.pending = Some(p);
            return self.transfer_failed(id, TransferFailure::BadData);
        }
        // An applied remote copy ends a local walk.
        self.walk = None;
        self.last_write_seq = Some(seq);
        self.applied = None;
        let offer = p.offer;
        out.push(Event::FilesApplied {
            id,
            origin: offer.origin,
            total: offer.total,
            count: offer.count,
        });
        self.set_current(
            Current::Files(HeldFiles {
                offer,
                content: manifest.content_hash(),
                stopped: false,
            }),
            &mut out,
        );
        out
    }

    /// A fetch failed on this side. A broken link is asked again when the origin is back;
    /// anything else ends the fetch for good (and tells the origin).
    pub fn transfer_failed(&mut self, id: FilesId, failure: TransferFailure) -> Vec<Event> {
        let mut out = Vec::new();
        let Some(p) = self
            .pending
            .as_mut()
            .filter(|p| p.offer.id == id && matches!(p.fetch, Fetch::Asking | Fetch::Wanted))
        else {
            return out;
        };
        if failure == TransferFailure::Interrupted {
            self.interrupted(&mut out);
            return out;
        }
        let (origin, wanted) = (p.offer.origin, p.fetch == Fetch::Wanted);
        p.fetch = Fetch::Remembered;
        if wanted {
            let cancel = Message::FilesCancel {
                id,
                reason: files::CODE_ABORTED,
            };
            self.send(origin, &cancel, &mut out);
        }
        out.push(Event::TransferFailed {
            id,
            peer: origin,
            failure,
        });
        out
    }

    /// The stream the receiver asked for came and was accepted: a later break of it counts
    /// as a new one.
    pub fn stream_started(&mut self, peer: DeviceId, id: FilesId) {
        if let Some(p) = self
            .pending
            .as_mut()
            .filter(|p| p.offer.id == id && p.offer.origin == peer && p.fetch == Fetch::Wanted)
        {
            p.charged = false;
        }
    }

    /// A send failed before its stream could tell the receiver anything (no data link, the
    /// copy is gone here, or the stream could not be opened). The receiver gets a
    /// `FilesCancel` (failed, not aborted: nobody pressed Stop), so it does not wait for a stream that never comes; then the
    /// next queued send starts.
    pub fn serve_failed(&mut self, peer: DeviceId, id: FilesId) -> Vec<Event> {
        let mut out = Vec::new();
        if self.running.contains(&(peer, id)) || self.queue.contains(&(peer, id)) {
            let cancel = Message::FilesCancel {
                id,
                reason: files::CODE_FAILED,
            };
            self.send(peer, &cancel, &mut out);
        }
        out.extend(self.serve_done(peer, id));
        out
    }

    /// A send ended (done, failed or reset). The next queued send starts.
    pub fn serve_done(&mut self, peer: DeviceId, id: FilesId) -> Vec<Event> {
        let mut out = Vec::new();
        self.running.retain(|s| *s != (peer, id));
        self.queue.retain(|s| *s != (peer, id));
        self.pump(&mut out);
        out
    }

    /// Stop (the window's Stop link, `uc transfer stop`): cancel the running fetch and every
    /// send. A stopped copy is not fetched or offered again; a new copy starts afresh.
    pub fn stop_transfer(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        if let Some(Current::Files(f)) = &mut self.current
            && f.offer.origin == self.cfg.device.id
        {
            f.stopped = true;
        }
        self.halt_transfers(CancelReason::Stopped, &mut out);
        out
    }

    /// Explicit fetch: get the group's newest clip with no age limit. This runs when the phone
    /// app opens, or for `uclip fetch`. It is not a button.
    pub fn fetch(&mut self, cb: &mut dyn Clipboard, now_ms: u64) -> (FetchOutcome, Vec<Event>) {
        let mut out = Vec::new();
        if !self.has_full_peer() {
            out.push(Event::NoMemberOnline);
            return (FetchOutcome::NoMemberOnline, out);
        }
        // The best clip a peer told us about.
        let best_peer = self
            .peers
            .iter()
            .filter_map(|(id, p)| p.latest.as_ref().map(|h| (*id, h)))
            .filter(|(_, h)| self.hlc.check(&h.stamp, now_ms).is_ok())
            .max_by_key(|(_, h)| h.stamp)
            .map(|(id, h)| (id, h.id, h.stamp));
        let held_stamp = self.current.as_ref().map(Current::stamp);
        match (best_peer, held_stamp) {
            (Some((from, id, stamp)), held) if held.is_none_or(|s| stamp > s) => {
                self.explicit.insert(id);
                self.send(from, &Message::Want { id }, &mut out);
                (FetchOutcome::Requested { from, id }, out)
            }
            // A file copy is fetched on its own; there is nothing more to apply.
            (_, Some(_)) if self.held_ref().is_none() => (FetchOutcome::AlreadyCurrent, out),
            (_, Some(_)) => {
                let id = self.held_id().unwrap_or_default();
                if self.applied == Some(id) {
                    (FetchOutcome::AlreadyCurrent, out)
                } else {
                    self.apply_held(cb, &mut out);
                    (FetchOutcome::Applied(id), out)
                }
            }
            (_, None) => (FetchOutcome::NothingHeld, out),
        }
    }

    // ----- internals -----

    fn held_ref(&self) -> Option<&Held> {
        match &self.current {
            Some(Current::Clip(h)) => Some(h),
            _ => None,
        }
    }

    fn held_id(&self) -> Option<ClipId> {
        self.held_ref().map(|h| h.header.id)
    }

    fn start_walk(&mut self, paths: Vec<PathBuf>, picked: bool) -> Event {
        self.next_gen += 1;
        let generation = self.next_gen;
        self.walk = Some(Walk { generation, picked });
        Event::WalkFiles {
            generation,
            paths,
            picked,
        }
    }

    /// Make `new` the current copy. Running sends belong to the old one and end; a pending
    /// offer older than `new` is dropped.
    fn set_current(&mut self, new: Current, out: &mut Vec<Event>) {
        let stamp = new.stamp();
        self.cancel_sends(CancelReason::Superseded, out);
        if self.pending.as_ref().is_some_and(|p| p.offer.stamp < stamp) {
            self.drop_pending(CancelReason::Superseded, out);
        }
        self.current = Some(new);
    }

    /// End every send (running and queued) and tell each receiver.
    fn cancel_sends(&mut self, reason: CancelReason, out: &mut Vec<Event>) {
        let running = std::mem::take(&mut self.running);
        let queued: Vec<_> = self.queue.drain(..).collect();
        for &(peer, id) in &running {
            out.push(Event::CancelTransfer {
                id,
                peer,
                reason,
                sending: true,
            });
        }
        for (peer, id) in running.into_iter().chain(queued) {
            let cancel = Message::FilesCancel {
                id,
                reason: reason.code(),
            };
            self.send(peer, &cancel, out);
        }
    }

    /// Forget the pending offer, cancelling its fetch.
    fn drop_pending(&mut self, reason: CancelReason, out: &mut Vec<Event>) {
        if let Some(p) = self.pending.take() {
            self.cancel_fetch(&p, reason, out);
        }
    }

    /// Cancel a running fetch of `p` (if it runs) and tell the origin (if it was asked).
    fn cancel_fetch(&mut self, p: &Pending, reason: CancelReason, out: &mut Vec<Event>) {
        let (id, origin) = (p.offer.id, p.offer.origin);
        if matches!(p.fetch, Fetch::Asking | Fetch::Wanted) {
            out.push(Event::CancelTransfer {
                id,
                peer: origin,
                reason,
                sending: false,
            });
        }
        if p.fetch == Fetch::Wanted {
            let cancel = Message::FilesCancel {
                id,
                reason: reason.code(),
            };
            self.send(origin, &cancel, out);
        }
    }

    /// Stop or pause: cancel the fetch (the offer stays remembered) and
    /// every send.
    fn halt_transfers(&mut self, reason: CancelReason, out: &mut Vec<Event>) {
        if let Some(mut p) = self.pending.take() {
            self.cancel_fetch(&p, reason, out);
            p.fetch = Fetch::Remembered;
            self.pending = Some(p);
        }
        self.cancel_sends(reason, out);
    }

    /// Start queued sends while fewer than [`SERVE_MAX`] run.
    fn pump(&mut self, out: &mut Vec<Event>) {
        while self.running.len() < SERVE_MAX {
            let Some((peer, id)) = self.queue.pop_front() else {
                break;
            };
            if self.peers.contains_key(&peer) {
                self.running.push((peer, id));
                out.push(Event::ServeFiles { peer, id });
            }
        }
    }

    /// The link to `peer` is gone (closed or replaced). Its sends end; a fetch from it was
    /// interrupted.
    fn link_lost(&mut self, peer: DeviceId, out: &mut Vec<Event>) {
        self.running.retain(|(p, _)| *p != peer);
        self.queue.retain(|(p, _)| *p != peer);
        self.pump(out);
        if self.pending.as_ref().is_some_and(|p| {
            p.offer.origin == peer && matches!(p.fetch, Fetch::Asking | Fetch::Wanted)
        }) {
            self.interrupted(out);
        }
    }

    /// The running fetch broke. Ask again if the origin is still here, else wait for it; give
    /// up after [`FETCH_RETRIES`] breaks.
    fn interrupted(&mut self, out: &mut Vec<Event>) {
        let Some(p) = self.pending.as_mut() else {
            return;
        };
        // One blip is counted once, even when both the stream and the link report it.
        if p.fetch == Fetch::Wanted && !p.charged {
            p.interruptions += 1;
            p.charged = true;
        }
        if p.interruptions > FETCH_RETRIES {
            p.fetch = Fetch::Remembered;
            out.push(Event::TransferFailed {
                id: p.offer.id,
                peer: p.offer.origin,
                failure: TransferFailure::Retries,
            });
            return;
        }
        p.fetch = Fetch::Waiting;
        let origin = p.offer.origin;
        if self.peers.get(&origin).is_some_and(|s| s.ext.is_some()) {
            self.ask(out);
        }
    }

    /// Emit `FetchFiles` for the pending offer.
    fn ask(&mut self, out: &mut Vec<Event>) {
        if let Some(p) = self.pending.as_mut() {
            p.fetch = Fetch::Asking;
            out.push(Event::FetchFiles {
                peer: p.offer.origin,
                id: p.offer.id,
                total: p.offer.total,
                count: p.offer.count,
            });
        }
    }

    /// May an offer with this stamp be fetched now (not only remembered)?
    fn may_fetch(&self, stamp: &Stamp, now_ms: u64) -> bool {
        !self.paused && self.cfg.role == Role::Full && self.auto_apply_allowed(stamp, now_ms)
    }

    /// Offer the current file copy to `peer`, if this device is its origin and the peer can
    /// take files. A full member without the files bit gets a note instead. Returns true if
    /// the offer went out.
    fn offer_files(&mut self, peer: DeviceId, out: &mut Vec<Event>) -> bool {
        let offer = match &self.current {
            Some(Current::Files(f)) if f.offer.origin == self.cfg.device.id && !f.stopped => {
                f.offer.clone()
            }
            _ => return false,
        };
        if self.paused {
            return false;
        }
        // Nothing goes out before the peer's engine Hello: its features are not known yet.
        let Some((role, ext)) = self
            .peers
            .get(&peer)
            .and_then(|p| p.ext.map(|e| (p.role, e)))
        else {
            return false;
        };
        if role != Role::Full {
            return false;
        }
        if !ext.files() {
            out.push(Event::Skipped(SkipReason::PeerNoFiles(peer)));
            return false;
        }
        self.send(peer, &Message::FilesOffer(offer), out)
    }

    fn on_files_offer(
        &mut self,
        from: DeviceId,
        offer: FilesOffer,
        now_ms: u64,
        out: &mut Vec<Event>,
    ) {
        // Only the origin offers its copy.
        if offer.origin != from || !offer.is_consistent() {
            out.push(Event::BadClip {
                peer: from,
                error: ClipError::Mismatch,
            });
            return;
        }
        if self.hlc.check(&offer.stamp, now_ms).is_err() {
            out.push(Event::RefusedFutureStamp { peer: from });
            return;
        }
        // A later local copy must be newer than this offer, fetched or not.
        let _ = self.hlc.observe(&offer.stamp, now_ms);
        if !self.is_newer_stamp(&offer.stamp) {
            return;
        }
        if let Some(p) = &self.pending {
            if p.offer.id == offer.id {
                // The origin is back after a broken fetch: ask again from the start.
                if p.fetch == Fetch::Waiting && self.may_fetch(&offer.stamp, now_ms) {
                    self.ask(out);
                }
                return;
            }
            if p.offer.stamp >= offer.stamp {
                return;
            }
            self.drop_pending(CancelReason::Superseded, out);
        }
        // A newer copy exists: sends of the older current copy are of no use any more.
        self.cancel_sends(CancelReason::Superseded, out);
        let fetch_now = self.may_fetch(&offer.stamp, now_ms);
        let id = offer.id;
        self.pending = Some(Pending {
            offer,
            fetch: Fetch::Remembered,
            interruptions: 0,
            charged: false,
        });
        if fetch_now {
            self.ask(out);
        } else {
            out.push(Event::FilesHeldOnly { id });
        }
    }

    fn on_files_want(&mut self, from: DeviceId, id: FilesId, out: &mut Vec<Event>) {
        let ours = match &self.current {
            Some(Current::Files(f)) if f.offer.id == id => {
                Some(f.offer.origin == self.cfg.device.id && !f.stopped && !self.paused)
            }
            _ => None,
        };
        let full = self.peers.get(&from).is_some_and(|p| p.role == Role::Full);
        if ours != Some(true) || !full {
            // We moved on (superseded), or we do not serve this copy (aborted).
            let reason = if ours.is_some() {
                files::CODE_ABORTED
            } else {
                files::CODE_SUPERSEDED
            };
            self.send(from, &Message::FilesCancel { id, reason }, out);
            return;
        }
        if let Some(i) = self.running.iter().position(|s| *s == (from, id)) {
            // The peer asks again (it lost the stream): reset the old one first.
            self.running.remove(i);
            out.push(Event::CancelTransfer {
                id,
                peer: from,
                reason: CancelReason::Requested,
                sending: true,
            });
        } else if self.queue.contains(&(from, id)) {
            return;
        }
        self.queue.push_back((from, id));
        self.pump(out);
    }

    fn on_files_cancel(&mut self, from: DeviceId, id: FilesId, code: u8, out: &mut Vec<Event>) {
        let reason = CancelReason::ByPeer(code);
        if let Some(p) = self
            .pending
            .as_mut()
            .filter(|p| p.offer.id == id && p.offer.origin == from)
        {
            let active = matches!(p.fetch, Fetch::Asking | Fetch::Wanted);
            p.fetch = Fetch::Remembered;
            if active {
                out.push(Event::CancelTransfer {
                    id,
                    peer: from,
                    reason,
                    sending: false,
                });
            }
            return;
        }
        if let Some(i) = self.running.iter().position(|s| *s == (from, id)) {
            self.running.remove(i);
            out.push(Event::CancelTransfer {
                id,
                peer: from,
                reason,
                sending: true,
            });
            self.pump(out);
        }
        self.queue.retain(|s| *s != (from, id));
    }

    fn has_full_peer(&self) -> bool {
        self.peers.values().any(|p| p.role == Role::Full)
    }

    /// `Latest` for the current copy. While it is files, `Latest { header: None }`: files never
    /// appear in `Latest` or `Clip` (file sync spec 3).
    fn latest_msg(&self) -> Message {
        Message::Latest {
            header: self.held_ref().map(|h| h.header.clone()),
        }
    }

    fn clip_msg(&self) -> Option<Message> {
        self.held_ref().map(|h| Message::Clip {
            header: h.header.clone(),
            data: h.clip.parts().iter().map(|(_, d)| d.clone()).collect(),
        })
    }

    /// Push the held clip to one peer (or announce it). Returns false if the peer is gone.
    fn offer_held(&mut self, peer: DeviceId, out: &mut Vec<Event>) -> bool {
        let Some(role) = self.peers.get(&peer).map(|p| p.role) else {
            return false;
        };
        let small = self
            .held_ref()
            .is_some_and(|h| h.clip.size() <= self.cfg.push_max);
        let m = if role == Role::Full && small {
            self.clip_msg()
        } else {
            Some(self.latest_msg())
        };
        match m {
            Some(m) => self.send(peer, &m, out),
            None => false,
        }
    }

    fn send(&mut self, peer: DeviceId, m: &Message, out: &mut Vec<Event>) -> bool {
        let Some(p) = self.peers.get_mut(&peer) else {
            return false;
        };
        if p.link.send(msg::encode(m)).is_err() {
            self.peers.remove(&peer);
            out.push(Event::PeerLost { peer });
            self.link_lost(peer, out);
            return false;
        }
        true
    }

    fn age_ms(&self, stamp: &Stamp, now_ms: u64) -> u64 {
        self.hlc.now_wall(now_ms).saturating_sub(stamp.wall_ms)
    }

    /// May this clip be put on the clipboard without an explicit fetch?
    fn auto_apply_allowed(&self, stamp: &Stamp, now_ms: u64) -> bool {
        self.cfg.role == Role::PullOnly
            || self.age_ms(stamp, now_ms) < self.cfg.auto_apply_max_age_ms
    }

    /// Only the stamp decides. The same bytes copied again carry the same wire ID but a newer
    /// stamp, and must be applied. A clip already held (or older) never has a newer stamp.
    fn is_newer(&self, h: &ClipHeader) -> bool {
        self.is_newer_stamp(&h.stamp)
    }

    /// Is `stamp` newer than the current copy (clip or files)?
    fn is_newer_stamp(&self, stamp: &Stamp) -> bool {
        self.current.as_ref().is_none_or(|c| *stamp > c.stamp())
    }

    fn on_hello(&mut self, from: DeviceId, h: Hello, now_ms: u64, out: &mut Vec<Event>) {
        if h.device.id != from {
            out.push(Event::BadFrame {
                peer: from,
                error: CodecError::Malformed,
            });
            return;
        }
        if let Some(p) = self.peers.get_mut(&from) {
            p.role = h.role;
            p.ext = Some(h.ext);
        }
        out.push(Event::PeerHello {
            peer: from,
            role: h.role,
            protocol: h.protocol,
        });
        // File offers go out only now, when the peer's features are known.
        self.offer_files(from, out);
        // The peer refuses stamps more than the limit ahead of its clock.
        if now_ms > h.wall_ms.saturating_add(self.cfg.future_limit_ms) {
            out.push(Event::OwnClockAhead { peer: from });
        }
    }

    fn on_latest(
        &mut self,
        from: DeviceId,
        header: Option<ClipHeader>,
        now_ms: u64,
        out: &mut Vec<Event>,
    ) {
        let Some(h) = header else {
            if let Some(p) = self.peers.get_mut(&from) {
                p.latest = None;
            }
            return;
        };
        if !h.is_consistent() {
            out.push(Event::BadClip {
                peer: from,
                error: ClipError::Mismatch,
            });
            return;
        }
        if self.hlc.check(&h.stamp, now_ms).is_err() {
            out.push(Event::RefusedFutureStamp { peer: from });
            return;
        }
        if let Some(p) = self.peers.get_mut(&from) {
            p.latest = Some(h.clone());
        }
        if self.paused || !self.is_newer(&h) {
            return;
        }
        if self.explicit.contains(&h.id) || self.auto_apply_allowed(&h.stamp, now_ms) {
            self.send(from, &Message::Want { id: h.id }, out);
        }
    }

    fn on_clip(
        &mut self,
        from: DeviceId,
        header: ClipHeader,
        data: Vec<Vec<u8>>,
        cb: &mut dyn Clipboard,
        now_ms: u64,
        out: &mut Vec<Event>,
    ) {
        if !header.is_consistent() {
            out.push(Event::BadClip {
                peer: from,
                error: ClipError::Mismatch,
            });
            return;
        }
        if self.hlc.check(&header.stamp, now_ms).is_err() {
            out.push(Event::RefusedFutureStamp { peer: from });
            return;
        }
        // Layer 3: never apply a clip we already hold, or one older than it.
        if !self.is_newer(&header) {
            // Still move the clock: the stamp is valid.
            let _ = self.hlc.observe(&header.stamp, now_ms);
            return;
        }
        let clip = match Clip::from_wire(header.id, &header.parts, data) {
            Ok(c) => c,
            Err(error) => {
                out.push(Event::BadClip { peer: from, error });
                return;
            }
        };
        let _ = self.hlc.observe(&header.stamp, now_ms);
        if let Some(p) = self.peers.get_mut(&from) {
            p.latest = Some(header.clone());
        }
        let explicit = self.explicit.remove(&header.id);
        let from_pull_only = self
            .peers
            .get(&from)
            .is_some_and(|p| p.role == Role::PullOnly);
        let stamp = header.stamp;
        let content = clip.content_hash();
        self.set_current(
            Current::Clip(Held {
                clip,
                header,
                content,
            }),
            out,
        );

        if self.paused || !(explicit || self.auto_apply_allowed(&stamp, now_ms)) {
            out.push(Event::HeldOnly {
                id: self.held_id().unwrap_or_default(),
            });
        } else {
            self.apply_held(cb, out);
        }

        // A pull-only client talks only to the members it reached. A full member passes its
        // clip on to the other full members, so the whole group gets it.
        if from_pull_only && self.cfg.role == Role::Full {
            let others: Vec<DeviceId> = self
                .peers
                .iter()
                .filter(|(id, p)| **id != from && p.role == Role::Full)
                .map(|(id, _)| *id)
                .collect();
            for peer in others {
                self.offer_held(peer, out);
            }
        }
    }

    fn apply_held(&mut self, cb: &mut dyn Clipboard, out: &mut Vec<Event>) {
        let Some(held) = self.held_ref() else {
            return;
        };
        let ending = cb.line_ending();
        let native: Vec<RawPart> = held
            .clip
            .parts()
            .iter()
            .map(|(f, d)| match f {
                Format::TextPlain | Format::TextHtml => {
                    let s = String::from_utf8_lossy(d);
                    (*f, text::to_native(&s, ending).into_bytes())
                }
                Format::ImagePng => (*f, d.clone()),
            })
            .collect();
        let (id, kind, origin) = (held.header.id, held.clip.kind(), held.header.origin);
        match cb.write(&native) {
            Ok(seq) => {
                self.last_write_seq = Some(seq);
                self.applied = Some(id);
                // An applied remote copy ends a local walk.
                self.walk = None;
                out.push(Event::Applied { id, kind, origin });
            }
            Err(_) => out.push(Event::WriteFailed),
        }
    }
}
