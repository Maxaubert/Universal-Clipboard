//! [`Net`]: the networking side of one device. It owns the endpoint, the address book, mDNS,
//! the dialer and the path states, and runs as one tokio task.
//!
//! The agent (M7) uses it like this:
//!
//! 1. [`Net::start`], then [`Net::set_members`] and [`Net::set_secrets`] from the membership
//!    chain (M3).
//! 2. On [`NetEvent::Up`]: `engine.connect(Box::new(link.peer_handle()), now)`.
//!    On [`NetEvent::Frame`]: `engine.on_frame(peer, &frame, ...)`.
//!    On [`NetEvent::Down`]: `engine.disconnect(peer)`.
//! 3. After membership sync passed for a peer (protocol 5.9 step 5): [`Net::ready`]. Only then
//!    does this device send addresses to it.
//! 4. On the engine's `Event::AddrUpdate { peer, signed }`: [`Net::addr_update`].
//! 5. On [`NetEvent::Path`]: update the device row (and "Needs attention" for
//!    [`crate::path::NoPath::VpnBlocksLan`]).
//! 6. On OS wake or network-change notifications: [`Net::wake`] (the task also notices both
//!    by itself, by polling interfaces and watching for time jumps).
//!
//! Connections from keys that are not members are closed right after the handshake, before
//! any frame, with one exception: the **admission hook** of protocol 5.9. An unknown key
//! that is not removed ([`Net::set_removed`]) may get a probation link, one at a time, for at
//! most [`PROBATION`]. The agent runs membership sync on it (steps 1 to 4 only). If the key is
//! a member afterwards, [`Net::set_members`] makes it a normal link; if not, the agent calls
//! [`Net::end_probation`] (or the time runs out) and the link closes. Pairing connections
//! (`uclip-pair/1`) are handed out as [`NetEvent::Pairing`] with the LAN rule already applied.

use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinSet;
use uc_core::{DeviceId, Message, msg};

use crate::addr::{self, Policy};
use crate::binder::{SocketBinder, Tie};
use crate::book::{Applied, LanBook, Source};
use crate::discovery::Discovery;
use crate::endpoint::{
    ALPN_PAIR, ALPN_SYNC, BindError, CLOSE_DUPLICATE, CLOSE_NOT_ADMITTED, Conn, DialError,
    LanEndpoint, Timing,
};
use crate::gossip::{self, Announcement};
use crate::ifaces::{self, ClassifyOptions, IfAddr, Interface, Kind};
use crate::link::{self, Link, LinkEvent};
use crate::path::{self, PathState};
use crate::tags::{self, Secrets};

/// Interface source: the real OS, or a test's list.
pub type IfaceSource = Arc<dyn Fn() -> Vec<Interface> + Send + Sync>;

/// How long a probation link for an unknown key may stay open (protocol 5.9: 10 s).
pub const PROBATION: Duration = Duration::from_secs(10);

/// Settings.
#[derive(Clone)]
pub struct NetConfig {
    /// The device key.
    pub key: SigningKey,
    /// The fixed UDP port (0: pick one).
    pub port: u16,
    /// LAN rule settings.
    pub policy: Policy,
    /// QUIC timers.
    pub timing: Timing,
    /// Classifier options.
    pub classify: ClassifyOptions,
    /// Main loop tick.
    pub tick: Duration,
    /// Interface scan interval.
    pub iface_poll: Duration,
    /// The VPN-blocks-LAN delay (plan: 20 s).
    pub path_delay: Duration,
    /// Longest wait between dial attempts.
    pub backoff_max: Duration,
    /// Where interfaces come from (default: [`ifaces::detect`]).
    pub ifaces: Option<IfaceSource>,
    /// Admission hook (protocol 5.9): give unknown keys a probation link. On by default.
    pub probation: bool,
    /// Accept incoming connections at all. Off is the "outbound-only" mode of plan 2.2 (the
    /// user refused the Windows firewall row): this device only dials.
    pub accept_incoming: bool,
}

impl NetConfig {
    /// Defaults for the agent.
    pub fn new(key: SigningKey, port: u16) -> Self {
        Self {
            key,
            port,
            policy: Policy::default(),
            timing: Timing::default(),
            classify: ClassifyOptions::default(),
            tick: Duration::from_millis(250),
            iface_poll: Duration::from_secs(2),
            path_delay: path::BLOCK_DELAY,
            backoff_max: Duration::from_secs(30),
            ifaces: None,
            probation: true,
            accept_incoming: true,
        }
    }
}

/// Something for the agent.
#[derive(Debug)]
pub enum NetEvent {
    /// A sync link to a member is up. Connect it to the engine.
    Up(Link),
    /// A frame from a peer.
    Frame {
        /// From.
        peer: DeviceId,
        /// The frame.
        frame: Vec<u8>,
    },
    /// A link is down. Disconnect it from the engine.
    Down {
        /// Peer.
        peer: DeviceId,
    },
    /// The path state of a member changed.
    Path {
        /// The member.
        peer: DeviceId,
        /// Its new state.
        state: PathState,
    },
    /// An incoming pairing connection (LAN rule already passed). M3 takes it from here.
    Pairing(Conn),
    /// The fixed port was taken; this is the new one. Store it; peers learn it by gossip.
    PortChanged(u16),
    /// The physical interfaces or the VPN state changed.
    Interfaces {
        /// Physical interfaces in use.
        physical: Vec<Interface>,
        /// A VPN interface is up.
        vpn_up: bool,
    },
}

