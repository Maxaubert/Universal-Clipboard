//! A sync link: one QUIC connection with one bidirectional stream of frames, and the
//! [`uc_core::Peer`] implementation over it.
//!
//! Framing: `u32 big-endian length || frame`, frames as `uc_core::msg` encodes them. The dialer
//! opens the stream and writes one start byte, so the acceptor sees the stream at once. Frame
//! order is kept (one stream). A frame larger than [`uc_core::msg::MAX_FRAME`] closes the link.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::sync::mpsc;
use uc_core::sync::{Peer, PeerGone};
use uc_core::{DeviceId, msg};

use crate::endpoint::Conn;

const START: u8 = 0x55;

/// Something that happened on a link.
#[derive(Debug)]
pub enum LinkEvent {
    /// A frame arrived.
    Frame {
        /// From.
        peer: DeviceId,
        /// Which link (see [`Link::id`]).
        link: u64,
        /// The frame.
        frame: Vec<u8>,
    },
    /// The link is closed (either side, or the path died).
    Closed {
        /// Peer.
        peer: DeviceId,
        /// Which link.
        link: u64,
    },
}

/// An open sync link.
#[derive(Clone, Debug)]
pub struct Link {
    /// The peer.
    pub peer: DeviceId,
    /// A number unique among this process's links.
    pub id: u64,
    /// The peer's address.
    pub remote: SocketAddr,
    /// Our interface.
    pub iface: String,
    /// Did we dial?
    pub dialed: bool,
    conn: quinn::Connection,
    tx: mpsc::UnboundedSender<Vec<u8>>,
}

impl Link {
    /// A [`Peer`] for the sync engine. Frames sent through it keep their order.
    pub fn peer_handle(&self) -> QuicPeer {
        QuicPeer {
            peer: self.peer,
            tx: self.tx.clone(),
        }
    }

    /// Send one frame.
    pub fn send(&self, frame: Vec<u8>) -> Result<(), PeerGone> {
        self.tx.send(frame).map_err(|_| PeerGone)
    }

    /// Close the link with a code.
    pub fn close(&self, code: u32, reason: &[u8]) {
        self.conn.close(code.into(), reason);
    }

    /// Is the connection closed?
    pub fn is_closed(&self) -> bool {
        self.conn.close_reason().is_some()
    }

    /// The TLS exporter of this connection (for the pairing transcript, protocol 4.5).
    pub fn export(&self, label: &[u8], context: &[u8], out: &mut [u8]) -> bool {
        self.conn
            .export_keying_material(out, label, context)
            .is_ok()
    }
}

/// The engine's handle to a link.
#[derive(Clone, Debug)]
pub struct QuicPeer {
    peer: DeviceId,
    tx: mpsc::UnboundedSender<Vec<u8>>,
}

impl Peer for QuicPeer {
    fn id(&self) -> DeviceId {
        self.peer
    }

    fn send(&mut self, frame: Vec<u8>) -> Result<(), PeerGone> {
        self.tx.send(frame).map_err(|_| PeerGone)
    }
}

/// Error while opening the stream.
#[derive(Debug)]
pub struct OpenError(pub String);

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "link open failed: {}", self.0)
    }
}

impl std::error::Error for OpenError {}

/// Open the frame stream on an authenticated connection and start its reader and writer.
/// Events go to `events`.
pub async fn open(
    c: Conn,
    events: mpsc::UnboundedSender<LinkEvent>,
    timeout: Duration,
) -> Result<Link, OpenError> {
    let e = |x: &dyn std::fmt::Display| OpenError(x.to_string());
    let (mut send, mut recv) = tokio::time::timeout(timeout, async {
        if c.dialed {
            let (mut s, r) = c.conn.open_bi().await.map_err(|x| e(&x))?;
            s.write_all(&[START]).await.map_err(|x| e(&x))?;
            Ok::<_, OpenError>((s, r))
        } else {
            let (s, mut r) = c.conn.accept_bi().await.map_err(|x| e(&x))?;
            let mut b = [0u8; 1];
            r.read_exact(&mut b).await.map_err(|x| e(&x))?;
            if b[0] != START {
                return Err(OpenError("bad start byte".into()));
            }
            Ok((s, r))
        }
    })
    .await
    .map_err(|_| OpenError("timeout".into()))??;

    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let peer = c.peer;

    let wconn = c.conn.clone();
    tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            let len = (frame.len() as u32).to_be_bytes();
            if send.write_all(&len).await.is_err() || send.write_all(&frame).await.is_err() {
                break;
            }
        }
        let _ = send.finish();
        drop(wconn);
    });

    let rconn = c.conn.clone();
    tokio::spawn(async move {
        loop {
            let mut len = [0u8; 4];
            if recv.read_exact(&mut len).await.is_err() {
                break;
            }
            let n = u32::from_be_bytes(len) as usize;
            if n > msg::MAX_FRAME {
                rconn.close(4u32.into(), b"frame too large");
                break;
            }
            let mut frame = vec![0u8; n];
            if recv.read_exact(&mut frame).await.is_err() {
                break;
            }
            if events
                .send(LinkEvent::Frame {
                    peer,
                    link: id,
                    frame,
                })
                .is_err()
            {
                break;
            }
        }
        rconn.close(0u32.into(), b"");
        let _ = events.send(LinkEvent::Closed { peer, link: id });
    });

    Ok(Link {
        peer,
        id,
        remote: c.remote,
        iface: c.iface,
        dialed: c.dialed,
        conn: c.conn,
        tx,
    })
}
