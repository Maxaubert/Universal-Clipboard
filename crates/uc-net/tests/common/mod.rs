//! Shared helpers for the loopback tests: several [`Net`] nodes on 127.0.0.x in one process,
//! with fake mDNS and, optionally, a sync engine with a fake clipboard on each.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use uc_clip::FakeClipboard;
use uc_core::sync::{Config, Engine, Event};
use uc_core::{DeviceId, DeviceKind, DeviceRecord, Message, Role, msg};
use uc_net::ifaces::{IfAddr, Interface, Kind};
use uc_net::{
    AddressBinder, FakeLan, FakeMdns, Net, NetConfig, NetEvent, PathState, Policy, Secrets, Timing,
};

pub const SECRET: [u8; 32] = [42; 32];

pub fn key(n: u8) -> SigningKey {
    SigningKey::from_bytes(&[n; 32])
}

pub fn id(n: u8) -> DeviceId {
    DeviceId(key(n).verifying_key().to_bytes())
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A loopback "physical" interface with one address.
pub fn lo(ip: &str, prefix: u8) -> Interface {
    Interface {
        name: "lo-test".into(),
        index: 1,
        description: "test loopback".into(),
        kind: Kind::Loopback,
        up: true,
        addrs: vec![IfAddr {
            ip: ip.parse().unwrap(),
            prefix,
        }],
    }
}

pub fn vpn_iface() -> Interface {
    Interface {
        name: "wg-test".into(),
        index: 99,
        description: "test VPN".into(),
        kind: Kind::Vpn,
        up: true,
        addrs: vec![IfAddr {
            ip: "10.64.0.2".parse().unwrap(),
            prefix: 32,
        }],
    }
}

/// Loopback address for test node `n`: 127.0.0.n on Windows and Linux. macOS has only
/// 127.0.0.1 on lo0, so there every node shares it (with its own port).
pub fn ip(n: u8) -> String {
    if cfg!(target_os = "macos") {
        "127.0.0.1".into()
    } else {
        format!("127.0.0.{n}")
    }
}

pub type Ifaces = Arc<Mutex<Vec<Interface>>>;

/// One test node.
pub struct Node {
    pub n: u8,
    pub net: Net,
    pub mdns: FakeMdns,
    pub ifaces: Ifaces,
    pub engine: Option<Engine>,
    pub clip: FakeClipboard,
    pub links: BTreeMap<DeviceId, uc_net::Link>,
    pub paths: BTreeMap<DeviceId, PathState>,
    pub ups: usize,
    pub frames: Vec<(DeviceId, Vec<u8>)>,
    pub pairing: Vec<uc_net::Conn>,
    pub port_changes: Vec<u16>,
    pub applied: usize,
}

pub fn timing() -> Timing {
    Timing {
        keep_alive: Duration::from_millis(300),
        idle_timeout: Duration::from_millis(1500),
        handshake_timeout: Duration::from_millis(1500),
    }
}

pub fn config(n: u8, port: u16, ifaces: &Ifaces) -> NetConfig {
    let mut c = NetConfig::new(key(n), port);
    c.policy = Policy {
        allow_loopback: true,
    };
    c.timing = timing();
    c.tick = Duration::from_millis(50);
    c.iface_poll = Duration::from_millis(100);
    c.path_delay = Duration::from_millis(1500);
    c.backoff_max = Duration::from_millis(400);
    let f = ifaces.clone();
    c.ifaces = Some(Arc::new(move || f.lock().unwrap().clone()));
    c
}

impl Node {
    pub fn start(n: u8, ip: &str, port: u16, lan: &FakeLan) -> Node {
        let ifaces: Ifaces = Arc::new(Mutex::new(vec![lo(ip, 8)]));
        Self::start_with(n, port, lan, ifaces)
    }

    pub fn start_with(n: u8, port: u16, lan: &FakeLan, ifaces: Ifaces) -> Node {
        Self::start_cfg(n, port, lan, ifaces, |_| {})
    }

    pub fn start_cfg(
        n: u8,
        port: u16,
        lan: &FakeLan,
        ifaces: Ifaces,
        tweak: impl FnOnce(&mut NetConfig),
    ) -> Node {
        let mdns = lan.node();
        let mut cfg = config(n, port, &ifaces);
        tweak(&mut cfg);
        let net = Net::start(
            cfg,
            Arc::new(AddressBinder),
            Arc::new(mdns.clone()),
            uc_net::LanBook::new(),
        )
        .expect("start");
        Node {
            n,
            net,
            mdns,
            ifaces,
            engine: None,
            clip: FakeClipboard::new(),
            links: BTreeMap::new(),
            paths: BTreeMap::new(),
            ups: 0,
            frames: Vec::new(),
            pairing: Vec::new(),
            port_changes: Vec::new(),
            applied: 0,
        }
    }

    pub fn with_engine(mut self) -> Self {
        let rec = DeviceRecord {
            id: id(self.n),
            name: format!("node {}", self.n),
            kind: DeviceKind::Linux,
        };
        self.engine = Some(Engine::new(Config::new(rec, Role::Full)));
        self
    }

    pub fn join(&self, members: &[u8]) {
        self.net.set_members(
            members
                .iter()
                .filter(|m| **m != self.n)
                .map(|m| id(*m))
                .collect(),
        );
        self.net.set_secrets(Secrets {
            current: Some(SECRET),
            previous: None,
        });
    }

    /// Handle every pending event (non-blocking).
    pub fn pump(&mut self) {
        while let Some(e) = self.net.try_event() {
            match e {
                NetEvent::Up(l) => {
                    self.ups += 1;
                    if let Some(eng) = &mut self.engine {
                        eng.connect(Box::new(l.peer_handle()), now_ms());
                    }
                    // No membership chain in M2: admit at once.
                    self.net.ready(l.peer);
                    self.links.insert(l.peer, l);
                }
                NetEvent::Down { peer } => {
                    self.links.remove(&peer);
                    if let Some(eng) = &mut self.engine {
                        eng.disconnect(peer);
                    }
                }
                NetEvent::Frame { peer, frame } => {
                    if let Some(eng) = &mut self.engine {
                        for ev in eng.on_frame(peer, &frame, &mut self.clip, now_ms()) {
                            match ev {
                                Event::AddrUpdate { peer, signed } => {
                                    self.net.addr_update(peer, signed)
                                }
                                Event::Applied { .. } => self.applied += 1,
                                _ => {}
                            }
                        }
                    } else if let Ok(Some(Message::AddrUpdate { signed })) = msg::decode(&frame) {
                        self.net.addr_update(peer, signed);
                    } else {
                        self.frames.push((peer, frame));
                    }
                }
                NetEvent::Path { peer, state } => {
                    self.paths.insert(peer, state);
                }
                NetEvent::Pairing(c) => self.pairing.push(c),
                NetEvent::PortChanged(p) => self.port_changes.push(p),
                NetEvent::Interfaces { .. } => {}
            }
        }
    }

    pub fn copy(&mut self, text: &str) {
        self.clip.copy_text(text);
        if let Some(eng) = &mut self.engine {
            eng.on_local_change(&mut self.clip, now_ms());
        }
    }

    pub fn ips(&self) -> Vec<IpAddr> {
        self.ifaces
            .lock()
            .unwrap()
            .iter()
            .flat_map(|i| i.addrs.iter().map(|a| a.ip))
            .collect()
    }
}

/// Pump all nodes until `cond` holds or `limit` passes. Returns the time it took.
pub async fn until(
    nodes: &mut [&mut Node],
    limit: Duration,
    mut cond: impl FnMut(&[&mut Node]) -> bool,
) -> Option<Duration> {
    let start = Instant::now();
    loop {
        for n in nodes.iter_mut() {
            n.pump();
        }
        if cond(nodes) {
            return Some(start.elapsed());
        }
        if start.elapsed() > limit {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Pump all nodes for a while.
pub async fn settle(nodes: &mut [&mut Node], d: Duration) {
    let _ = until(nodes, d, |_| false).await;
}
