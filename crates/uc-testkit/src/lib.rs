//! In-process multi-agent test harness.
//!
//! [`FakeNet`] runs several [`uc_core::Engine`]s in one process. Each device has a
//! [`uc_clip::FakeClipboard`] and, if wanted, a fake clipboard manager that rewrites every
//! clipboard change (strips our marker, re-encodes the bytes). Frames travel through in-memory
//! links that keep order per link, like one QUIC stream. The test decides which link delivers
//! next, so a test can try every delivery order.
//!
//! File copies (file sync spec 7): the harness also plays the agent's part. It keeps fake
//! file trees ([`FakeNet::add_file`], [`FakeNet::add_folder`]), runs walks, answers
//! `FetchFiles` with a free space check, and carries each `ServeFiles` as a fake data stream
//! that a test can hold back ([`FakeNet::hold_streams`]) and deliver one by one.
//!
//! Nothing here touches a real clipboard, a real file or a real network.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use uc_clip::{Clipboard, FakeClipboard, Format, RawPart};
use uc_core::files::{self, Entry, Manifest};
use uc_core::sync::{Config, Engine, Event, Peer, PeerGone, SkipReason, TransferFailure};
use uc_core::{DeviceId, DeviceKind, DeviceRecord, FilesId, Role};

pub mod images;

/// A fake clipboard manager. After every clipboard change it did not make itself, it rewrites
/// the clipboard the way real managers do: our marker is gone and the bytes change, but the
/// content stays the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Manager {
    /// Re-encodes text with CRLF and a trailing NUL, and adds an HTML copy.
    TextRewriter,
    /// Re-encodes every PNG with other compression settings.
    PngReencoder,
}

impl Manager {
    fn rewrite(self, parts: &[RawPart]) -> Vec<RawPart> {
        parts
            .iter()
            .map(|(f, d)| match (self, f) {
                (Manager::TextRewriter, Format::TextPlain) => {
                    let s = String::from_utf8_lossy(d).replace("\r\n", "\n");
                    (*f, format!("{}\0", s.replace('\n', "\r\n")).into_bytes())
                }
                (Manager::PngReencoder, Format::ImagePng) => (*f, images::reencode(d)),
                _ => (*f, d.clone()),
            })
            .chain(
                (self == Manager::TextRewriter
                    && parts.iter().any(|(f, _)| *f == Format::TextPlain)
                    && !parts.iter().any(|(f, _)| *f == Format::TextHtml))
                .then(|| (Format::TextHtml, b"<p>managed</p>".to_vec())),
            )
            .collect()
    }
}

/// One simulated device.
pub struct Device {
    /// Its sync engine.
    pub engine: Engine,
    /// Its clipboard.
    pub clip: FakeClipboard,
    /// Its clipboard manager, if any.
    pub manager: Option<Manager>,
    /// Every event the engine reported.
    pub events: Vec<Event>,
    manager_seq: u64,
}

impl Device {
    /// The device ID.
    pub fn id(&self) -> DeviceId {
        self.engine.id()
    }
}

struct InFlight {
    from: usize,
    to: usize,
    frame: Vec<u8>,
}

#[derive(Default)]
struct Wire {
    links: Vec<(usize, usize, VecDeque<Vec<u8>>, bool)>,
}

impl Wire {
    fn link(
        &mut self,
        from: usize,
        to: usize,
    ) -> Option<&mut (usize, usize, VecDeque<Vec<u8>>, bool)> {
        self.links.iter_mut().find(|l| l.0 == from && l.1 == to)
    }
}

/// The sending end of an in-memory link.
pub struct FakePeer {
    from: usize,
    to: usize,
    to_id: DeviceId,
    wire: Arc<Mutex<Wire>>,
}

impl Peer for FakePeer {
    fn id(&self) -> DeviceId {
        self.to_id
    }

    fn send(&mut self, frame: Vec<u8>) -> Result<(), PeerGone> {
        let mut w = self.wire.lock().map_err(|_| PeerGone)?;
        match w.link(self.from, self.to) {
            Some(l) if l.3 => {
                l.2.push_back(frame);
                Ok(())
            }
            _ => Err(PeerGone),
        }
    }
}

