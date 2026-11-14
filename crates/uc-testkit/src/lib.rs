//! In-process multi-agent test harness.
//!
//! [`FakeNet`] runs several [`uc_core::Engine`]s in one process. Each device has a
//! [`uc_clip::FakeClipboard`] and, if wanted, a fake clipboard manager that rewrites every
//! clipboard change (strips our marker, re-encodes the bytes). Frames travel through in-memory
//! links that keep order per link, like one QUIC stream. The test decides which link delivers
//! next, so a test can try every delivery order.
//!
//! Nothing here touches a real clipboard or a real network.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use uc_clip::{FakeClipboard, Format, RawPart};
use uc_core::sync::{Config, Engine, Event, Peer, PeerGone};
use uc_core::{DeviceId, DeviceKind, DeviceRecord, Role};

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
        self.devices.push(Device {
            engine: Engine::new(Config::new(rec, role)),
            clip,
            manager: None,
            events: Vec::new(),
            manager_seq: 0,
        });
        self.skew_ms.push(0);
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
    pub fn connect(&mut self, a: usize, b: usize) {
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
            self.devices[x].events.extend(ev);
        }
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

    /// Close the connection between `a` and `b`. Frames in flight are lost.
    pub fn disconnect(&mut self, a: usize, b: usize) {
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
        self.devices[a].engine.disconnect(ib);
        self.devices[b].engine.disconnect(ia);
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
        d.events.extend(ev);
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
        d.events.extend(ev);
    }

    /// Explicit fetch on device `i`.
    pub fn fetch(&mut self, i: usize) -> uc_core::FetchOutcome {
        let now = self.now(i);
        let d = &mut self.devices[i];
        let (outcome, ev) = d.engine.fetch(&mut d.clip, now);
        d.events.extend(ev);
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
        d.events.extend(ev);
        self.run_manager(f.to);
        true
    }

    /// Deliver everything, in link order, until the network is quiet.
    /// Panics if it does not go quiet within `limit` frames (a ping-pong loop).
    pub fn run_bounded(&mut self, limit: u64) {
        let start = self.delivered;
        while self.step(0) {
            assert!(
                self.delivered - start <= limit,
                "the network did not go quiet within {limit} frames"
            );
        }
    }

    /// Deliver everything until quiet (with a generous loop guard).
    pub fn run(&mut self) {
        self.run_bounded(100_000);
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
