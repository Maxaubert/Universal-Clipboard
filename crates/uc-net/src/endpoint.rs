//! The LAN-only QUIC endpoint: one UDP socket per address of each physical interface.
//!
//! What it does NOT have, on purpose (plan 4.1, "never reachable from outside the LAN"):
//! no relay, no address lookup service (no DNS, no pkarr), no hole punching or NAT traversal,
//! no port mapping (UPnP, NAT-PMP, PCP), no wildcard bind, no connection migration. See
//! `docs/net.md` for why this is plain `quinn` and not iroh.
//!
//! The LAN rule ([`crate::addr`]) runs on the first packet of every incoming connection, before
//! the handshake: a remote address that fails it is dropped without an answer
//! (`Incoming::ignore`). Every dial target is checked the same way.

use std::net::{IpAddr, SocketAddr, SocketAddrV6};
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use ed25519_dalek::SigningKey;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uc_core::DeviceId;

use crate::addr::{self, AddrClass, Policy};
use crate::binder::{SocketBinder, Tie};
use crate::ifaces::Interface;
use crate::tls;

/// ALPN of sync connections.
pub const ALPN_SYNC: &[u8] = b"uclip/1";
/// ALPN of pairing connections.
pub const ALPN_PAIR: &[u8] = b"uclip-pair/1";

/// Timers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// QUIC keep-alive (plan 3.3: 15 s).
    pub keep_alive: Duration,
    /// Idle timeout: a dead path is found within this time (plan 3.3: under 30 s).
    pub idle_timeout: Duration,
    /// Limit for one dial or one handshake.
    pub handshake_timeout: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            keep_alive: Duration::from_secs(15),
            idle_timeout: Duration::from_secs(29),
            handshake_timeout: Duration::from_secs(5),
        }
    }
}

/// One established, authenticated QUIC connection.
#[derive(Clone, Debug)]
pub struct Conn {
    /// The connection.
    pub conn: quinn::Connection,
    /// The other side's device key (from the TLS handshake).
    pub peer: DeviceId,
    /// The ALPN.
    pub alpn: Vec<u8>,
    /// The other side's address.
    pub remote: SocketAddr,
    /// Our interface name.
    pub iface: String,
    /// Did we dial it?
    pub dialed: bool,
}

/// Why a bind failed.
#[derive(Debug)]
pub enum BindError {
    /// The fixed port is taken on one of the addresses.
    PortInUse(u16),
    /// Another I/O error.
    Io(std::io::Error),
    /// TLS setup failed.
    Tls(tls::TlsError),
}

impl std::fmt::Display for BindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BindError::PortInUse(p) => write!(f, "UDP port {p} is in use"),
            BindError::Io(e) => write!(f, "bind failed: {e}"),
            BindError::Tls(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for BindError {}

/// Why a dial failed.
#[derive(Debug)]
pub enum DialError {
    /// The address fails the LAN rule, or no bound interface can reach it.
    NotLan(SocketAddr),
    /// The handshake did not finish in time.
    Timeout,
    /// The other side is not the device we wanted, or the ALPN differs.
    WrongPeer,
    /// QUIC error.
    Quic(String),
}

impl std::fmt::Display for DialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DialError::NotLan(a) => write!(f, "{a} is not on this LAN"),
            DialError::Timeout => f.write_str("dial timed out"),
            DialError::WrongPeer => f.write_str("wrong peer"),
            DialError::Quic(e) => write!(f, "QUIC: {e}"),
        }
    }
}

impl std::error::Error for DialError {}

/// One bound socket.
#[derive(Debug)]
pub struct BoundSocket {
    iface: RwLock<Interface>,
    /// The local address.
    pub local: SocketAddr,
    /// How it is tied to the interface.
    pub tie: Tie,
    ep: quinn::Endpoint,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl BoundSocket {
    /// The interface it belongs to (its addresses can change while the socket stays).
    pub fn iface(&self) -> Interface {
        self.iface
            .read()
            .map(|i| i.clone())
            .unwrap_or_else(|e| e.into_inner().clone())
    }

    fn allows(&self, remote: SocketAddr, policy: &Policy) -> bool {
        match self.iface.read() {
            Ok(i) => addr::allowed_on(remote, &i, policy),
            Err(_) => false,
        }
    }

    fn shut(&self) {
        self.ep.close(CLOSE_SHUTDOWN.into(), b"bye");
        if let Ok(mut t) = self.task.lock()
            && let Some(t) = t.take()
        {
            t.abort();
        }
    }
}

/// One bound socket, as diagnostics show it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SocketInfo {
    /// Interface name.
    pub iface: String,
    /// The interface record.
    pub interface: Interface,
    /// Local address.
    pub local: SocketAddr,
    /// Tie.
    pub tie: Tie,
}