/// A fake data stream: one file send from `from` to `to`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FakeStream {
    /// The sender (the copy's origin).
    pub from: usize,
    /// The receiver.
    pub to: usize,
    /// The copy.
    pub id: FilesId,
}

/// A walk the engine asked for, not run yet.
struct PendingWalk {
    device: usize,
    generation: u64,
    paths: Vec<PathBuf>,
    picked: bool,
}

/// Several devices on an in-memory network.
pub struct FakeNet {
    /// The devices, by index.
    pub devices: Vec<Device>,
    /// The simulated wall clock in ms, the same on every device unless a test skews it.
    pub now_ms: u64,
    /// Per-device clock offsets in ms (to simulate a wrong clock).
    pub skew_ms: Vec<i64>,
    wire: Arc<Mutex<Wire>>,
    /// Frames delivered so far.
    pub delivered: u64,
    /// Fake file trees by absolute path. Each tree's entry paths start with the item's name.
    pub fs: BTreeMap<PathBuf, Vec<Entry>>,
    /// Walks of these paths fail with this reason (cloud placeholders and the like).
    pub fs_fail: BTreeMap<PathBuf, SkipReason>,
    /// Free disk space per device, in bytes.
    pub free_space: Vec<u64>,
    /// Hold walks back: `run` does not finish them (to copy something during a walk).
    pub hold_walks: bool,
    /// Hold data streams back: `run` does not deliver them.
    pub hold_streams: bool,
    /// Senders send a manifest that does not match their offer (a hostile or broken peer).
    pub tamper: bool,
    /// Data streams that are open, oldest first.
    pub streams: Vec<FakeStream>,
    walks: VecDeque<PendingWalk>,
    /// The manifest each origin serves, by (device, copy).
    manifests: BTreeMap<(usize, FilesId), Manifest>,
}

/// A made-up content hash for a fake file: the same name and size give the same hash.
pub fn fake_hash(name: &str, size: u64) -> [u8; 32] {
    let mut h = [0u8; 32];
    h[..8].copy_from_slice(&size.to_le_bytes());
    for (k, b) in name.bytes().enumerate() {
        h[8 + k % 24] ^= b.wrapping_add(k as u8);
    }
    h
}

/// A device ID for test device `i`.
pub fn device_id(i: usize) -> DeviceId {
    let mut b = [0u8; 32];
    b[0] = 0xd0;
    b[1..9].copy_from_slice(&(i as u64).to_le_bytes());
    DeviceId(b)
}

impl Default for FakeNet {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeNet {
    /// An empty network. The clock starts at a fixed date.
    pub fn new() -> Self {
        Self {
            devices: Vec::new(),
            now_ms: 1_790_000_000_000,
            skew_ms: Vec::new(),
            wire: Arc::default(),
            delivered: 0,
            fs: BTreeMap::new(),
            fs_fail: BTreeMap::new(),
            free_space: Vec::new(),
            hold_walks: false,
            hold_streams: false,
            tamper: false,
            streams: Vec::new(),
            walks: VecDeque::new(),
            manifests: BTreeMap::new(),
        }
    }

    /// A network of `n` full members with LF clipboards, all connected to each other, after
    /// the hello exchange.
    pub fn full_mesh(n: usize) -> Self {
        let mut net = Self::new();
        for _ in 0..n {
            net.add(Role::Full, FakeClipboard::new());
        }
        net.connect_all();
        net.run();
        net
    }

    /// Add a device. Returns its index.
    pub fn add(&mut self, role: Role, clip: FakeClipboard) -> usize {
        self.add_with(role, clip, |_| {})
    }

    /// Add a device whose engine settings `tune` changes first (for example `features = 0`
    /// to act as a 0.1.x peer). Returns its index.
    pub fn add_with(
        &mut self,
        role: Role,
        clip: FakeClipboard,
        tune: impl FnOnce(&mut Config),
    ) -> usize {
        let i = self.devices.len();
        let rec = DeviceRecord {
            id: device_id(i),
            name: format!("device {i}"),
            kind: if role == Role::Full {
                DeviceKind::Linux
            } else {
                DeviceKind::IPhone
            },
        };
        let mut cfg = Config::new(rec, role);
        tune(&mut cfg);
        self.devices.push(Device {
            engine: Engine::new(cfg),
            clip,
            manager: None,
            events: Vec::new(),
            manager_seq: 0,
        });
        self.skew_ms.push(0);
        self.free_space.push(u64::MAX);
        i
    }