/// A snapshot for diagnostics and the tests.
#[derive(Clone, Debug)]
pub struct Status {
    /// Our device ID.
    pub id: DeviceId,
    /// The UDP port.
    pub port: u16,
    /// Bound sockets: interface, address, tie.
    pub sockets: Vec<(String, SocketAddr, Tie)>,
    /// Path state per member.
    pub paths: BTreeMap<DeviceId, PathState>,
    /// Open links: peer, remote address, our interface.
    pub links: Vec<(DeviceId, SocketAddr, String)>,
    /// A VPN interface is up here.
    pub vpn_up: bool,
    /// Incoming connections dropped by the LAN rule.
    pub refused: u64,
    /// The address book.
    pub book: LanBook,
}

enum Cmd {
    Members(Vec<DeviceId>),
    Removed(Vec<DeviceId>),
    EndProbation(DeviceId),
    Secrets(Secrets),
    AddAddr(DeviceId, SocketAddr),
    AddrUpdate(DeviceId, Vec<u8>),
    Ready(DeviceId),
    Wake,
    Status(oneshot::Sender<Status>),
    DialPair(
        Option<DeviceId>,
        SocketAddr,
        oneshot::Sender<Result<Conn, DialError>>,
    ),
    Shutdown,
}

enum Internal {
    Opened(Link, Instant),
    DialFailed(DeviceId),
}

/// A cheap, cloneable handle for other tasks: pairing dials and status. It does not keep
/// the task alive.
#[derive(Clone)]
pub struct NetHandle {
    cmd: mpsc::UnboundedSender<Cmd>,
}

impl NetHandle {
    /// See [`Net::dial_pair_any`].
    pub async fn dial_pair_any(&self, addr: SocketAddr) -> Result<Conn, DialError> {
        let (tx, rx) = oneshot::channel();
        self.cmd
            .send(Cmd::DialPair(None, addr, tx))
            .map_err(|_| DialError::Quic("stopped".into()))?;
        rx.await.map_err(|_| DialError::Quic("stopped".into()))?
    }

    /// See [`Net::status`].
    pub async fn status(&self) -> Option<Status> {
        let (tx, rx) = oneshot::channel();
        self.cmd.send(Cmd::Status(tx)).ok()?;
        rx.await.ok()
    }
}

/// The handle. Dropping it stops the task.
pub struct Net {
    id: DeviceId,
    cmd: mpsc::UnboundedSender<Cmd>,
    events: mpsc::UnboundedReceiver<NetEvent>,
}

impl Net {
    /// Start. Must be called inside a tokio runtime.
    pub fn start(
        cfg: NetConfig,
        binder: Arc<dyn SocketBinder>,
        discovery: Arc<dyn Discovery>,
        book: LanBook,
    ) -> Result<Net, BindError> {
        let id = DeviceId(cfg.key.verifying_key().to_bytes());
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (ev_tx, ev_rx) = mpsc::unbounded_channel();
        let actor = Actor::new(cfg, binder, discovery, book, ev_tx)?;
        tokio::spawn(actor.run(cmd_rx));
        Ok(Net {
            id,
            cmd: cmd_tx,
            events: ev_rx,
        })
    }

    /// Our device ID.
    pub fn id(&self) -> DeviceId {
        self.id
    }

    /// A handle for other tasks.
    pub fn handle(&self) -> NetHandle {
        NetHandle {
            cmd: self.cmd.clone(),
        }
    }

    /// The next event. `None` after shutdown.
    pub async fn next_event(&mut self) -> Option<NetEvent> {
        self.events.recv().await
    }

    /// Non-blocking variant of [`Self::next_event`].
    pub fn try_event(&mut self) -> Option<NetEvent> {
        self.events.try_recv().ok()
    }

    /// The current members (other than this device). Links to keys that are not in the list
    /// are closed.
    pub fn set_members(&self, members: Vec<DeviceId>) {
        let _ = self.cmd.send(Cmd::Members(members));
    }

    /// Keys that an effective remove names (protocol 5.7). Their connections are refused
    /// before any data, and they never get a probation link.
    pub fn set_removed(&self, removed: Vec<DeviceId>) {
        let _ = self.cmd.send(Cmd::Removed(removed));
    }

    /// Membership sync on a probation link did not make the peer a member: close it.
    pub fn end_probation(&self, peer: DeviceId) {
        let _ = self.cmd.send(Cmd::EndProbation(peer));
    }

    /// The group secrets for discovery tags.
    pub fn set_secrets(&self, s: Secrets) {
        let _ = self.cmd.send(Cmd::Secrets(s));
    }

    /// Remember an address of a member (from the QR code or pairing).
    pub fn add_addr(&self, d: DeviceId, a: SocketAddr) {
        let _ = self.cmd.send(Cmd::AddAddr(d, a));
    }