/// What [`LanEndpoint::update`] changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// New sockets.
    pub added: Vec<SocketAddr>,
    /// Closed sockets (their links are gone).
    pub removed: Vec<SocketAddr>,
}

impl Changes {
    /// Nothing changed.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

/// The endpoint.
pub struct LanEndpoint {
    key: SigningKey,
    port: AtomicU16,
    policy: Policy,
    timing: Timing,
    server: quinn::ServerConfig,
    runtime: Arc<dyn quinn::Runtime>,
    bound: RwLock<Vec<Arc<BoundSocket>>>,
    refused: Arc<AtomicU64>,
    tx: mpsc::Sender<Conn>,
}

impl std::fmt::Debug for LanEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LanEndpoint")
            .field("port", &self.port())
            .field("sockets", &self.sockets())
            .finish()
    }
}

fn transport(t: &Timing) -> Arc<quinn::TransportConfig> {
    let mut c = quinn::TransportConfig::default();
    c.keep_alive_interval(Some(t.keep_alive));
    c.max_idle_timeout(quinn::IdleTimeout::try_from(t.idle_timeout).ok());
    c.max_concurrent_uni_streams(0u8.into());
    c.max_concurrent_bidi_streams(4u8.into());
    Arc::new(c)
}

/// The addresses of `iface` that the endpoint binds: only LAN-class addresses (never public
/// ones, so on a network with only public addresses nothing listens).
pub fn bind_addrs(iface: &Interface, port: u16, policy: &Policy) -> Vec<SocketAddr> {
    iface
        .addrs
        .iter()
        .filter(|a| addr::is_lan_class(a.ip, policy))
        .map(|a| match a.ip {
            IpAddr::V6(v6) if addr::classify(a.ip) == AddrClass::LinkLocalV6 => {
                SocketAddr::V6(SocketAddrV6::new(v6, port, 0, iface.index))
            }
            ip => SocketAddr::new(ip, port),
        })
        .collect()
}

fn eligible(iface: &Interface, policy: &Policy) -> bool {
    iface.usable()
        || (policy.allow_loopback
            && iface.up
            && iface.kind == crate::ifaces::Kind::Loopback
            && !iface.addrs.is_empty())
}

fn same_addr(a: &SocketAddr, b: &SocketAddr) -> bool {
    a.ip() == b.ip()
}

impl LanEndpoint {
    /// Bind one socket per LAN address of every given interface, all on `port` (0 picks one).
    /// Only interfaces that are physical (or loopback under [`Policy::allow_loopback`]) are
    /// used. Returns the endpoint and the stream of accepted connections.
    pub fn bind(
        key: SigningKey,
        ifaces: &[Interface],
        port: u16,
        policy: Policy,
        timing: Timing,
        binder: &dyn SocketBinder,
        alpns: &[Vec<u8>],
    ) -> Result<(Self, mpsc::Receiver<Conn>), BindError> {
        let mut tries = 0;
        loop {
            match Self::bind_once(key.clone(), ifaces, port, policy, timing, binder, alpns) {
                Err(BindError::PortInUse(_)) if port == 0 && tries < 8 => tries += 1,
                r => return r,
            }
        }
    }

    fn bind_once(
        key: SigningKey,
        ifaces: &[Interface],
        port: u16,
        policy: Policy,
        timing: Timing,
        binder: &dyn SocketBinder,
        alpns: &[Vec<u8>],
    ) -> Result<(Self, mpsc::Receiver<Conn>), BindError> {
        let crypto = tls::server_crypto(&key, alpns).map_err(BindError::Tls)?;
        let mut server = quinn::ServerConfig::with_crypto(Arc::new(crypto));
        server.migration(false);
        server.max_incoming(64);
        server.transport_config(transport(&timing));
        let runtime = quinn::default_runtime()
            .ok_or_else(|| BindError::Io(std::io::Error::other("no tokio runtime")))?;
        let (tx, rx) = mpsc::channel(64);
        let ep = Self {
            key,
            port: AtomicU16::new(port),
            policy,
            timing,
            server,
            runtime,
            bound: RwLock::new(Vec::new()),
            refused: Arc::new(AtomicU64::new(0)),
            tx,
        };
        match ep.update(ifaces, binder) {
            Ok(_) => Ok((ep, rx)),
            Err(e) => {
                ep.close();
                Err(e)
            }
        }
    }