    /// The clock of device `i`.
    pub fn now(&self, i: usize) -> u64 {
        self.now_ms.saturating_add_signed(self.skew_ms[i])
    }

    /// Move time forward.
    pub fn advance(&mut self, ms: u64) {
        self.now_ms += ms;
    }

    /// Open a connection between `a` and `b` (both directions). Hello and Latest are queued.
    /// An old connection between them is replaced, and its data streams die with it.
    pub fn connect(&mut self, a: usize, b: usize) {
        let broken = self.drop_streams(a, b);
        {
            let mut w = self.wire.lock().expect("wire");
            for (x, y) in [(a, b), (b, a)] {
                match w.link(x, y) {
                    Some(l) => {
                        l.2.clear();
                        l.3 = true;
                    }
                    None => w.links.push((x, y, VecDeque::new(), true)),
                }
            }
        }
        for (x, y) in [(a, b), (b, a)] {
            let peer = FakePeer {
                from: x,
                to: y,
                to_id: device_id(y),
                wire: self.wire.clone(),
            };
            let now = self.now(x);
            let ev = self.devices[x].engine.connect(Box::new(peer), now);
            self.absorb(x, ev);
        }
        self.report_broken(broken);
    }

    /// Connect every pair of devices.
    pub fn connect_all(&mut self) {
        let n = self.devices.len();
        for a in 0..n {
            for b in a + 1..n {
                self.connect(a, b);
            }
        }
    }

    /// The wire between `a` and `b` dies without the engines being told (they find out when
    /// they send). Frames in flight are lost.
    pub fn cut(&mut self, a: usize, b: usize) {
        let mut w = self.wire.lock().expect("wire");
        for (x, y) in [(a, b), (b, a)] {
            if let Some(l) = w.link(x, y) {
                l.2.clear();
                l.3 = false;
            }
        }
    }

    /// Close the connection between `a` and `b`. Frames in flight and data streams are lost.
    pub fn disconnect(&mut self, a: usize, b: usize) {
        let broken = self.drop_streams(a, b);
        {
            let mut w = self.wire.lock().expect("wire");
            for (x, y) in [(a, b), (b, a)] {
                if let Some(l) = w.link(x, y) {
                    l.2.clear();
                    l.3 = false;
                }
            }
        }
        let (ia, ib) = (device_id(a), device_id(b));
        let ev = self.devices[a].engine.disconnect(ib);
        self.absorb(a, ev);
        let ev = self.devices[b].engine.disconnect(ia);
        self.absorb(b, ev);
        self.report_broken(broken);
    }

    /// The user copies `parts` on device `i`. The device's manager (if any) reacts too.
    pub fn copy(&mut self, i: usize, parts: Vec<RawPart>) {
        self.devices[i].clip.copy(parts);
        self.local_change(i);
    }

    /// The user copies text on device `i`.
    pub fn copy_text(&mut self, i: usize, text: &str) {
        self.copy(i, vec![(Format::TextPlain, text.as_bytes().to_vec())]);
    }

    /// Tell device `i`'s engine that its clipboard changed, then let its manager react.
    pub fn local_change(&mut self, i: usize) {
        let now = self.now(i);
        let d = &mut self.devices[i];
        let ev = d.engine.on_local_change(&mut d.clip, now);
        self.absorb(i, ev);
        self.run_manager(i);
    }

    fn run_manager(&mut self, i: usize) {
        let d = &mut self.devices[i];
        let Some(m) = d.manager else { return };
        let seq = d.clip.peek().seq;
        if seq == d.manager_seq {
            return;
        }
        let parts = m.rewrite(&d.clip.peek().parts);
        d.manager_seq = d.clip.set(parts, false, false, false);
        let now = self.now(i);
        let d = &mut self.devices[i];
        let ev = d.engine.on_local_change(&mut d.clip, now);
        self.absorb(i, ev);
    }

    /// Explicit fetch on device `i`.
    pub fn fetch(&mut self, i: usize) -> uc_core::FetchOutcome {
        let now = self.now(i);
        let d = &mut self.devices[i];
        let (outcome, ev) = d.engine.fetch(&mut d.clip, now);
        self.absorb(i, ev);
        outcome
    }

