//! PAKE pairing and the group membership chain.
//!
//! This crate implements `docs/protocol.md` (version 1):
//!
//! - [`pake`]: the SPAKE2 wrapper with low-order point rejection, the transcript and the
//!   confirmation MACs (4.4, 4.5).
//! - [`frame`]: the pairing frames (4.4, 4.7).
//! - [`session`]: the inviter and joiner state machines, try limits, expiry and lockout (4.2 to
//!   4.7). The transport is the [`session::PairTransport`] trait; M2 wires QUIC.
//! - [`qr`]: the QR payload and its strict parser (4.3).
//! - [`lan`]: the LAN rule (4.8).
//! - [`chain`]: signed changes, the graph and the two-layer validity rule (section 5).
//! - [`gate`]: the order of a sync connection (5.9).

pub mod chain;
pub mod frame;
pub mod gate;
pub mod lan;
pub mod pake;
pub mod qr;
pub mod session;

pub use pake::{ALPN, EXPORTER_LABEL};