    fn bind_socket(
        &self,
        iface: &Interface,
        local: SocketAddr,
        binder: &dyn SocketBinder,
    ) -> Result<Option<Arc<BoundSocket>>, BindError> {
        let (sock, tie) = match binder.bind(iface, local) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                return Err(BindError::PortInUse(self.port()));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AddrNotAvailable => {
                // A tentative or vanished address: skip it, the next scan retries.
                tracing::debug!(%local, "address not available");
                return Ok(None);
            }
            Err(e) => return Err(BindError::Io(e)),
        };
        let ep = quinn::Endpoint::new(
            quinn::EndpointConfig::default(),
            Some(self.server.clone()),
            sock,
            self.runtime.clone(),
        )
        .map_err(BindError::Io)?;
        let local = ep.local_addr().map_err(BindError::Io)?;
        let _ = self
            .port
            .compare_exchange(0, local.port(), Ordering::Relaxed, Ordering::Relaxed);
        let b = Arc::new(BoundSocket {
            iface: RwLock::new(iface.clone()),
            local,
            tie,
            ep,
            task: Mutex::new(None),
        });
        let task = tokio::spawn(accept_loop(
            b.clone(),
            self.policy,
            self.timing,
            self.refused.clone(),
            self.tx.clone(),
        ));
        if let Ok(mut t) = b.task.lock() {
            *t = Some(task);
        }
        Ok(Some(b))
    }

    /// Follow an interface change: keep the sockets whose address is still there (their links
    /// stay up), close the ones whose address is gone, and bind new addresses on the same port.
    /// On [`BindError::PortInUse`] the caller binds a new endpoint on another port.
    pub fn update(
        &self,
        ifaces: &[Interface],
        binder: &dyn SocketBinder,
    ) -> Result<Changes, BindError> {
        let mut want: Vec<(Interface, SocketAddr)> = Vec::new();
        for i in ifaces.iter().filter(|i| eligible(i, &self.policy)) {
            for a in bind_addrs(i, self.port(), &self.policy) {
                if !want.iter().any(|(_, w)| same_addr(w, &a)) {
                    want.push((i.clone(), a));
                }
            }
        }
        let mut ch = Changes::default();
        // Hold the lock for the whole update, so a dial never sees a half-done list.
        let mut guard = match self.bound.write() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        let cur = &mut *guard;
        cur.retain(|b| {
            let keep = want
                .iter()
                .find(|(i, a)| same_addr(a, &b.local) && i.name == b.iface().name);
            match keep {
                Some((i, _)) => {
                    if let Ok(mut g) = b.iface.write() {
                        *g = i.clone();
                    }
                    true
                }
                None => {
                    b.shut();
                    ch.removed.push(b.local);
                    false
                }
            }
        });
        let mut result = Ok(());
        for (i, a) in want {
            if cur.iter().any(|b| same_addr(&b.local, &a)) {
                continue;
            }
            let a = match a {
                SocketAddr::V4(v) => {
                    SocketAddr::V4(std::net::SocketAddrV4::new(*v.ip(), self.port()))
                }
                SocketAddr::V6(v) => {
                    SocketAddr::V6(SocketAddrV6::new(*v.ip(), self.port(), 0, v.scope_id()))
                }
            };
            match self.bind_socket(&i, a, binder) {
                Ok(Some(b)) => {
                    ch.added.push(b.local);
                    cur.push(b);
                }
                Ok(None) => {}
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        drop(guard);
        result.map(|()| ch)
    }

    fn list(&self) -> Vec<Arc<BoundSocket>> {
        match self.bound.read() {
            Ok(g) => g.clone(),
            Err(e) => e.into_inner().clone(),
        }
    }

    /// The UDP port of every socket.
    pub fn port(&self) -> u16 {
        self.port.load(Ordering::Relaxed)
    }

    /// Our device ID.
    pub fn id(&self) -> DeviceId {
        DeviceId(self.key.verifying_key().to_bytes())
    }

    /// The bound sockets.
    pub fn sockets(&self) -> Vec<SocketInfo> {
        self.list()
            .iter()
            .map(|b| {
                let i = b.iface();
                SocketInfo {
                    iface: i.name.clone(),
                    interface: i,
                    local: b.local,
                    tie: b.tie,
                }
            })
            .collect()
    }

    /// The local addresses we listen on.
    pub fn local_addrs(&self) -> Vec<SocketAddr> {
        self.list().iter().map(|b| b.local).collect()
    }

    /// Incoming connections dropped by the LAN rule so far.
    pub fn refused(&self) -> u64 {
        self.refused.load(Ordering::Relaxed)
    }

    /// Dial `peer` at `remote` with `alpn`. The address must pass the LAN rule on one of our
    /// bound interfaces, and the other side must show exactly `peer`'s key.
    pub async fn dial(
        &self,
        peer: DeviceId,
        remote: SocketAddr,
        alpn: &[u8],
    ) -> Result<Conn, DialError> {
        self.dial_opt(Some(peer), remote, alpn).await
    }

    /// Dial a pairing connection to `remote` without knowing the key there (code-mode
    /// pairing, protocol 4.4). Only [`ALPN_PAIR`] is allowed. The LAN rule applies as usual.
    pub async fn dial_pair_any(&self, remote: SocketAddr) -> Result<Conn, DialError> {
        self.dial_opt(None, remote, ALPN_PAIR).await
    }

    async fn dial_opt(
        &self,
        peer: Option<DeviceId>,
        remote: SocketAddr,
        alpn: &[u8],
    ) -> Result<Conn, DialError> {
        if peer.is_none() && alpn != ALPN_PAIR {
            return Err(DialError::WrongPeer);
        }
        let remote = SocketAddr::new(addr::canonical(remote.ip()), remote.port());
        let b = self
            .list()
            .into_iter()
            .find(|b| b.local.is_ipv6() == remote.is_ipv6() && b.allows(remote, &self.policy))
            .ok_or(DialError::NotLan(remote))?;
        let iface = b.iface();
        let target = match remote {
            SocketAddr::V6(v6)
                if v6.scope_id() == 0 && addr::classify(remote.ip()) == AddrClass::LinkLocalV6 =>
            {
                SocketAddr::V6(SocketAddrV6::new(*v6.ip(), v6.port(), 0, iface.index))
            }
            r => r,
        };
        let crypto = match peer {
            Some(p) => tls::client_crypto(&self.key, p, alpn),
            None => tls::client_crypto_any(&self.key, alpn),
        }
        .map_err(|e| DialError::Quic(e.to_string()))?;
        let mut cfg = quinn::ClientConfig::new(Arc::new(crypto));
        cfg.transport_config(transport(&self.timing));
        let connecting =
            b.ep.connect_with(cfg, target, tls::SERVER_NAME)
                .map_err(|e| DialError::Quic(e.to_string()))?;
        let conn = match tokio::time::timeout(self.timing.handshake_timeout, connecting).await {
            Err(_) => return Err(DialError::Timeout),
            Ok(Err(e)) => return Err(DialError::Quic(e.to_string())),
            Ok(Ok(c)) => c,
        };
        let got = tls::peer_key(&conn);
        let key_ok = match peer {
            Some(p) => got == Some(p),
            None => got.is_some(),
        };
        let (Some(peer), true, true) = (got, key_ok, tls::alpn(&conn).as_deref() == Some(alpn))
        else {
            conn.close(CLOSE_WRONG_PEER.into(), b"wrong peer");
            return Err(DialError::WrongPeer);
        };
        Ok(Conn {
            conn,
            peer,
            alpn: alpn.to_vec(),
            remote: target,
            iface: iface.name,
            dialed: true,
        })
    }

    /// Close every socket.
    pub fn close(&self) {
        for b in self.list() {
            b.shut();
        }
    }
}

impl Drop for LanEndpoint {
    fn drop(&mut self) {
        self.close();
    }
}

/// Close code: endpoint shutdown.
pub const CLOSE_SHUTDOWN: u32 = 0;
/// Close code: the server key or ALPN was not the one we dialed.
pub const CLOSE_WRONG_PEER: u32 = 1;
/// Close code: the key is not admitted.
pub const CLOSE_NOT_ADMITTED: u32 = 2;
/// Close code: a better connection to the same peer exists.
pub const CLOSE_DUPLICATE: u32 = 3;

async fn accept_loop(
    b: Arc<BoundSocket>,
    policy: Policy,
    timing: Timing,
    refused: Arc<AtomicU64>,
    tx: mpsc::Sender<Conn>,
) {
    while let Some(inc) = b.ep.accept().await {
        let remote = inc.remote_address();
        if !b.allows(remote, &policy) {
            // Closed before the handshake, without an answer (plan 4.1).
            refused.fetch_add(1, Ordering::Relaxed);
            tracing::debug!(%remote, "dropped: not on this LAN");
            inc.ignore();
            continue;
        }
        let tx = tx.clone();
        let iface = b.iface().name;
        tokio::spawn(async move {
            let Ok(connecting) = inc.accept() else {
                return;
            };
            let Ok(Ok(conn)) = tokio::time::timeout(timing.handshake_timeout, connecting).await
            else {
                return;
            };
            let (Some(peer), Some(alpn)) = (tls::peer_key(&conn), tls::alpn(&conn)) else {
                conn.close(CLOSE_WRONG_PEER.into(), b"no key");
                return;
            };
            let _ = tx
                .send(Conn {
                    remote: conn.remote_address(),
                    conn,
                    peer,
                    alpn,
                    iface,
                    dialed: false,
                })
                .await;
        });
    }
}