    /// The engine reported `Event::AddrUpdate` from `from`.
    pub fn addr_update(&self, from: DeviceId, signed: Vec<u8>) {
        let _ = self.cmd.send(Cmd::AddrUpdate(from, signed));
    }

    /// Membership sync with `peer` passed: addresses may flow now.
    pub fn ready(&self, peer: DeviceId) {
        let _ = self.cmd.send(Cmd::Ready(peer));
    }

    /// The OS said: woke from sleep, or the network changed. Rescan and redial at once.
    pub fn wake(&self) {
        let _ = self.cmd.send(Cmd::Wake);
    }

    /// A snapshot.
    pub async fn status(&self) -> Option<Status> {
        let (tx, rx) = oneshot::channel();
        self.cmd.send(Cmd::Status(tx)).ok()?;
        rx.await.ok()
    }

    /// Dial a pairing connection (`uclip-pair/1`) to `peer` at `addr` (from the QR code or
    /// mDNS). The LAN rule applies.
    pub async fn dial_pair(&self, peer: DeviceId, addr: SocketAddr) -> Result<Conn, DialError> {
        let (tx, rx) = oneshot::channel();
        self.cmd
            .send(Cmd::DialPair(Some(peer), addr, tx))
            .map_err(|_| DialError::Quic("stopped".into()))?;
        rx.await.map_err(|_| DialError::Quic("stopped".into()))?
    }

    /// Dial a pairing connection to `addr` without knowing the key there (code mode: the
    /// joiner found the inviter through the `_uclip-pair._udp` mDNS service). The LAN rule
    /// applies. The PAKE proves who answered.
    pub async fn dial_pair_any(&self, addr: SocketAddr) -> Result<Conn, DialError> {
        let (tx, rx) = oneshot::channel();
        self.cmd
            .send(Cmd::DialPair(None, addr, tx))
            .map_err(|_| DialError::Quic("stopped".into()))?;
        rx.await.map_err(|_| DialError::Quic("stopped".into()))?
    }

    /// Stop.
    pub fn shutdown(&self) {
        let _ = self.cmd.send(Cmd::Shutdown);
    }
}

impl Drop for Net {
    fn drop(&mut self) {
        let _ = self.cmd.send(Cmd::Shutdown);
    }
}