    /// Links that have frames waiting, as (from, to).
    pub fn pending_links(&self) -> Vec<(usize, usize)> {
        let w = self.wire.lock().expect("wire");
        w.links
            .iter()
            .filter(|l| !l.2.is_empty())
            .map(|l| (l.0, l.1))
            .collect()
    }

    /// Number of frames in flight.
    pub fn in_flight(&self) -> usize {
        let w = self.wire.lock().expect("wire");
        w.links.iter().map(|l| l.2.len()).sum()
    }

    /// Deliver the next frame on the `choice`-th busy link (modulo their number).
    /// Returns false if nothing was in flight.
    pub fn step(&mut self, choice: usize) -> bool {
        let next = {
            let mut w = self.wire.lock().expect("wire");
            let busy: Vec<usize> = (0..w.links.len())
                .filter(|&k| !w.links[k].2.is_empty())
                .collect();
            if busy.is_empty() {
                None
            } else {
                let k = busy[choice % busy.len()];
                let l = &mut w.links[k];
                l.2.pop_front().map(|frame| InFlight {
                    from: l.0,
                    to: l.1,
                    frame,
                })
            }
        };
        let Some(f) = next else { return false };
        self.delivered += 1;
        let now = self.now(f.to);
        let from_id = device_id(f.from);
        let d = &mut self.devices[f.to];
        let ev = d.engine.on_frame(from_id, &f.frame, &mut d.clip, now);
        self.absorb(f.to, ev);
        self.run_manager(f.to);
        true
    }

    /// Hand `frame` to device `to` as if `from` had sent it (a hostile or odd peer).
    pub fn inject(&mut self, from: usize, to: usize, frame: &[u8]) {
        let now = self.now(to);
        let d = &mut self.devices[to];
        let ev = d.engine.on_frame(device_id(from), frame, &mut d.clip, now);
        self.absorb(to, ev);
    }

