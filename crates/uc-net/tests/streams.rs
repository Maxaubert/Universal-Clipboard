//! Data streams on loopback (file sync plan, sections 3, 6 and 7): a 1 GiB transfer, cancel
//! from each side, no streams on probation and pairing connections, and a link replaced in
//! the middle of a transfer. Real QUIC on 127.0.0.x; nothing touches a clipboard.

mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use common::*;
use tokio::sync::mpsc;
use uc_net::endpoint::LanEndpoint;
use uc_net::link::{self, LinkEvent};
use uc_net::{
    ALPN_SYNC, AddressBinder, FakeLan, Link, Policy, RecvStream, STREAM_ABORTED, STREAM_SUPERSEDED,
    StreamError,
};

const T: Duration = Duration::from_secs(10);
const CHUNK: usize = 1 << 20;

fn addr(n: &Node) -> SocketAddr {
    let rt = tokio::runtime::Handle::current();
    let port = tokio::task::block_in_place(|| rt.block_on(n.net.status()))
        .map(|s| s.port)
        .unwrap_or(0);
    SocketAddr::new(n.ips()[0], port)
}

/// Two member nodes, connected.
async fn pair() -> (FakeLan, Node, Node) {
    let lan = FakeLan::new();
    let mut a = Node::start(1, &ip(1), 0, &lan);
    let mut b = Node::start(2, &ip(2), 0, &lan);
    a.join(&[1, 2]);
    b.join(&[1, 2]);
    until(&mut [&mut a, &mut b], T, |n| {
        n[0].links.contains_key(&id(2)) && n[1].links.contains_key(&id(1))
    })
    .await
    .expect("connected");
    (lan, a, b)
}

/// Pump until `n` has a stream from `peer`, and take it.
async fn take_stream(nodes: &mut [&mut Node], at: usize, peer: u8) -> (u64, RecvStream) {
    until(nodes, T, |n| n[at].streams.iter().any(|s| s.0 == id(peer)))
        .await
        .expect("stream delivered");
    let i = nodes[at]
        .streams
        .iter()
        .position(|s| s.0 == id(peer))
        .unwrap();
    let (_, link, s) = nodes[at].streams.remove(i);
    (link, s)
}

/// Chunk `i` of the test pattern: the index at both ends, a fixed byte in between.
fn fill(buf: &mut [u8], i: u64) {
    buf[..8].copy_from_slice(&i.to_le_bytes());
    let n = buf.len();
    buf[n - 8..].copy_from_slice(&i.to_le_bytes());
}

fn check(buf: &[u8], i: u64) {
    let n = buf.len();
    assert_eq!(&buf[..8], &i.to_le_bytes(), "chunk {i} head");
    assert_eq!(&buf[n - 8..], &i.to_le_bytes(), "chunk {i} tail");
    assert_eq!(buf[n / 2], 0xA5, "chunk {i} body");
}

/// Send `chunks` MiB of the pattern on a new stream and finish it.
async fn send_pattern(link: Link, chunks: u64) -> Result<(), StreamError> {
    let mut s = link.open_stream().await?;
    let mut buf = vec![0xA5u8; CHUNK];
    for i in 0..chunks {
        fill(&mut buf, i);
        s.write(&buf).await?;
    }
    s.finish().await
}

/// Read `chunks` MiB of the pattern and then the clean end. `done` counts chunks.
async fn recv_pattern(
    mut s: RecvStream,
    chunks: u64,
    done: Arc<AtomicU64>,
) -> Result<(), StreamError> {
    let mut buf = vec![0u8; CHUNK];
    for i in 0..chunks {
        s.read_exact(&mut buf).await?;
        check(&buf, i);
        done.store(i + 1, Ordering::Relaxed);
    }
    let mut one = [0u8; 1];
    assert_eq!(
        s.read(&mut one).await?,
        None,
        "nothing after the declared size"
    );
    Ok(())
}

/// The plan's 1 GB transfer (1 GiB here) on one data stream, with throughput logged. A
/// clipboard frame sent halfway still arrives quickly (priority -1 for data).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_gigabyte_crosses_a_data_stream() {
    const CHUNKS: u64 = 1024;
    let (_lan, mut a, mut b) = pair().await;
    let link = a.links[&id(2)].clone();
    let start = Instant::now();
    let sender = tokio::spawn(send_pattern(link.clone(), CHUNKS));
    let (lid, stream) = take_stream(&mut [&mut a, &mut b], 1, 1).await;
    assert_eq!(lid, b.links[&id(1)].id, "delivered for the current link");
    let done = Arc::new(AtomicU64::new(0));
    let reader = tokio::spawn(recv_pattern(stream, CHUNKS, done.clone()));

    until(&mut [&mut a, &mut b], Duration::from_secs(120), |_| {
        done.load(Ordering::Relaxed) >= CHUNKS / 2
    })
    .await
    .expect("half the transfer");
    let sent = Instant::now();
    link.send(b"ping".to_vec()).unwrap();
    until(&mut [&mut a, &mut b], T, |n| {
        n[1].frames.iter().any(|(_, f)| f == b"ping")
    })
    .await
    .expect("frame during the transfer");
    let frame_latency = sent.elapsed();

    tokio::time::timeout(Duration::from_secs(120), reader)
        .await
        .expect("transfer in time")
        .unwrap()
        .expect("received");
    sender.await.unwrap().expect("sent");
    let took = start.elapsed();
    let mib_s = CHUNKS as f64 / took.as_secs_f64();
    eprintln!(
        "1 GiB over loopback QUIC: {took:.2?}, {mib_s:.0} MiB/s; frame latency mid-transfer {frame_latency:?}"
    );
    assert!(
        frame_latency < Duration::from_secs(2),
        "a clipboard frame waited {frame_latency:?} behind file data"
    );
    assert!(b.links.contains_key(&id(1)), "the link stays up");
}

