//! A sync link: one QUIC connection with one bidirectional stream of frames, the
//! [`uc_core::Peer`] implementation over it, and unidirectional data streams for file
//! transfers (file sync plan, sections 3 and 6).
//!
//! Framing: `u32 big-endian length || frame`, frames as `uc_core::msg` encodes them. The dialer
//! opens the stream and writes one start byte, so the acceptor sees the stream at once. Frame
//! order is kept (one stream). A frame larger than [`uc_core::msg::MAX_FRAME`] closes the link.
//!
//! Data streams: [`Link::open_stream`] opens one unidirectional stream per transfer at
//! priority [`DATA_PRIORITY`], so buffered clipboard frames go out first. The peer can open
//! none until [`Link::allow_streams`] grants it credit; the node does that only for the
//! current link of a member, never for a probation link or a pairing connection. Every link
//! runs an `accept_uni` loop that hands incoming streams to the node as
//! [`LinkEvent::Stream`]. A transfer is cancelled with a stream code ([`STREAM_SUPERSEDED`],
//! [`STREAM_ABORTED`]), never by closing the link, which would also end the clipboard frames.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::sync::mpsc;
use uc_core::sync::{Peer, PeerGone};
use uc_core::{DeviceId, msg};

use crate::endpoint::Conn;

const START: u8 = 0x55;

/// Stream code: the transfer was cancelled because a newer copy exists (plan section 3).
pub const STREAM_SUPERSEDED: u32 = 1;
/// Stream code: the transfer was aborted (an error, Stop, or a stream nobody asked for).
pub const STREAM_ABORTED: u32 = 2;

/// Data streams the peer may have open at once on a member link (plan section 6).
pub const MAX_DATA_STREAMS: u8 = 8;

/// Priority of data streams. The frame stream has 0, so its buffered data is sent first.
pub const DATA_PRIORITY: i32 = -1;

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
    /// The peer opened a data stream. The node passes it on only for the current link of a
    /// member and stops it otherwise.
    Stream {
        /// From.
        peer: DeviceId,
        /// Which link.
        link: u64,
        /// The stream.
        stream: RecvStream,
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
    open_timeout: Duration,
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

    /// Let the peer open up to [`MAX_DATA_STREAMS`] data streams on this link. Until this is
    /// called the limit is 0 (see `endpoint::transport`), so the peer cannot open a single
    /// one. The node calls it once the link is the current link of a member.
    pub fn allow_streams(&self) {
        self.conn
            .set_max_concurrent_uni_streams(MAX_DATA_STREAMS.into());
    }

    /// Open a data stream to the peer at priority [`DATA_PRIORITY`]. Waits for stream credit
    /// for at most the handshake timeout: a peer that grants none (it has this device on
    /// probation, or all its streams are in use) gives [`StreamError::Refused`].
    pub async fn open_stream(&self) -> Result<SendStream, StreamError> {
        let s = tokio::time::timeout(self.open_timeout, self.conn.open_uni())
            .await
            .map_err(|_| StreamError::Refused)?
            .map_err(|_| StreamError::LinkClosed)?;
        // Fails only on a stream that is already closed; the first write reports that.
        let _ = s.set_priority(DATA_PRIORITY);
        Ok(SendStream {
            inner: s,
            done: false,
        })
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

/// Why a data stream operation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamError {
    /// The receiver stopped the stream with this code.
    Stopped(u32),
    /// The sender reset the stream with this code.
    Reset(u32),
    /// The stream ended before all expected bytes arrived.
    FinishedEarly,
    /// The peer granted no stream credit in time.
    Refused,
    /// The link is closed (either side, or the path died).
    LinkClosed,
    /// This side already finished, reset or stopped the stream.
    Closed,
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StreamError::Stopped(c) => write!(f, "stream stopped by the receiver (code {c})"),
            StreamError::Reset(c) => write!(f, "stream reset by the sender (code {c})"),
            StreamError::FinishedEarly => f.write_str("stream ended early"),
            StreamError::Refused => f.write_str("the peer accepts no stream"),
            StreamError::LinkClosed => f.write_str("link closed"),
            StreamError::Closed => f.write_str("stream already closed"),
        }
    }
}

impl std::error::Error for StreamError {}

/// A peer's stream code as `u32` (ours all fit; anything larger becomes `u32::MAX`).
fn code(v: quinn::VarInt) -> u32 {
    u32::try_from(v.into_inner()).unwrap_or(u32::MAX)
}

impl From<quinn::WriteError> for StreamError {
    fn from(e: quinn::WriteError) -> Self {
        match e {
            quinn::WriteError::Stopped(c) => StreamError::Stopped(code(c)),
            quinn::WriteError::ClosedStream => StreamError::Closed,
            _ => StreamError::LinkClosed,
        }
    }
}