/// The device with the larger key waits a moment before it dials, so two devices that find
/// each other at the same time usually end with one connection, not two.
fn dial_delay(me: DeviceId, peer: DeviceId) -> Duration {
    if me > peer {
        Duration::from_millis(300)
    } else {
        Duration::ZERO
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

struct PeerSt {
    link: Option<Link>,
    link_since: Option<Instant>,
    ready: bool,
    dialing: bool,
    next_dial: Instant,
    backoff: Duration,
    down_since: Instant,
    path: Option<PathState>,
    /// Set while this is a probation link for a key that is not a member yet (5.9).
    probation: Option<Instant>,
}

impl PeerSt {
    fn new(now: Instant) -> Self {
        Self {
            link: None,
            link_since: None,
            ready: false,
            dialing: false,
            next_dial: now,
            backoff: Duration::ZERO,
            down_since: now,
            path: None,
            probation: None,
        }
    }

    fn is_member(&self) -> bool {
        self.probation.is_none()
    }
}

struct Actor {
    cfg: NetConfig,
    id: DeviceId,
    binder: Arc<dyn SocketBinder>,
    disc: Arc<dyn Discovery>,
    book: LanBook,
    ev: mpsc::UnboundedSender<NetEvent>,
    ep: Option<Arc<LanEndpoint>>,
    incoming: Option<mpsc::Receiver<Conn>>,
    ifaces: Vec<Interface>,
    vpn_up: bool,
    members: BTreeMap<DeviceId, PeerSt>,
    removed: BTreeSet<DeviceId>,
    secrets: Secrets,
    advertised: Option<String>,
    own_seq: u64,
    own_ann: Option<(Vec<IfAddr>, u16, bool, Vec<u8>)>,
    link_tx: mpsc::UnboundedSender<LinkEvent>,
    link_rx: mpsc::UnboundedReceiver<LinkEvent>,
    int_tx: mpsc::UnboundedSender<Internal>,
    int_rx: mpsc::UnboundedReceiver<Internal>,
    last_tick: (Instant, u64),
    last_scan: Instant,
    /// Frames (and a close) of links the actor has not registered yet.
    pending: BTreeMap<u64, (Vec<Vec<u8>>, bool)>,
    /// Links that were refused or replaced: their late events are dropped.
    retired: BTreeSet<u64>,
}

impl Actor {
    fn new(
        cfg: NetConfig,
        binder: Arc<dyn SocketBinder>,
        disc: Arc<dyn Discovery>,
        book: LanBook,
        ev: mpsc::UnboundedSender<NetEvent>,
    ) -> Result<Self, BindError> {
        let (link_tx, link_rx) = mpsc::unbounded_channel();
        let (int_tx, int_rx) = mpsc::unbounded_channel();
        let now = Instant::now();
        let mut a = Self {
            id: DeviceId(cfg.key.verifying_key().to_bytes()),
            cfg,
            binder,
            disc,
            book,
            ev,
            ep: None,
            incoming: None,
            ifaces: Vec::new(),
            vpn_up: false,
            members: BTreeMap::new(),
            removed: BTreeSet::new(),
            secrets: Secrets::default(),
            advertised: None,
            own_seq: 0,
            own_ann: None,
            link_tx,
            link_rx,
            int_tx,
            int_rx,
            last_tick: (now, now_ms()),
            last_scan: now,
            pending: BTreeMap::new(),
            retired: BTreeSet::new(),
        };
        let list = a.scan();
        a.apply_ifaces(list, true)?;
        Ok(a)
    }

    fn scan(&self) -> Vec<Interface> {
        match &self.cfg.ifaces {
            Some(f) => f(),
            None => ifaces::detect(&self.cfg.classify),
        }
    }

    fn in_use(&self, list: &[Interface]) -> Vec<Interface> {
        list.iter()
            .filter(|i| {
                i.usable()
                    || (self.cfg.policy.allow_loopback
                        && i.kind == Kind::Loopback
                        && i.up
                        && !i.addrs.is_empty())
            })
            .cloned()
            .collect()
    }

    /// Apply a new interface list. Sockets of addresses that stay keep their links; new
    /// addresses get sockets on the same port; gone addresses lose theirs.
    fn apply_ifaces(&mut self, list: Vec<Interface>, first: bool) -> Result<(), BindError> {
        let old_use = self.in_use(&self.ifaces);
        let new_use = self.in_use(&list);
        let vpn = ifaces::vpn_up(&list);
        let vpn_changed = vpn != self.vpn_up;
        self.ifaces = list;
        self.vpn_up = vpn;
        let mut changed = first;
        if !first && old_use != new_use {
            if let Some(ep) = self.ep.clone() {
                match ep.update(&new_use, &*self.binder) {
                    Ok(ch) => changed = !ch.is_empty(),
                    Err(BindError::PortInUse(_)) => {
                        // A new address cannot take our port: move everything to a new one.
                        self.rebind(&new_use, 0)?;
                        changed = true;
                    }
                    Err(e) => return Err(e),
                }
            } else {
                changed = true;
            }
            let _ = self.disc.set_interfaces(&new_use);
        }
        if first {
            self.rebind(&new_use, self.cfg.port)?;
            let _ = self.disc.set_interfaces(&new_use);
        }
        if changed {
            self.advertised = None;
            let now = Instant::now();
            let me = self.id;
            for (d, p) in self.members.iter_mut().filter(|(_, p)| p.is_member()) {
                if p.link.is_none() {
                    p.next_dial = now + dial_delay(me, *d);
                    p.backoff = Duration::ZERO;
                    p.down_since = now;
                }
            }
        }
        if changed || vpn_changed || old_use != new_use {
            let _ = self.ev.send(NetEvent::Interfaces {
                physical: new_use,
                vpn_up: vpn,
            });
            self.announce_all();
        }
        Ok(())
    }

    /// Bind a fresh endpoint. `port` 0 or a taken port picks a new port and reports it.
    fn rebind(&mut self, use_: &[Interface], port: u16) -> Result<(), BindError> {
        let wanted = if port == 0 { self.cfg.port } else { port };
        if let Some(old) = self.ep.take() {
            old.close();
        }
        self.incoming = None;
        let alpns = vec![ALPN_SYNC.to_vec(), ALPN_PAIR.to_vec()];
        let bind = |p| {
            LanEndpoint::bind(
                self.cfg.key.clone(),
                use_,
                p,
                self.cfg.policy,
                self.cfg.timing,
                &*self.binder,
                &alpns,
            )
        };
        let (ep, rx) = match bind(port) {
            Err(BindError::PortInUse(_)) if port != 0 => bind(0)?,
            r => r?,
        };
        if ep.port() != wanted && wanted != 0 {
            let _ = self.ev.send(NetEvent::PortChanged(ep.port()));
        }
        tracing::info!(
            port = ep.port(),
            sockets = ep.sockets().len(),
            "endpoint bound"
        );
        self.cfg.port = ep.port();
        self.ep = Some(Arc::new(ep));
        self.incoming = Some(rx);
        Ok(())
    }

    async fn run(mut self, mut cmd: mpsc::UnboundedReceiver<Cmd>) {
        let mut tick = tokio::time::interval(self.cfg.tick);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                c = cmd.recv() => match c {
                    None | Some(Cmd::Shutdown) => break,
                    Some(c) => self.on_cmd(c),
                },
                Some(c) = recv_opt(&mut self.incoming) => self.on_incoming(c),
                Some(e) = self.link_rx.recv() => self.on_link_event(e),
                Some(i) = self.int_rx.recv() => self.on_internal(i),
                _ = tick.tick() => self.on_tick(),
            }
        }
        self.disc.withdraw();
        for p in self.members.values() {
            if let Some(l) = &p.link {
                l.close(0, b"bye");
            }
        }
        if let Some(ep) = self.ep.take() {
            ep.close();
        }
    }

    fn on_cmd(&mut self, c: Cmd) {
        match c {
            Cmd::Members(list) => {
                let now = Instant::now();
                let keep: BTreeSet<DeviceId> = list.into_iter().filter(|d| *d != self.id).collect();
                // A probation link stays until its own end (agent or timeout).
                let gone: Vec<DeviceId> = self
                    .members
                    .iter()
                    .filter(|(d, p)| !keep.contains(d) && p.is_member())
                    .map(|(d, _)| *d)
                    .collect();
                for d in gone {
                    if let Some(p) = self.members.remove(&d)
                        && let Some(l) = p.link
                    {
                        l.close(CLOSE_NOT_ADMITTED, b"not a member");
                        let _ = self.ev.send(NetEvent::Down { peer: d });
                    }
                    self.book.forget(&d);
                }
                let me = self.id;
                for d in keep {
                    let p = self.members.entry(d).or_insert_with(|| {
                        let mut p = PeerSt::new(now);
                        p.next_dial = now + dial_delay(me, d);
                        p
                    });
                    // A probation peer that membership sync made a member: a normal link now.
                    p.probation = None;
                }
                self.update_paths();
            }
            Cmd::Removed(list) => {
                self.removed = list.into_iter().collect();
                let gone: Vec<DeviceId> = self
                    .members
                    .keys()
                    .filter(|d| self.removed.contains(d))
                    .copied()
                    .collect();
                for d in gone {
                    self.drop_peer(d);
                }
            }
            Cmd::EndProbation(d) => {
                if self.members.get(&d).is_some_and(|p| !p.is_member()) {
                    self.drop_peer(d);
                }
            }
            Cmd::Secrets(s) => {
                self.secrets = s;
                self.advertised = None;
            }
            Cmd::AddAddr(d, a) => {
                if self
                    .book
                    .add(d, a, Source::Pairing, now_ms(), &self.cfg.policy)
                {
                    self.dial_soon(d);
                }
            }
            Cmd::AddrUpdate(from, signed) => self.on_addr_update(from, signed),
            Cmd::Ready(d) => {
                // A probation peer never gets addresses (5.9).
                let Some(p) = self.members.get_mut(&d).filter(|p| p.is_member()) else {
                    return;
                };
                p.ready = true;
                self.send_announcements_to(d);
            }
            Cmd::Wake => self.on_wake(),
            Cmd::Status(tx) => {
                let _ = tx.send(self.status());
            }
            Cmd::DialPair(peer, addr, tx) => {
                let Some(ep) = self.ep.clone() else {
                    let _ = tx.send(Err(DialError::NotLan(addr)));
                    return;
                };
                tokio::spawn(async move {
                    let r = match peer {
                        Some(p) => ep.dial(p, addr, ALPN_PAIR).await,
                        None => ep.dial_pair_any(addr).await,
                    };
                    let _ = tx.send(r);
                });
            }
            Cmd::Shutdown => {}
        }
    }

    fn status(&self) -> Status {
        Status {
            id: self.id,
            port: self.cfg.port,
            sockets: self
                .ep
                .iter()
                .flat_map(|e| e.sockets().into_iter().map(|s| (s.iface, s.local, s.tie)))
                .collect(),
            paths: self
                .members
                .iter()
                .filter(|(_, p)| p.is_member())
                .filter_map(|(d, p)| p.path.map(|s| (*d, s)))
                .collect(),
            links: self
                .members
                .iter()
                .filter_map(|(d, p)| p.link.as_ref().map(|l| (*d, l.remote, l.iface.clone())))
                .collect(),
            vpn_up: self.vpn_up,
            refused: self.ep.as_ref().map_or(0, |e| e.refused()),
            book: self.book.clone(),
        }
    }

    fn on_incoming(&mut self, c: Conn) {
        if !self.cfg.accept_incoming {
            // Outbound-only mode: this device only dials.
            c.conn.close(CLOSE_NOT_ADMITTED.into(), b"outbound only");
            return;
        }
        if c.alpn == ALPN_PAIR {
            let _ = self.ev.send(NetEvent::Pairing(c));
            return;
        }
        if c.alpn != ALPN_SYNC || self.removed.contains(&c.peer) {
            c.conn.close(CLOSE_NOT_ADMITTED.into(), b"not a member");
            return;
        }
        if !self.members.contains_key(&c.peer) {
            // Admission hook (protocol 5.9): an unknown key may be a device that was added
            // while we were away. One probation link at a time, for at most 10 s; the agent
            // runs membership sync on it and reads nothing else. Plan 4.1 still holds: unknown
            // keys get no clip, no secret and no address.
            let busy = self.members.values().any(|p| !p.is_member());
            if !self.cfg.probation || busy {
                c.conn.close(CLOSE_NOT_ADMITTED.into(), b"not a member");
                return;
            }
            let now = Instant::now();
            let mut p = PeerSt::new(now);
            p.probation = Some(now);
            p.next_dial = now + Duration::from_secs(3600);
            self.members.insert(c.peer, p);
        }
        self.spawn_open(c);
    }

    /// Forget a peer and close its link.
    fn drop_peer(&mut self, d: DeviceId) {
        if let Some(p) = self.members.remove(&d) {
            if let Some(l) = p.link {
                l.close(CLOSE_NOT_ADMITTED, b"not a member");
                self.retired.insert(l.id);
                let _ = self.ev.send(NetEvent::Down { peer: d });
            }
            if p.probation.is_none() {
                self.book.forget(&d);
            }
        }
    }

    fn spawn_open(&self, c: Conn) {
        let tx = self.link_tx.clone();
        let int = self.int_tx.clone();
        let timeout = self.cfg.timing.handshake_timeout;
        tokio::spawn(async move {
            let peer = c.peer;
            let dialed = c.dialed;
            match link::open(c, tx, timeout).await {
                Ok(l) => {
                    let _ = int.send(Internal::Opened(l, Instant::now()));
                }
                Err(e) => {
                    tracing::debug!(error = %e, "link open failed");
                    if dialed {
                        let _ = int.send(Internal::DialFailed(peer));
                    }
                }
            }
        });
    }

    fn on_internal(&mut self, i: Internal) {
        match i {
            Internal::DialFailed(d) => {
                let max = self.cfg.backoff_max;
                if let Some(p) = self.members.get_mut(&d) {
                    p.dialing = false;
                    p.backoff = (p.backoff * 2).max(Duration::from_millis(250)).min(max);
                    p.next_dial = Instant::now() + p.backoff;
                }
            }
            Internal::Opened(l, at) => self.on_opened(l, at),
        }
    }

    /// Prefer the connection dialed by the smaller key, so two simultaneous dials settle on
    /// the same connection on both sides.
    fn preferred(&self, l: &Link) -> bool {
        l.dialed == (self.id < l.peer)
    }

    fn on_opened(&mut self, l: Link, at: Instant) {
        let pref_new = self.preferred(&l);
        let id = self.id;
        let Some(p) = self.members.get_mut(&l.peer) else {
            l.close(CLOSE_NOT_ADMITTED, b"not a member");
            self.pending.remove(&l.id);
            self.retired.insert(l.id);
            return;
        };
        if l.dialed {
            p.dialing = false;
        }
        if let Some(old) = &p.link
            && !old.is_closed()
        {
            let old_pref = old.dialed == (id < old.peer);
            let race = p
                .link_since
                .is_some_and(|s| at.saturating_duration_since(s) < Duration::from_secs(2));
            if race && old_pref && !pref_new {
                l.close(CLOSE_DUPLICATE, b"duplicate");
                self.pending.remove(&l.id);
                self.retired.insert(l.id);
                return;
            }
            old.close(CLOSE_DUPLICATE, b"duplicate");
            self.retired.insert(old.id);
            let _ = self.ev.send(NetEvent::Down { peer: l.peer });
        }
        p.link = Some(l.clone());
        p.link_since = Some(at);
        p.ready = false;
        p.backoff = Duration::ZERO;
        if p.probation.is_none() {
            self.book
                .connected(l.peer, l.remote, now_ms(), &self.cfg.policy);
        }
        let (peer, lid) = (l.peer, l.id);
        let _ = self.ev.send(NetEvent::Up(l));
        if let Some((frames, closed)) = self.pending.remove(&lid) {
            for frame in frames {
                let _ = self.ev.send(NetEvent::Frame { peer, frame });
            }
            if closed {
                self.on_link_event(LinkEvent::Closed { peer, link: lid });
            }
        }
        self.update_paths();
    }

    fn on_link_event(&mut self, e: LinkEvent) {
        match e {
            LinkEvent::Frame { peer, link, frame } => {
                let current = self
                    .members
                    .get(&peer)
                    .and_then(|p| p.link.as_ref())
                    .is_some_and(|l| l.id == link);
                if current {
                    let _ = self.ev.send(NetEvent::Frame { peer, frame });
                } else if !self.retired.contains(&link) {
                    // Not registered yet: its Opened message is still on the way.
                    let e = self.pending.entry(link).or_default();
                    if e.0.len() < 256 {
                        e.0.push(frame);
                    }
                }
            }
            LinkEvent::Closed { peer, link } => {
                let now = Instant::now();
                let current = self
                    .members
                    .get(&peer)
                    .and_then(|p| p.link.as_ref())
                    .is_some_and(|l| l.id == link);
                if !current && !self.retired.contains(&link) {
                    self.pending.entry(link).or_default().1 = true;
                    return;
                }
                self.retired.insert(link);
                self.pending.remove(&link);
                if self.members.get(&peer).is_some_and(|p| {
                    !p.is_member() && p.link.as_ref().is_some_and(|l| l.id == link)
                }) {
                    // A probation link ended: forget the peer.
                    self.members.remove(&peer);
                    let _ = self.ev.send(NetEvent::Down { peer });
                    return;
                }
                if let Some(p) = self.members.get_mut(&peer)
                    && p.link.as_ref().is_some_and(|l| l.id == link)
                {
                    p.link = None;
                    p.link_since = None;
                    p.ready = false;
                    p.down_since = now;
                    p.next_dial = now + dial_delay(self.id, peer);
                    let _ = self.ev.send(NetEvent::Down { peer });
                    self.update_paths();
                }
            }
        }
    }

    fn on_wake(&mut self) {
        tracing::info!("wake or network change: rescan and redial");
        let list = self.scan();
        if let Err(e) = self.apply_ifaces(list, false) {
            tracing::warn!(error = %e, "rebind failed");
        }
        let now = Instant::now();
        let me = self.id;
        for (d, p) in self.members.iter_mut().filter(|(_, p)| p.is_member()) {
            p.next_dial = now + dial_delay(me, *d);
            p.backoff = Duration::ZERO;
            if p.link.is_none() {
                p.down_since = now;
            }
        }
        self.advertised = None;
    }

    fn end_old_probation(&mut self, now: Instant) {
        let old: Vec<DeviceId> = self
            .members
            .iter()
            .filter(|(_, p)| {
                p.probation
                    .is_some_and(|s| now.saturating_duration_since(s) >= PROBATION)
            })
            .map(|(d, _)| *d)
            .collect();
        for d in old {
            tracing::debug!("probation link timed out");
            self.drop_peer(d);
        }
    }

    fn on_tick(&mut self) {
        let now = Instant::now();
        let wall = now_ms();
        let (last, last_wall) = self.last_tick;
        let mono_gap = now.saturating_duration_since(last);
        let wall_gap = Duration::from_millis(wall.saturating_sub(last_wall));
        self.last_tick = (now, wall);
        let late = (self.cfg.tick * 20).max(Duration::from_secs(5));
        if mono_gap > late || wall_gap > mono_gap + late {
            // The process was frozen (sleep, SIGSTOP) or the clock jumped.
            self.on_wake();
        } else if now.saturating_duration_since(self.last_scan) >= self.cfg.iface_poll {
            self.last_scan = now;
            let list = self.scan();
            if list != self.ifaces
                && let Err(e) = self.apply_ifaces(list, false)
            {
                tracing::warn!(error = %e, "rebind failed");
            }
        }
        self.end_old_probation(now);
        self.mdns(wall);
        self.dial_due(now);
        self.update_paths();
    }

    fn mdns(&mut self, wall: u64) {
        let Some(ep) = self.ep.clone() else { return };
        if let Some(secret) = self.secrets.current {
            let name = tags::instance_name(&tags::tag(&secret, tags::slot(wall), &self.id));
            if self.advertised.as_deref() != Some(name.as_str()) {
                let ips: Vec<IpAddr> = ep.local_addrs().iter().map(|a| a.ip()).collect();
                match self.disc.advertise(&name, ep.port(), &ips) {
                    Ok(()) => self.advertised = Some(name),
                    Err(e) => tracing::warn!(error = %e, "mDNS advertise failed"),
                }
            }
        }
        let members = self.member_ids();
        for f in self.disc.poll() {
            let Some(t) = tags::parse_instance(&f.instance) else {
                continue;
            };
            let Some(d) = tags::recognise(&t, &self.secrets, &members, wall) else {
                continue;
            };
            let mut new = false;
            for ip in f.addrs {
                let a = SocketAddr::new(ip, f.port);
                if addr::allowed_any(a, &self.ifaces, &self.cfg.policy) {
                    new |= self.book.add(d, a, Source::Mdns, wall, &self.cfg.policy);
                }
            }
            if new {
                self.dial_soon(d);
            }
        }
    }

    /// Keys that are members (probation peers left out).
    fn member_ids(&self) -> Vec<DeviceId> {
        self.members
            .iter()
            .filter(|(_, p)| p.is_member())
            .map(|(d, _)| *d)
            .collect()
    }

    fn dial_soon(&mut self, d: DeviceId) {
        let delay = dial_delay(self.id, d);
        if let Some(p) = self.members.get_mut(&d) {
            p.next_dial = Instant::now() + delay;
            p.backoff = Duration::ZERO;
        }
    }

    fn dial_due(&mut self, now: Instant) {
        let Some(ep) = self.ep.clone() else { return };
        let due: Vec<DeviceId> = self
            .members
            .iter()
            .filter(|(_, p)| p.is_member() && p.link.is_none() && !p.dialing && p.next_dial <= now)
            .map(|(d, _)| *d)
            .collect();
        for d in due {
            let targets: Vec<SocketAddr> = self
                .book
                .dial_list(&d)
                .into_iter()
                .filter(|a| addr::allowed_any(*a, &self.ifaces, &self.cfg.policy))
                .collect();
            let max = self.cfg.backoff_max;
            let Some(p) = self.members.get_mut(&d) else {
                continue;
            };
            if targets.is_empty() {
                p.backoff = (p.backoff * 2).max(Duration::from_millis(250)).min(max);
                p.next_dial = now + p.backoff;
                continue;
            }
            p.dialing = true;
            let ep = ep.clone();
            let int = self.int_tx.clone();
            let link_tx = self.link_tx.clone();
            let timeout = self.cfg.timing.handshake_timeout;
            tokio::spawn(async move {
                let mut set = JoinSet::new();
                for (i, a) in targets.into_iter().enumerate() {
                    let ep = ep.clone();
                    set.spawn(async move {
                        // Stagger a little: the best address goes first.
                        tokio::time::sleep(Duration::from_millis(100 * i as u64)).await;
                        ep.dial(d, a, ALPN_SYNC).await
                    });
                }
                while let Some(r) = set.join_next().await {
                    if let Ok(Ok(c)) = r {
                        set.abort_all();
                        match link::open(c, link_tx, timeout).await {
                            Ok(l) => {
                                let _ = int.send(Internal::Opened(l, Instant::now()));
                            }
                            Err(_) => {
                                let _ = int.send(Internal::DialFailed(d));
                            }
                        }
                        return;
                    }
                }
                let _ = int.send(Internal::DialFailed(d));
            });
        }
    }

    fn seen_on_lan(&self, d: &DeviceId) -> bool {
        self.book.get(d).is_some_and(|e| {
            e.connected_ms > 0
                && e.addrs
                    .iter()
                    .any(|a| addr::allowed_any(a.addr, &self.ifaces, &self.cfg.policy))
        })
    }

    fn update_paths(&mut self) {
        let now = Instant::now();
        let ds = self.member_ids();
        for d in ds {
            let seen = self.seen_on_lan(&d);
            let peer_vpn = self.book.get(&d).is_some_and(|e| e.vpn_up);
            let local_vpn = self.vpn_up;
            let delay = self.cfg.path_delay;
            let Some(p) = self.members.get_mut(&d) else {
                continue;
            };
            let inputs = path::Inputs {
                connected: p.link.is_some(),
                seen_on_lan: seen,
                local_vpn,
                peer_vpn,
            };
            let s = path::evaluate(inputs, now.saturating_duration_since(p.down_since), delay);
            if p.path != Some(s) {
                p.path = Some(s);
                let _ = self.ev.send(NetEvent::Path { peer: d, state: s });
            }
        }
    }

    /// Our own signed announcement, re-signed when the addresses, port or VPN state change.
    fn own_announcement(&mut self) -> Option<Vec<u8>> {
        let ep = self.ep.as_ref()?;
        let mut addrs: Vec<IfAddr> = ep
            .sockets()
            .into_iter()
            .map(|s| IfAddr {
                ip: s.local.ip(),
                prefix: s
                    .interface
                    .addrs
                    .iter()
                    .find(|a| a.ip == s.local.ip())
                    .map_or(0, |a| a.prefix),
            })
            .collect();
        addrs.sort();
        addrs.dedup();
        addrs.truncate(gossip::MAX_ADDRS);
        let port = ep.port();
        if let Some((a, p, v, signed)) = &self.own_ann
            && *a == addrs
            && *p == port
            && *v == self.vpn_up
        {
            return Some(signed.clone());
        }
        self.own_seq = (self.own_seq + 1).max(now_ms());
        let signed = gossip::sign(
            &self.cfg.key,
            &Announcement {
                device: self.id,
                seq: self.own_seq,
                port,
                vpn_up: self.vpn_up,
                addrs: addrs.clone(),
            },
        );
        self.own_ann = Some((addrs, port, self.vpn_up, signed.clone()));
        Some(signed)
    }

    fn send_frame(&self, d: &DeviceId, m: &Message) {
        if let Some(l) = self.members.get(d).and_then(|p| p.link.as_ref()) {
            let _ = l.send(msg::encode(m));
        }
    }

    fn send_announcements_to(&mut self, d: DeviceId) {
        let mut list = Vec::new();
        if let Some(own) = self.own_announcement() {
            list.push(own);
        }
        for (other, e) in self
            .members
            .iter()
            .filter(|(_, p)| p.is_member())
            .filter_map(|(o, _)| self.book.get(o).map(|e| (o, e)))
        {
            if *other != d
                && let Some(a) = &e.announcement
            {
                list.push(a.clone());
            }
        }
        for signed in list {
            self.send_frame(&d, &Message::AddrUpdate { signed });
        }
    }

    fn announce_all(&mut self) {
        let Some(own) = self.own_announcement() else {
            return;
        };
        let ready: Vec<DeviceId> = self
            .members
            .iter()
            .filter(|(_, p)| p.ready)
            .map(|(d, _)| *d)
            .collect();
        for d in ready {
            self.send_frame(
                &d,
                &Message::AddrUpdate {
                    signed: own.clone(),
                },
            );
        }
    }

    fn on_addr_update(&mut self, from: DeviceId, signed: Vec<u8>) {
        let members = self.member_ids();
        if !members.contains(&from) {
            return;
        }
        match self
            .book
            .apply_announcement(&signed, &members, now_ms(), &self.cfg.policy)
        {
            Ok(Applied::New(a)) => {
                if self
                    .members
                    .get(&a.device)
                    .is_some_and(|p| p.link.is_none())
                {
                    self.dial_soon(a.device);
                }
                let to: Vec<DeviceId> = self
                    .members
                    .iter()
                    .filter(|(d, p)| p.ready && **d != from && **d != a.device)
                    .map(|(d, _)| *d)
                    .collect();
                for d in to {
                    self.send_frame(
                        &d,
                        &Message::AddrUpdate {
                            signed: signed.clone(),
                        },
                    );
                }
            }
            Ok(Applied::Old) => {}
            Err(e) => tracing::debug!(error = %e, "address update refused"),
        }
    }
}

async fn recv_opt(rx: &mut Option<mpsc::Receiver<Conn>>) -> Option<Conn> {
    match rx {
        Some(r) => r.recv().await,
        None => std::future::pending().await,
    }
}