/// Sender side cancel: `reset(code)` reaches the reader as `Reset(code)`, a dropped stream as
/// `Reset(STREAM_ABORTED)`, and the link keeps carrying frames and new streams.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sender_cancel_resets_the_stream_and_keeps_the_link() {
    let (_lan, mut a, mut b) = pair().await;
    let link = a.links[&id(2)].clone();

    let mut s = link.open_stream().await.unwrap();
    s.write(&[7u8; 1000]).await.unwrap();
    let (_, mut r) = take_stream(&mut [&mut a, &mut b], 1, 1).await;
    let mut buf = [0u8; 1000];
    r.read_exact(&mut buf).await.unwrap();
    s.reset(STREAM_SUPERSEDED);
    let e = loop {
        match r.read(&mut buf).await {
            Ok(Some(_)) => continue,
            other => break other,
        }
    };
    assert_eq!(e, Err(StreamError::Reset(STREAM_SUPERSEDED)));

    // A send task that dies (dropped stream) is an abort, never a clean end.
    let mut s = link.open_stream().await.unwrap();
    s.write(b"x").await.unwrap();
    let (_, mut r) = take_stream(&mut [&mut a, &mut b], 1, 1).await;
    drop(s);
    let e = loop {
        match r.read(&mut buf).await {
            Ok(Some(_)) => continue,
            other => break other,
        }
    };
    assert_eq!(e, Err(StreamError::Reset(STREAM_ABORTED)));

    link.send(b"after".to_vec()).unwrap();
    until(&mut [&mut a, &mut b], T, |n| {
        n[1].frames.iter().any(|(_, f)| f == b"after")
    })
    .await
    .expect("frames still flow");
    let sender = tokio::spawn(send_pattern(link, 4));
    let (_, r) = take_stream(&mut [&mut a, &mut b], 1, 1).await;
    recv_pattern(r, 4, Arc::default()).await.unwrap();
    sender.await.unwrap().expect("a new stream works");
}

/// Receiver side cancel: `stop(code)` reaches the writer as `Stopped(code)` (the receiver's
/// answer to an unsolicited stream or a Stop), and the link stays.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn receiver_stop_cancels_the_sender_and_keeps_the_link() {
    let (_lan, mut a, mut b) = pair().await;
    let link = a.links[&id(2)].clone();
    let sender = tokio::spawn(send_pattern(link.clone(), 1024));
    let (_, mut r) = take_stream(&mut [&mut a, &mut b], 1, 1).await;
    let mut buf = vec![0u8; CHUNK];
    r.read_exact(&mut buf).await.unwrap();
    r.stop(STREAM_ABORTED);
    let e = tokio::time::timeout(T, sender)
        .await
        .expect("the sender notices")
        .unwrap();
    assert_eq!(e, Err(StreamError::Stopped(STREAM_ABORTED)));

    // `stopped` also tells a sender that is not writing at the moment.
    let mut s = link.open_stream().await.unwrap();
    s.write(b"x").await.unwrap();
    let (_, r) = take_stream(&mut [&mut a, &mut b], 1, 1).await;
    r.stop(STREAM_SUPERSEDED);
    let code = tokio::time::timeout(T, s.stopped()).await.expect("stopped");
    assert_eq!(code, Some(STREAM_SUPERSEDED));

    link.send(b"after".to_vec()).unwrap();
    until(&mut [&mut a, &mut b], T, |n| {
        n[1].frames.iter().any(|(_, f)| f == b"after")
    })
    .await
    .expect("frames still flow");
}

/// A raw endpoint for device `n`, outside any [`uc_net::Net`]: the test drives its links.
fn raw(n: u8) -> (LanEndpoint, mpsc::Receiver<uc_net::Conn>) {
    let p = Policy {
        allow_loopback: true,
    };
    LanEndpoint::bind(
        key(n),
        &[lo(&ip(n), 8)],
        0,
        p,
        timing(),
        &AddressBinder,
        &[ALPN_SYNC.to_vec()],
    )
    .unwrap()
}

