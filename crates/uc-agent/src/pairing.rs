//! Pairing over QUIC (protocol 4.4 to 4.7): the stream I/O for both sides. The state machines
//! (`uc-pair`) stay pure; the inviter's run lives in the agent core (it needs the graph), the
//! joiner's run lives in its task.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot};
use uc_net::{Conn, NetHandle};
use uc_pair::frame::{AbortReason, Decoder, Frame};
use uc_pair::pake::{Mode, PairSecret};
use uc_pair::session::{
    InviterStep, JoinError, Joined, JoinerRun, JoinerStep, STEP_TIMEOUT_MS, TransportInfo,
};

use crate::ipc::IpcError;

/// Messages from pairing tasks to the core.
pub enum PairMsg {
    /// A frame on inviter run `run`. The core answers with the step.
    InviterFrame {
        /// Run ID.
        run: u64,
        /// The frame.
        frame: Frame,
        /// The step.
        reply: oneshot::Sender<InviterStep>,
    },
    /// Inviter run `run` ended (closed, timed out or finished).
    InviterClosed {
        /// Run ID.
        run: u64,
    },
    /// A joiner run got a good `PairState`. The core stores it and answers `true` if it did.
    Joined {
        /// What to store.
        joined: Box<Joined>,
        /// The inviter's key (from TLS).
        inviter: [u8; 32],
        /// The inviter's address.
        remote: SocketAddr,
        /// Stored?
        reply: oneshot::Sender<bool>,
    },
    /// The whole join ended.
    JoinFinished(Result<String, IpcError>),
}

/// TLS facts of a pairing connection (protocol 4.5: exporter, empty context, 32 bytes).
pub fn transport(c: &Conn) -> Option<TransportInfo> {
    let mut exporter = [0u8; 32];
    c.conn
        .export_keying_material(&mut exporter, uc_pair::EXPORTER_LABEL, b"")
        .ok()?;
    let scope = match c.remote {
        SocketAddr::V6(v6) if v6.scope_id() != 0 => Some(v6.scope_id()),
        _ => None,
    };
    Some(TransportInfo {
        peer_key: c.peer.0,
        exporter,
        remote: c.remote.ip(),
        scope,
    })
}

async fn next_frame(
    recv: &mut quinn::RecvStream,
    dec: &mut Decoder,
    step: Duration,
) -> Option<Frame> {
    let deadline = Instant::now() + step;
    let mut buf = vec![0u8; 8192];
    loop {
        match dec.next_frame() {
            Ok(Some(f)) => return Some(f),
            Ok(None) => {}
            Err(_) => return None,
        }
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, recv.read(&mut buf)).await {
            Ok(Ok(Some(n))) => dec.push(&buf[..n]),
            _ => return None,
        }
    }
}

/// Run the inviter side of one accepted pairing connection.
pub async fn inviter(conn: Conn, run: u64, tx: mpsc::UnboundedSender<PairMsg>) {
    let step = Duration::from_millis(STEP_TIMEOUT_MS);
    let streams = tokio::time::timeout(step, conn.conn.accept_bi()).await;
    let Ok(Ok((mut send, mut recv))) = streams else {
        let _ = tx.send(PairMsg::InviterClosed { run });
        conn.conn.close(0u32.into(), b"");
        return;
    };
    let mut dec = Decoder::new();
    loop {
        let Some(frame) = next_frame(&mut recv, &mut dec, step).await else {
            break;
        };
        let (rtx, rrx) = oneshot::channel();
        if tx
            .send(PairMsg::InviterFrame {
                run,
                frame,
                reply: rtx,
            })
            .is_err()
        {
            break;
        }
        let Ok(st) = rrx.await else { break };
        match st {
            InviterStep::Send(b) | InviterStep::SendState { frame: b, .. } => {
                if send.write_all(&b).await.is_err() {
                    break;
                }
            }
            InviterStep::Finished => break,
            InviterStep::Abort(b) => {
                if let Some(b) = b {
                    let _ = send.write_all(&b).await;
                }
                break;
            }
        }
    }
    let _ = send.finish();
    let _ = tokio::time::timeout(Duration::from_millis(500), send.stopped()).await;
    let _ = tx.send(PairMsg::InviterClosed { run });
    conn.conn.close(0u32.into(), b"");
}

/// One joiner attempt against one address. Returns the inviter's name on success.
async fn join_one(
    net: NetHandle,
    addr: SocketAddr,
    secret: PairSecret,
    own: [u8; 32],
    record: uc_pair::frame::DeviceRecord,
    tx: mpsc::UnboundedSender<PairMsg>,
) -> Result<(), JoinError> {
    let step = Duration::from_millis(STEP_TIMEOUT_MS);
    let conn = net
        .dial_pair_any(addr)
        .await
        .map_err(|_| JoinError::Aborted(AbortReason::Other))?;
    let t = transport(&conn).ok_or(JoinError::BadMessage)?;
    let (mut run, hello) =
        JoinerRun::start(&secret, Mode::Code, own, &t, None, record, rand_core::OsRng)?;
    let (mut send, mut recv) = tokio::time::timeout(step, conn.conn.open_bi())
        .await
        .map_err(|_| JoinError::Aborted(AbortReason::Other))?
        .map_err(|_| JoinError::Aborted(AbortReason::Other))?;
    send.write_all(&hello)
        .await
        .map_err(|_| JoinError::Aborted(AbortReason::Other))?;
    let mut dec = Decoder::new();
    let result = loop {
        let Some(f) = next_frame(&mut recv, &mut dec, step).await else {
            break Err(JoinError::CloseSilently);
        };
        match run.on_frame(f) {
            Ok(JoinerStep::Send(b)) => {
                if send.write_all(&b).await.is_err() {
                    break Err(JoinError::Aborted(AbortReason::Other));
                }
            }
            Ok(JoinerStep::Joined(j)) => {
                let done = j.done_frame.clone();
                let (rtx, rrx) = oneshot::channel();
                let _ = tx.send(PairMsg::Joined {
                    joined: j,
                    inviter: t.peer_key,
                    remote: conn.remote,
                    reply: rtx,
                });
                if rrx.await.unwrap_or(false) {
                    let _ = send.write_all(&done).await;
                    break Ok(());
                }
                break Err(JoinError::Aborted(AbortReason::Other));
            }
            Err(e) => {
                if e == JoinError::BadMessage {
                    let _ = send
                        .write_all(&uc_pair::frame::abort(AbortReason::BadMessage))
                        .await;
                }
                break Err(e);
            }
        }
    };
    let _ = send.finish();
    let _ = tokio::time::timeout(Duration::from_millis(500), send.stopped()).await;
    conn.conn.close(0u32.into(), b"");
    result
}

