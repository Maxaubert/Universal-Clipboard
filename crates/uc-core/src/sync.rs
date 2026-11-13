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

use std::collections::{BTreeMap, BTreeSet};

use uc_clip::{Clipboard, Format, RawPart};

use crate::caps::{self, Dropped};
use crate::clip::{Clip, ClipError, ClipKind};
use crate::hlc::{FUTURE_LIMIT_MS, Hlc, Stamp};
use crate::ids::{ClipId, ContentHash, DeviceId};
use crate::msg::{self, ClipHeader, CodecError, DeviceRecord, Hello, Message, Role};
use crate::recent::{RECENT, Recent};
use crate::text;

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
    /// Echo layer 3: this wire ID was received or sent before.
    SeenId,
    /// Echo layer 4: this content was applied or sent in the last 64 clips.
    SeenContent,
    /// The source marked it as private (password manager).
    Private,
    /// It holds copied files.
    Files,
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
}

struct PeerState {
    link: Box<dyn Peer>,
    role: Role,
    latest: Option<ClipHeader>,
}

struct Held {
    clip: Clip,
    header: ClipHeader,
}

/// The sync state machine of one device.
pub struct Engine {
    cfg: Config,
    hlc: Hlc,
    held: Option<Held>,
    applied: Option<ClipId>,
    peers: BTreeMap<DeviceId, PeerState>,
    recent_ids: Recent<ClipId>,
    recent_content: Recent<ContentHash>,
    last_write_seq: Option<u64>,
    explicit: BTreeSet<ClipId>,
    paused: bool,
}

impl Engine {
    /// A new engine with nothing held.
    pub fn new(cfg: Config) -> Self {
        let hlc = Hlc::new(cfg.device.id).with_future_limit(cfg.future_limit_ms);
        Self {
            cfg,
            hlc,
            held: None,
            applied: None,
            peers: BTreeMap::new(),
            recent_ids: Recent::new(RECENT),
            recent_content: Recent::new(RECENT),
            last_write_seq: None,
            explicit: BTreeSet::new(),
            paused: false,
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

    /// The header of the newest clip held, if any.
    pub fn held(&self) -> Option<&ClipHeader> {
        self.held.as_ref().map(|h| &h.header)
    }

    /// The newest clip held, if any.
    pub fn held_clip(&self) -> Option<&Clip> {
        self.held.as_ref().map(|h| &h.clip)
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
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    /// Is sync paused?
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Change the "Sync images" setting.
    pub fn set_sync_images(&mut self, on: bool) {
        self.cfg.sync_images = on;
    }

    /// A connection to a peer is up. Sends `Hello` and `Latest`. A second connection with the
    /// same peer replaces the first.
    pub fn connect(&mut self, link: Box<dyn Peer>, now_ms: u64) -> Vec<Event> {
        let mut out = Vec::new();
        let id = link.id();
        self.peers.insert(
            id,
            PeerState {
                link,
                role: Role::Full,
                latest: None,
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
        });
        self.send(id, &hello, &mut out);
        let latest = self.latest_msg();
        self.send(id, &latest, &mut out);
        out
    }

    /// The connection to a peer ended.
    pub fn disconnect(&mut self, peer: DeviceId) {
        self.peers.remove(&peer);
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
        if snap.private {
            out.push(Event::Skipped(SkipReason::Private));
            return out;
        }
        if snap.has_files {
            out.push(Event::Skipped(SkipReason::Files));
            return out;
        }
        let mut parts: Vec<RawPart> = Vec::with_capacity(snap.parts.len());
        for (format, data) in snap.parts {
            if parts.iter().any(|(f, _)| *f == format) {
                continue;
            }
            match format {
                Format::TextPlain | Format::TextHtml => match text::to_wire(&data) {
                    Some(s) => parts.push((format, s.into_bytes())),
                    None => {
                        out.push(Event::Skipped(SkipReason::Invalid(ClipError::NotUtf8)));
                        return out;
                    }
                },
                Format::ImagePng if !self.cfg.sync_images => {}
                Format::ImagePng => parts.push((format, data)),
            }
        }
        if parts.is_empty() {
            out.push(Event::Skipped(SkipReason::Empty));
            return out;
        }
        let (parts, dropped) = caps::fit(parts);
        out.extend(dropped.into_iter().map(Event::PartDropped));
        if parts.is_empty() {
            out.push(Event::Skipped(SkipReason::TooLarge));
            return out;
        }
        let clip = match Clip::from_parts(parts) {
            Ok(c) => c,
            Err(e) => {
                out.push(Event::Skipped(SkipReason::Invalid(e)));
                return out;
            }
        };
        // Layer 3: never send a wire ID that was received (or already sent).
        if self.recent_ids.contains(&clip.id()) || self.held_id() == Some(clip.id()) {
            out.push(Event::Skipped(SkipReason::SeenId));
            return out;
        }
        // Layer 4: never send content that was applied or sent recently.
        if self.recent_content.contains(&clip.content_hash()) {
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
        self.recent_ids.push(clip.id());
        self.recent_content.push(clip.content_hash());
        self.applied = Some(clip.id());
        let (id, kind, size) = (clip.id(), clip.kind(), clip.size());
        self.held = Some(Held { clip, header });

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
            Message::Membership { changes } => out.push(Event::Membership {
                peer: from,
                changes,
            }),
            Message::AddrUpdate { signed } => out.push(Event::AddrUpdate { peer: from, signed }),
        }
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
        let held_stamp = self.held.as_ref().map(|h| h.header.stamp);
        match (best_peer, held_stamp) {
            (Some((from, id, stamp)), held) if held.is_none_or(|s| stamp > s) => {
                self.explicit.insert(id);
                self.send(from, &Message::Want { id }, &mut out);
                (FetchOutcome::Requested { from, id }, out)
            }
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

    fn held_id(&self) -> Option<ClipId> {
        self.held.as_ref().map(|h| h.header.id)
    }

    fn has_full_peer(&self) -> bool {
        self.peers.values().any(|p| p.role == Role::Full)
    }

    fn latest_msg(&self) -> Message {
        Message::Latest {
            header: self.held.as_ref().map(|h| h.header.clone()),
        }
    }

    fn clip_msg(&self) -> Option<Message> {
        self.held.as_ref().map(|h| Message::Clip {
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
            .held
            .as_ref()
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

    fn is_newer(&self, h: &ClipHeader) -> bool {
        !self.recent_ids.contains(&h.id)
            && self
                .held
                .as_ref()
                .is_none_or(|held| h.stamp > held.header.stamp)
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
        }
        out.push(Event::PeerHello {
            peer: from,
            role: h.role,
            protocol: h.protocol,
        });
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
        // Layer 3: never apply a wire ID we already hold or had.
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
        self.recent_ids.push(header.id);
        self.held = Some(Held { clip, header });

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
        let Some(held) = self.held.as_ref() else {
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
        let (id, kind, origin, content) = (
            held.header.id,
            held.clip.kind(),
            held.header.origin,
            held.clip.content_hash(),
        );
        match cb.write(&native) {
            Ok(seq) => {
                self.last_write_seq = Some(seq);
                self.applied = Some(id);
                self.recent_content.push(content);
                out.push(Event::Applied { id, kind, origin });
            }
            Err(_) => out.push(Event::WriteFailed),
        }
    }
}
