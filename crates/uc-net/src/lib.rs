//! LAN-only networking: QUIC with raw-key TLS, physical interface detection, the LAN rule,
//! LAN discovery (remembered addresses, signed gossip, mDNS with rotating tags), reconnect and
//! path state with the VPN-blocks-LAN rule.
//!
//! **Never reachable from outside the LAN** (plan 4.1). This crate has no relay, no address
//! lookup service (no DNS, no pkarr), no hole punching or NAT traversal and no port mapping
//! (no UPnP, NAT-PMP or PCP). It binds only LAN-class addresses of physical interfaces, never a
//! wildcard address, and drops every packet from outside the LAN before the TLS handshake.
//! `tests/lan_only.rs` checks this. `docs/net.md` says why this is plain `quinn` and not iroh.
//!
//! | Module | Plan section |
//! |---|---|
//! | [`addr`] | the LAN rule (protocol 4.8, plan 4.1) |
//! | [`ifaces`] | physical interface detection (3.5) |
//! | [`binder`] | socket binding trait, per-interface sockets (3.1, 3.5) |
//! | [`tls`] | TLS 1.3 with raw Ed25519 keys (3.3) |
//! | [`endpoint`] | the QUIC endpoint (3.3) |
//! | [`link`] | a sync link and the `uc_core::Peer` over it (3.3, 3.8) |
//! | [`book`] | `LanBook`, remembered addresses (3.4) |
//! | [`gossip`] | signed address announcements (3.4) |
//! | [`tags`] | rotating discovery tags (3.4, protocol 5.8) |
//! | [`discovery`] | mDNS trait and the `mdns-sd` implementation (3.4) |
//! | [`path`] | path state, VPN-blocks-LAN rule (3.5, 2.7) |
//! | [`node`] | [`Net`]: the whole thing as one task, reconnect on wake and network change |

pub mod addr;
pub mod binder;
pub mod book;
pub mod discovery;
pub mod endpoint;
pub mod gossip;
pub mod ifaces;
pub mod link;
pub mod node;
pub mod path;
pub mod tags;
pub mod tls;

pub use addr::Policy;
pub use binder::{AddressBinder, OsBinder, SocketBinder, Tie};
pub use book::LanBook;
pub use discovery::{Discovery, FakeLan, FakeMdns, MdnsSd, NoDiscovery};
pub use endpoint::{ALPN_PAIR, ALPN_SYNC, Conn, LanEndpoint, Timing};
pub use ifaces::{ClassifyOptions, Interface, Kind};
pub use link::{Link, QuicPeer};
pub use node::{Net, NetConfig, NetEvent, NetHandle, Status};
pub use path::{Attention, NoPath, PathState};
pub use tags::Secrets;

/// What this crate does not do, as data, so a test and the diagnostics page can show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LanOnly {
    /// A relay server is used.
    pub relay: bool,
    /// An address lookup service (DNS, pkarr, n0 discovery) is used.
    pub address_lookup: bool,
    /// Hole punching / NAT traversal is done.
    pub hole_punching: bool,
    /// The router is asked to open a port (UPnP, NAT-PMP, PCP).
    pub port_mapping: bool,
    /// A wildcard address (`0.0.0.0`, `::`) is bound.
    pub wildcard_bind: bool,
    /// A connection may move to another address after the handshake (QUIC migration).
    pub migration: bool,
}

/// The facts for this build. All false.
pub const LAN_ONLY: LanOnly = LanOnly {
    relay: false,
    address_lookup: false,
    hole_punching: false,
    port_mapping: false,
    wildcard_bind: false,
    migration: false,
};