/// Where a joiner looks for the inviter.
pub trait Finder: Send + 'static {
    /// Addresses seen so far (called again and again for up to 20 s).
    fn poll(&mut self) -> Vec<SocketAddr>;
}

/// The joiner (protocol 4.4, "Code mode, finding the inviter"): run the flow with each
/// inviter found, at most 8 at the same time and 32 in total, for at most 20 seconds. The
/// first run with a correct `conf_a` wins.
pub async fn join(
    net: NetHandle,
    mut finder: Box<dyn Finder>,
    code: PairSecret,
    own: [u8; 32],
    record: uc_pair::frame::DeviceRecord,
    tx: mpsc::UnboundedSender<PairMsg>,
    limit: Duration,
) {
    let start = Instant::now();
    let mut tried: Vec<SocketAddr> = Vec::new();
    let mut set = tokio::task::JoinSet::new();
    let mut errors: Vec<JoinError> = Vec::new();
    let mut last_done: Option<Instant> = None;
    let result = loop {
        for a in finder.poll() {
            if tried.len() >= 32 || set.len() >= 8 {
                break;
            }
            if tried.contains(&a) {
                continue;
            }
            tried.push(a);
            set.spawn(join_one(
                net.clone(),
                a,
                code.clone(),
                own,
                record.clone(),
                tx.clone(),
            ));
        }
        tokio::select! {
            r = set.join_next(), if !set.is_empty() => {
                match r {
                    Some(Ok(Ok(()))) => break Ok(()),
                    Some(Ok(Err(e))) => {
                        errors.push(e);
                        last_done = Some(Instant::now());
                    }
                    _ => last_done = Some(Instant::now()),
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(200)) => {}
        }
        // Every inviter found so far answered and none was right: stop soon, so a wrong
        // code is reported at once (2 s more for other inviters to show up).
        if set.is_empty() && last_done.is_some_and(|t| t.elapsed() > Duration::from_secs(2)) {
            break Err(());
        }
        if start.elapsed() > limit && set.is_empty() {
            break Err(());
        }
        if start.elapsed() > limit + Duration::from_secs(STEP_TIMEOUT_MS / 1000 * 3) {
            break Err(());
        }
    };
    set.abort_all();
    let msg = match result {
        Ok(()) => return,
        Err(()) => join_error_text(tried.is_empty(), &errors),
    };
    let _ = tx.send(PairMsg::JoinFinished(Err(msg)));
}

/// The plain text for a failed join (plan 2.3).
pub fn join_error_text(found_none: bool, errors: &[JoinError]) -> IpcError {
    if found_none {
        return IpcError::new(
            "not-found",
            "Cannot find the other device. Check that both devices are on the same Wi-Fi or \
             network. If one of them uses a VPN, turn on Local network sharing in the VPN.",
        );
    }
    if errors.contains(&JoinError::Aborted(AbortReason::GroupFull)) {
        return IpcError::new(
            "group-full",
            "The other devices are already 8, the most a group can have.",
        );
    }
    if errors.contains(&JoinError::Aborted(AbortReason::NotOpen)) {
        return IpcError::new(
            "code-dead",
            "That code no longer works. Make a new code on the other device.",
        );
    }
    if errors.contains(&JoinError::CloseSilently) {
        return IpcError::new(
            "wrong-code",
            "That code is not right. Check it and try again.",
        );
    }
    IpcError::new(
        "pair-failed",
        "Pairing did not work. Make a new code on the other device and try again.",
    )
}

/// Format a 6-digit code as `418 207`.
pub fn show_code(s: &PairSecret) -> Option<String> {
    match s {
        PairSecret::Code(c) => {
            let t = String::from_utf8_lossy(c);
            Some(format!("{} {}", &t[..3], &t[3..]))
        }
        PairSecret::Qr(_) => None,
    }
}

/// Parse what a user typed: 6 digits, spaces and dashes ignored.
pub fn parse_code(s: &str) -> Option<PairSecret> {
    let d: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    PairSecret::code(&d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes() {
        let c = parse_code("418 207").unwrap();
        assert_eq!(show_code(&c).as_deref(), Some("418 207"));
        assert!(parse_code("418-207").is_some());
        assert!(parse_code("41820").is_none());
        assert!(parse_code("41820x").is_none());
    }

    #[test]
    fn error_texts_follow_the_plan() {
        assert_eq!(join_error_text(true, &[]).code, "not-found");
        assert_eq!(
            join_error_text(false, &[JoinError::CloseSilently]).message,
            "That code is not right. Check it and try again."
        );
        assert_eq!(
            join_error_text(false, &[JoinError::Aborted(AbortReason::NotOpen)]).code,
            "code-dead"
        );
    }
}