    /// The frames waiting on the link from `from` to `to`, oldest first.
    pub fn frames(&self, from: usize, to: usize) -> Vec<Vec<u8>> {
        let mut w = self.wire.lock().expect("wire");
        w.link(from, to)
            .map(|l| l.2.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Deliver everything, in link order, until the network is quiet. Walks and data streams
    /// run when no frame is left, unless held back.
    /// Panics if it does not go quiet within `limit` frames (a ping-pong loop).
    pub fn run_bounded(&mut self, limit: u64) {
        let start = self.delivered;
        let mut rounds = 0u64;
        loop {
            while self.step(0) {
                assert!(
                    self.delivered - start <= limit,
                    "the network did not go quiet within {limit} frames"
                );
            }
            rounds += 1;
            assert!(rounds <= limit, "walks and streams did not end");
            if !self.hold_walks && self.finish_walk() {
                continue;
            }
            if !self.hold_streams && !self.streams.is_empty() {
                self.deliver_stream(0);
                continue;
            }
            break;
        }
    }

    /// Deliver everything until quiet (with a generous loop guard).
    pub fn run(&mut self) {
        self.run_bounded(100_000);
    }

    // ----- files -----

    /// Where device `i`'s fake files live (never created on disk).
    pub fn root(i: usize) -> PathBuf {
        std::env::temp_dir()
            .join("uc-testkit-never-written")
            .join(format!("dev{i}"))
    }

    /// Add a fake file `name` of `size` bytes on device `i`. Returns its path.
    pub fn add_file(&mut self, i: usize, name: &str, size: u64) -> PathBuf {
        let path = Self::root(i).join(name);
        let entry = Entry::file(name, size, fake_hash(name, size), false);
        self.fs.insert(path.clone(), vec![entry]);
        path
    }

    /// Add a fake folder `name` on device `i` with `files` (relative path, size; parent
    /// folders are added) and empty folders `empty`. Returns its path.
    pub fn add_folder(
        &mut self,
        i: usize,
        name: &str,
        files: &[(&str, u64)],
        empty: &[&str],
    ) -> PathBuf {
        let mut folders = BTreeSet::from([name.to_owned()]);
        let mut entries = Vec::new();
        for (rel, size) in files {
            let path = format!("{name}/{rel}");
            let mut parent = path.as_str();
            while let Some((p, _)) = parent.rsplit_once('/') {
                folders.insert(p.to_owned());
                parent = p;
            }
            entries.push(Entry::file(&path, *size, fake_hash(&path, *size), false));
        }
        for e in empty {
            folders.insert(format!("{name}/{e}"));
        }
        entries.extend(folders.into_iter().map(Entry::folder));
        let path = Self::root(i).join(name);
        self.fs.insert(path.clone(), entries);
        path
    }

    /// The user copies `paths` in a file manager on device `i`.
    pub fn copy_paths(&mut self, i: usize, paths: &[PathBuf]) {
        self.devices[i].clip.copy_paths(paths);
        self.local_change(i);
    }

    /// "Copy files..." on device `i`.
    pub fn pick(&mut self, i: usize, paths: &[PathBuf]) {
        let ev = self.devices[i].engine.pick_files(paths.to_vec());
        self.absorb(i, ev);
    }

    /// Stop on device `i`.
    pub fn stop(&mut self, i: usize) {
        let ev = self.devices[i].engine.stop_transfer();
        self.absorb(i, ev);
    }

    /// Pause or resume device `i`.
    pub fn set_paused(&mut self, i: usize, paused: bool) {
        let ev = self.devices[i].engine.set_paused(paused);
        self.absorb(i, ev);
    }

    /// Turn "Copy smaller files automatically" on or off on device `i`.
    pub fn set_auto_file_copy(&mut self, i: usize, on: bool) {
        self.devices[i].engine.set_auto_file_copy(on);
    }

    /// The file paths on device `i`'s clipboard.
    pub fn clipboard_files(&self, i: usize) -> &[PathBuf] {
        self.devices[i].clip.files()
    }

    /// The ID of device `i`'s current file copy, if its current copy is files.
    pub fn current_files(&self, i: usize) -> Option<FilesId> {
        self.devices[i].engine.current_files().map(|o| o.id)
    }

    /// Finish the oldest held walk. Returns false if none is waiting.
    pub fn finish_walk(&mut self) -> bool {
        let Some(w) = self.walks.pop_front() else {
            return false;
        };
        let i = w.device;
        let reason = w
            .paths
            .iter()
            .find_map(|p| self.fs_fail.get(p).cloned())
            .or_else(|| {
                (!w.paths.iter().all(|p| self.fs.contains_key(p)))
                    .then_some(SkipReason::FilesVirtual)
            });
        let manifest = match reason {
            Some(r) => Err(r),
            None => self
                .walk_manifest(&w.paths)
                .map_err(|_| SkipReason::FilesTooMany),
        };
        let manifest = match manifest {
            Ok(m) => m,
            Err(r) => {
                let ev = self.devices[i].engine.walk_failed(w.generation, r);
                self.absorb(i, ev);
                return true;
            }
        };
        let ev = self.devices[i]
            .engine
            .on_walked(w.generation, manifest.total());
        let go = ev.contains(&Event::HashFiles {
            generation: w.generation,
        });
        self.absorb(i, ev);
        if !go {
            return true;
        }
        let now = self.now(i);
        let ev = self.devices[i]
            .engine
            .on_local_files(w.generation, manifest.summary(), now);
        let sent = ev.iter().find_map(|e| match e {
            Event::FilesSent { id, .. } => Some(*id),
            _ => None,
        });
        if let Some(id) = sent {
            self.manifests.insert((i, id), manifest);
        }
        self.absorb(i, ev);
        // A picked copy goes on the local clipboard too, with our marker, as our own write.
        if sent.is_some() && w.picked {
            let seq = self.devices[i]
                .clip
                .write_files(&w.paths, None)
                .expect("fake write");
            self.devices[i].engine.note_own_write(seq);
            self.local_change(i);
        }
        true
    }

    /// The manifest of a walk over `paths`: top-level names made unique, as the agent does.
    fn walk_manifest(&self, paths: &[PathBuf]) -> Result<Manifest, uc_core::ManifestError> {
        let mut taken = files::Names::new();
        let mut entries = Vec::new();
        for p in paths {
            let tree = &self.fs[p];
            let top = tree
                .iter()
                .map(|e| e.path.split('/').next().unwrap_or_default())
                .next()
                .unwrap_or_default()
                .to_owned();
            let is_file = tree.len() == 1 && tree[0].kind == uc_core::EntryKind::File;
            let name = files::unique_name(&top, is_file, true, &mut taken);
            for e in tree {
                let mut e = e.clone();
                e.path = format!("{name}{}", &e.path[top.len()..]);
                entries.push(e);
            }
        }
        Manifest::new(entries)
    }

    /// Deliver the `k`-th open data stream: the receiver checks it, "writes" the files, puts
    /// them on its clipboard and tells its engine; the sender's send ends.
    pub fn deliver_stream(&mut self, k: usize) {
        let s = self.streams.remove(k);
        let (from_id, to_id) = (device_id(s.from), device_id(s.to));
        let allowed = self.devices[s.to].engine.stream_allowed(from_id, s.id);
        if allowed {
            self.receive(s);
        }
        let ev = self.devices[s.from].engine.serve_done(to_id, s.id);
        self.absorb(s.from, ev);
    }

    /// The `k`-th open data stream breaks while the link is still up (for now): the receiver
    /// reports an interrupted fetch first, the sender ends its send. A real agent often sees
    /// this before the link's `Down`.
    pub fn break_stream(&mut self, k: usize) {
        let s = self.streams.remove(k);
        self.report_broken(vec![s]);
    }

    /// The `k`-th send fails before its stream opens (no stream credit, no data link): the
    /// sender's agent reports [`Engine::serve_failed`]; the receiver never sees a stream.
    pub fn fail_send(&mut self, k: usize) {
        let s = self.streams.remove(k);
        let ev = self.devices[s.from]
            .engine
            .serve_failed(device_id(s.to), s.id);
        self.absorb(s.from, ev);
    }

    fn receive(&mut self, s: FakeStream) {
        let to = s.to;
        let mut manifest = self.manifests[&(s.from, s.id)].clone();
        if self.tamper {
            let mut entries = manifest.entries().to_vec();
            entries.push(Entry::file("zz-extra", 1, [1; 32], false));
            manifest = Manifest::new(entries).expect("tampered manifest");
        }
        let expected = self.devices[to]
            .engine
            .pending_files()
            .map(|o| o.manifest_hash)
            .unwrap_or_default();
        let manifest = match Manifest::decode(manifest.bytes(), &expected) {
            Ok(m) => m,
            Err(_) => {
                let ev = self.devices[to]
                    .engine
                    .transfer_failed(s.id, TransferFailure::BadData);
                self.absorb(to, ev);
                return;
            }
        };
        let dir = Self::root(to).join("received").join(s.id.short());
        let local = manifest.local_paths(files::LocalRules {
            windows: false,
            fold_case: true,
        });
        let mut paths = Vec::new();
        for (e, l) in manifest.entries().iter().zip(&local) {
            if e.path.contains('/') {
                continue;
            }
            // The received tree, so that it can be copied again from here.
            let prefix = format!("{}/", e.path);
            let tree: Vec<Entry> = manifest
                .entries()
                .iter()
                .filter(|x| x.path == e.path || x.path.starts_with(&prefix))
                .cloned()
                .collect();
            let path = dir.join(l);
            self.fs.insert(path.clone(), tree);
            paths.push(path);
        }
        let seq = self.devices[to]
            .clip
            .write_files(&paths, None)
            .expect("fake write");
        let ev = self.devices[to].engine.files_applied(s.id, &manifest, seq);
        self.absorb(to, ev);
        // The OS reports our own write as a change.
        self.local_change(to);
    }

    /// Remove the streams between `a` and `b`; returns them.
    fn drop_streams(&mut self, a: usize, b: usize) -> Vec<FakeStream> {
        let (gone, kept) = self
            .streams
            .iter()
            .partition(|s| (s.from, s.to) == (a, b) || (s.from, s.to) == (b, a));
        self.streams = kept;
        gone
    }

    /// The agents see their broken streams: the receiver reports an interrupted fetch, the
    /// sender ends its send.
    fn report_broken(&mut self, broken: Vec<FakeStream>) {
        for s in broken {
            let ev = self.devices[s.to]
                .engine
                .transfer_failed(s.id, TransferFailure::Interrupted);
            self.absorb(s.to, ev);
            let ev = self.devices[s.from]
                .engine
                .serve_done(device_id(s.to), s.id);
            self.absorb(s.from, ev);
        }
    }

    fn index_of(&self, id: DeviceId) -> Option<usize> {
        (0..self.devices.len()).find(|&k| device_id(k) == id)
    }

    /// Keep device `i`'s events and do the agent's part for the file events.
    fn absorb(&mut self, i: usize, events: Vec<Event>) {
        self.devices[i].events.extend(events.iter().cloned());
        for e in events {
            match e {
                Event::WalkFiles {
                    generation,
                    paths,
                    picked,
                } => self.walks.push_back(PendingWalk {
                    device: i,
                    generation,
                    paths,
                    picked,
                }),
                Event::FetchFiles { id, total, .. } => {
                    let ev = if files::space_needed(total) > self.free_space[i] {
                        self.devices[i]
                            .engine
                            .transfer_failed(id, TransferFailure::NoSpace)
                    } else {
                        self.devices[i].engine.files_want(id)
                    };
                    self.absorb(i, ev);
                }
                Event::ServeFiles { peer, id } => {
                    if let Some(to) = self.index_of(peer) {
                        self.streams.push(FakeStream { from: i, to, id });
                    }
                }
                Event::CancelTransfer {
                    id, peer, sending, ..
                } => {
                    let Some(other) = self.index_of(peer) else {
                        continue;
                    };
                    let (from, to) = if sending { (i, other) } else { (other, i) };
                    let before = self.streams.len();
                    self.streams.retain(|s| *s != FakeStream { from, to, id });
                    // A receiver that stops a stream makes the sender's write fail.
                    if !sending && self.streams.len() < before {
                        let ev = self.devices[from].engine.serve_done(device_id(to), id);
                        self.absorb(from, ev);
                    }
                }
                _ => {}
            }
        }
    }

    /// Text on device `i`'s clipboard, with line endings folded to LF.
    pub fn text(&self, i: usize) -> Option<String> {
        self.devices[i]
            .clip
            .text()
            .map(|s| uc_core::text::normalize_for_hash(&s))
    }

    /// Clear the event logs.
    pub fn clear_events(&mut self) {
        for d in &mut self.devices {
            d.events.clear();
        }
    }

    /// Count `Sent` events on all devices.
    pub fn sends(&self) -> usize {
        self.devices
            .iter()
            .flat_map(|d| &d.events)
            .filter(|e| matches!(e, Event::Sent { .. }))
            .count()
    }

    /// The content hash of what is on device `i`'s clipboard (marker, HTML, line endings and
    /// PNG encoding do not matter). `None` if empty or not a valid clip.
    pub fn clipboard_content(&self, i: usize) -> Option<uc_core::ContentHash> {
        let parts: Vec<RawPart> = self.devices[i]
            .clip
            .peek()
            .parts
            .iter()
            .map(|(f, d)| match f {
                Format::TextPlain | Format::TextHtml => (
                    *f,
                    uc_core::text::to_wire(d).unwrap_or_default().into_bytes(),
                ),
                Format::ImagePng => (*f, d.clone()),
            })
            .collect();
        uc_core::Clip::from_parts(parts)
            .ok()
            .map(|c| c.content_hash())
    }

    /// True if every device holds the same clip and has its content on the clipboard.
    pub fn converged(&self) -> bool {
        let Some(first) = self.devices.first() else {
            return true;
        };
        let id = first.engine.held().map(|h| h.id);
        self.devices.iter().enumerate().all(|(i, d)| {
            d.engine.held().map(|h| h.id) == id
                && d.engine.held_clip().map(|c| c.content_hash()) == self.clipboard_content(i)
        })
    }

    /// A short report of each device's state, for assertion messages.
    pub fn describe(&self) -> String {
        self.devices
            .iter()
            .enumerate()
            .map(|(i, d)| {
                format!(
                    "device {i}: held {:?} stamp {:?} clipboard {:?}\n",
                    d.engine.held().map(|h| h.id),
                    d.engine.held().map(|h| (h.stamp.wall_ms, h.stamp.counter)),
                    self.text(i)
                )
            })
            .collect()
    }
}