/// Probation links and pairing connections get no stream credit: opening one is refused
/// and the node never passes a stream on. Membership turns the same link into one that does.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn probation_and_pairing_connections_carry_no_streams() {
    let lan = FakeLan::new();
    let mut b = Node::start(2, &ip(2), 0, &lan);
    b.join(&[2]); // B knows nobody: X gets a probation link.
    let target = addr(&b);
    let (x, _incoming) = raw(9);
    let conn = x.dial(id(2), target, ALPN_SYNC).await.expect("dial");
    let (tx, _events) = mpsc::unbounded_channel();
    let xl = link::open(conn, tx, timing().handshake_timeout)
        .await
        .expect("link");
    until(&mut [&mut b], T, |n| n[0].links.contains_key(&id(9)))
        .await
        .expect("probation link");
    assert_eq!(
        xl.open_stream().await.err(),
        Some(StreamError::Refused),
        "a probation peer cannot open a data stream"
    );
    settle(&mut [&mut b], Duration::from_millis(300)).await;
    assert!(b.streams.is_empty());

    // Pairing connections: no credit either, even for a raw QUIC stream.
    let a = Node::start(1, &ip(1), 0, &lan);
    let pair_conn = a.net.dial_pair(id(2), target).await.expect("pair dial");
    let r = tokio::time::timeout(Duration::from_millis(800), pair_conn.conn.open_uni()).await;
    assert!(r.is_err(), "a pairing connection never gets stream credit");
    settle(&mut [&mut b], Duration::from_millis(100)).await;
    assert_eq!(b.pairing.len(), 1);

    // Membership sync made X a member: the same link now takes streams.
    b.join(&[2, 9]);
    settle(&mut [&mut b], Duration::from_millis(200)).await;
    let mut s = xl.open_stream().await.expect("a member may open streams");
    s.write(b"hello").await.unwrap();
    let (lid, mut r) = take_stream(&mut [&mut b], 0, 9).await;
    assert_eq!(lid, b.links[&id(9)].id);
    let fin = tokio::spawn(s.finish());
    let mut buf = [0u8; 5];
    r.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"hello");
    assert_eq!(r.read(&mut buf).await, Ok(None));
    fin.await.unwrap().expect("finished and received");
}

/// A link replaced in the middle of a transfer ends the transfer on both sides (never a
/// silent short file), a stream can no longer be opened on the old link, and the retry on the
/// new link is delivered with the new link's id and completes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replaced_link_ends_its_transfer_and_the_retry_works() {
    let lan = FakeLan::new();
    let mut b = Node::start(2, &ip(2), 0, &lan);
    b.join(&[2, 9]);
    let target = addr(&b);
    let (x, _incoming) = raw(9);
    let (tx, _events) = mpsc::unbounded_channel::<LinkEvent>();
    let open = |tx| {
        let x = &x;
        async move {
            let c = x.dial(id(2), target, ALPN_SYNC).await.expect("dial");
            link::open(c, tx, timing().handshake_timeout)
                .await
                .expect("link")
        }
    };
    let l1 = open(tx.clone()).await;
    until(&mut [&mut b], T, |n| n[0].links.contains_key(&id(9)))
        .await
        .expect("link 1");
    let sender = tokio::spawn(send_pattern(l1.clone(), 1024));
    let (lid1, r1) = take_stream(&mut [&mut b], 0, 9).await;
    let done = Arc::new(AtomicU64::new(0));
    let reader = tokio::spawn(recv_pattern(r1, 1024, done.clone()));
    until(&mut [&mut b], T, |_| done.load(Ordering::Relaxed) >= 4)
        .await
        .expect("transfer running");

    // A new connection from X (after a network change, say) replaces link 1.
    let l2 = open(tx.clone()).await;
    until(&mut [&mut b], T, |n| {
        n[0].links.get(&id(9)).is_some_and(|l| l.id != lid1)
    })
    .await
    .expect("link 2 is current");
    let e = tokio::time::timeout(T, sender)
        .await
        .expect("sender ends")
        .unwrap();
    assert_eq!(e, Err(StreamError::LinkClosed));
    let e = tokio::time::timeout(T, reader)
        .await
        .expect("reader ends")
        .unwrap();
    assert_eq!(e, Err(StreamError::LinkClosed));
    assert!(
        done.load(Ordering::Relaxed) < 1024,
        "never a complete transfer"
    );
    assert_eq!(l1.open_stream().await.err(), Some(StreamError::LinkClosed));

    // The receiver asks again: the retry runs on link 2.
    let sender = tokio::spawn(send_pattern(l2.clone(), 8));
    let (lid2, r2) = take_stream(&mut [&mut b], 0, 9).await;
    assert_eq!(lid2, b.links[&id(9)].id);
    assert_ne!(lid2, lid1);
    recv_pattern(r2, 8, Arc::default()).await.unwrap();
    sender.await.unwrap().expect("retry sent");
}