impl From<quinn::ReadError> for StreamError {
    fn from(e: quinn::ReadError) -> Self {
        match e {
            quinn::ReadError::Reset(c) => StreamError::Reset(code(c)),
            quinn::ReadError::ClosedStream => StreamError::Closed,
            _ => StreamError::LinkClosed,
        }
    }
}

/// The sending side of a data stream. Writes wait for QUIC flow control, so a slow receiver
/// slows the sender down and nothing piles up in memory.
///
/// Dropping it without [`SendStream::finish`] resets the stream with [`STREAM_ABORTED`], so an
/// aborted task never looks like a complete transfer to the receiver. (A bare quinn stream
/// would finish cleanly on drop.)
#[derive(Debug)]
pub struct SendStream {
    inner: quinn::SendStream,
    done: bool,
}

impl SendStream {
    /// Write all of `buf`, waiting while the peer's receive window is full.
    pub async fn write(&mut self, buf: &[u8]) -> Result<(), StreamError> {
        if self.done {
            return Err(StreamError::Closed);
        }
        self.inner.write_all(buf).await.map_err(StreamError::from)
    }

    /// End the stream cleanly and wait until the peer has received every byte. If the peer
    /// stopped the stream first: [`StreamError::Stopped`].
    pub async fn finish(mut self) -> Result<(), StreamError> {
        self.done = true;
        self.inner.finish().map_err(|_| StreamError::Closed)?;
        match self.inner.stopped().await {
            Ok(None) => Ok(()),
            Ok(Some(c)) => Err(StreamError::Stopped(code(c))),
            Err(_) => Err(StreamError::LinkClosed),
        }
    }

    /// Cancel the transfer: the receiver's next read gives [`StreamError::Reset`] with `code`.
    pub fn reset(mut self, code: u32) {
        self.done = true;
        let _ = self.inner.reset(code.into());
    }

    /// Wait until the receiver stops the stream and give its code. `None` if the stream ended
    /// another way (finished and received, or the link closed).
    pub async fn stopped(&self) -> Option<u32> {
        self.inner.stopped().await.ok().flatten().map(code)
    }
}

impl Drop for SendStream {
    fn drop(&mut self) {
        if !self.done {
            let _ = self.inner.reset(STREAM_ABORTED.into());
        }
    }
}

/// The receiving side of a data stream.
///
/// Dropping it before its end stops the stream with [`STREAM_ABORTED`]. So read until
/// [`RecvStream::read`] gives `Ok(None)`: only then does the sender's
/// [`SendStream::finish`] report success for sure.
#[derive(Debug)]
pub struct RecvStream {
    inner: quinn::RecvStream,
    done: bool,
}

impl RecvStream {
    /// Read some bytes into `buf`. `Ok(None)` at the clean end of the stream.
    pub async fn read(&mut self, buf: &mut [u8]) -> Result<Option<usize>, StreamError> {
        if self.done {
            return Err(StreamError::Closed);
        }
        let r = self.inner.read(buf).await.map_err(StreamError::from)?;
        if r.is_none() {
            self.done = true;
        }
        Ok(r)
    }

    /// Fill `buf` completely. A clean end before that is [`StreamError::FinishedEarly`].
    pub async fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), StreamError> {
        if self.done {
            return Err(StreamError::Closed);
        }
        self.inner.read_exact(buf).await.map_err(|e| match e {
            quinn::ReadExactError::FinishedEarly(_) => StreamError::FinishedEarly,
            quinn::ReadExactError::ReadError(e) => StreamError::from(e),
        })
    }

    /// Refuse the rest of the stream: the sender's next write gives [`StreamError::Stopped`]
    /// with `code`.
    pub fn stop(mut self, code: u32) {
        self.done = true;
        let _ = self.inner.stop(code.into());
    }
}

impl Drop for RecvStream {
    fn drop(&mut self) {
        if !self.done {
            let _ = self.inner.stop(STREAM_ABORTED.into());
        }
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

/// Open the frame stream on an authenticated connection and start its reader, its writer and
/// its data stream acceptor. Events go to `events`. `timeout` also limits each wait for stream
/// credit in [`Link::open_stream`].
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

    let aconn = c.conn.clone();
    let aevents = events.clone();
    tokio::spawn(async move {
        // Ends when the connection closes. Credit stays 0 until `allow_streams`, so on a link
        // that never gets it this only waits.
        while let Ok(s) = aconn.accept_uni().await {
            let stream = RecvStream {
                inner: s,
                done: false,
            };
            if aevents
                .send(LinkEvent::Stream {
                    peer,
                    link: id,
                    stream,
                })
                .is_err()
            {
                break;
            }
        }
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
        open_timeout: timeout,
    })
}
