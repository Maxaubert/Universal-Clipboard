//! The pairing flow (protocol 4.2 to 4.7), without I/O.
//!
//! The transport is abstract: [`PairTransport`] gives the peer's TLS key, the TLS exporter value
//! and the remote address of one connection. M2 wires the real QUIC connection. The caller moves
//! frames between the stream and these state machines and passes the time in.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, MutexGuard};

use rand_core::{CryptoRng, RngCore};
use uc_keys::DeviceKey;
use zeroize::Zeroizing;

use crate::chain::{AddError, Change, Graph, GroupSecret, Key, MAX_MEMBERS};
use crate::frame::{
    self, AbortReason, Confirm, DeviceRecord, Frame, Hello, Reply, State, T_ABORT, T_CONFIRM,
    T_DONE, T_HELLO, T_REPLY, T_STATE,
};
use crate::lan::{Iface, is_lan_peer};
use crate::pake::{
    Confirmation, Mode, PairSecret, Pake, TranscriptInput, VERSION, confirmation, verify_mac,
};

/// Lifetime of one open pairing.
pub const PAIRING_LIFETIME_MS: u64 = 10 * 60 * 1000;
/// Time allowed for each protocol step. The driver enforces it and then drops the run.
pub const STEP_TIMEOUT_MS: u64 = 10 * 1000;
/// Tries per 6-digit code.
pub const CODE_TRIES: u32 = 5;
/// Tries per QR secret, per remote address (4.6).
pub const QR_TRIES: u32 = 20;
/// Remote addresses whose QR tries are tracked per pairing. Bounds memory (4.6).
pub const QR_SOURCES_MAX: usize = 1024;
/// Pairing connections handled at the same time.
pub const MAX_CONNECTIONS: u32 = 4;
/// Failed code tries in a row, over all pairings, before the short code is turned off (4.6).
pub const LOCKOUT_FAILED_TRIES: u32 = 15;
/// How long the short code stays off.
pub const LOCKOUT_MS: u64 = 60 * 60 * 1000;

/// What the pairing layer needs from one connection.
pub trait PairTransport {
    /// The peer's Ed25519 device key, read from the TLS layer (never from a message).
    fn peer_key(&self) -> Key;
    /// TLS exporter, label [`crate::EXPORTER_LABEL`], empty context, 32 bytes.
    fn exporter(&self) -> [u8; 32];
    /// The remote IP address, and the IPv6 scope ID of a link-local address.
    fn remote(&self) -> (IpAddr, Option<u32>);
}

/// The fixed values of a transport. Handy for tests and for drivers that read them once.
#[derive(Debug, Clone)]
pub struct TransportInfo {
    pub peer_key: Key,
    pub exporter: [u8; 32],
    pub remote: IpAddr,
    pub scope: Option<u32>,
}

impl PairTransport for TransportInfo {
    fn peer_key(&self) -> Key {
        self.peer_key
    }
    fn exporter(&self) -> [u8; 32] {
        self.exporter
    }
    fn remote(&self) -> (IpAddr, Option<u32>) {
        (self.remote, self.scope)
    }
}

/// Text of the notice after a failed try (4.6).
pub const NOTICE_FAILED_TRY: &str = "Someone else tried this code";
/// Text shown when the short code is off (4.6).
pub const NOTICE_LOCKOUT: &str =
    "Someone on this network keeps guessing the code. Use the QR code, or try again in an hour.";

/// What the inviter shows after it opened a pairing.
pub struct Opened {
    /// The 6-digit code, or `None` when the short code is off.
    pub code: Option<PairSecret>,
    pub qr_secret: [u8; 16],
    /// mDNS instance name: 16 lowercase hex characters.
    pub instance: String,
    pub expires_ms: u64,
}

impl Drop for Opened {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.qr_secret);
    }
}

/// What the UI shows about the open pairing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub open: bool,
    pub succeeded: bool,
    pub tries_code: u32,
    /// QR tries of this pairing, all addresses together.
    pub tries_qr: u32,
    /// Failed code tries so far. The notice [`NOTICE_FAILED_TRY`] shows after each.
    pub failed_code_tries: u32,
    pub code_dead: bool,
    /// The short code is off for this device (lockout).
    pub code_off: bool,
    /// Failed code tries over all pairings since the last success or lockout (4.6).
    pub failed_code_tries_in_a_row: u32,
    pub active_connections: u32,
    /// The open pairing still holds the code or the QR secret in memory.
    pub holds_secrets: bool,
}

/// Why a connection was refused before any frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refuse {
    /// No pairing is open: the ALPN is not accepted at all.
    NotOpen,
    /// The remote address fails the LAN rule: close before `PairHello` is read.
    NotLocal,
    /// More than 4 at once: send this `PairAbort(3)` and close. Not a try.
    Busy,
}

struct Open {
    code: Option<PairSecret>,
    /// `None` once the pairing succeeded: the secret is wiped (4.2 step 5).
    qr: Option<PairSecret>,
    expires_ms: u64,
    tries_code: u32,
    tries_qr: u32,
    /// QR tries per remote address (4.6).
    qr_by_source: HashMap<IpAddr, u32>,
    failed_code: u32,
    succeeded: bool,
    cancelled: bool,
    /// Identifies this pairing, so a run of an old pairing cannot touch a new one.
    serial: u64,
}

impl Open {
    fn is_live(&self, now_ms: u64) -> bool {
        !self.succeeded && !self.cancelled && now_ms < self.expires_ms
    }
}

#[derive(Default)]
struct HostState {
    open: Option<Open>,
    serial: u64,
    active: u32,
    /// Failed code tries over all pairings since the last success or lockout (4.6).
    failed_in_a_row: u32,
    code_off_until_ms: Option<u64>,
}

impl HostState {
    fn code_off(&self, now_ms: u64) -> bool {
        self.code_off_until_ms.is_some_and(|t| now_ms < t)
    }

    /// Records the end of a counted run that did not end with a correct `conf_b`.
    fn record_failure(&mut self, serial: u64, mode: Mode, now_ms: u64) {
        if mode != Mode::Code {
            return;
        }
        // The lockout counts every failed code try, also of a pairing that was cancelled or
        // replaced since, so neither stopping at 4 tries per code nor reopening escapes it (4.6).
        self.failed_in_a_row += 1;
        if self.failed_in_a_row >= LOCKOUT_FAILED_TRIES {
            self.code_off_until_ms = Some(now_ms + LOCKOUT_MS);
            self.failed_in_a_row = 0;
        }
        let Some(o) = self.open.as_mut().filter(|o| o.serial == serial) else {
            return;
        };
        if o.code.is_none() {
            return;
        }
        o.failed_code += 1;
        if o.failed_code == CODE_TRIES {
            // The code is dead (4.6). The QR code stays valid.
            o.code = None;
        }
    }
}

/// The inviter's pairing host. One per device for the whole agent run. The lockout count lives
/// here, in memory only (4.6). Cheap to clone; all clones share one state.
#[derive(Clone, Default)]
pub struct Host {
    inner: Arc<Mutex<HostState>>,
}

impl Host {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, HostState> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Opens a pairing (4.2). Any open pairing is closed first (at most 1 per device). Refused
    /// when the group has 8 members.
    pub fn open<R: RngCore + CryptoRng>(
        &self,
        graph: &Graph,
        now_ms: u64,
        rng: &mut R,
    ) -> Result<Opened, AbortReason> {
        if graph.members().len() >= MAX_MEMBERS {
            return Err(AbortReason::GroupFull);
        }
        let mut s = self.lock();
        let code_off = s.code_off(now_ms);
        let code = if code_off {
            None
        } else {
            Some(crate::pake::random_code(rng))
        };
        let mut qr_secret = [0u8; 16];
        rng.fill_bytes(&mut qr_secret);
        let mut tag = [0u8; 8];
        rng.fill_bytes(&mut tag);
        let instance = tag.iter().map(|b| format!("{b:02x}")).collect();
        s.serial += 1;
        let serial = s.serial;
        let expires_ms = now_ms + PAIRING_LIFETIME_MS;
        s.open = Some(Open {
            code: code.clone(),
            qr: Some(PairSecret::Qr(qr_secret)),
            expires_ms,
            tries_code: 0,
            tries_qr: 0,
            qr_by_source: HashMap::new(),
            failed_code: 0,
            succeeded: false,
            cancelled: false,
            serial,
        });
        Ok(Opened {
            code,
            qr_secret,
            instance,
            expires_ms,
        })
    }

    /// The user cancelled. Secrets are wiped.
    pub fn cancel(&self) {
        let mut s = self.lock();
        if let Some(o) = s.open.as_mut() {
            o.cancelled = true;
        }
        s.open = None;
    }

    /// True while the ALPN must be accepted and the mDNS service advertised.
    pub fn is_open(&self, now_ms: u64) -> bool {
        let mut s = self.lock();
        let live = s.open.as_ref().is_some_and(|o| o.is_live(now_ms));
        if !live && s.open.as_ref().is_some_and(|o| !o.succeeded) {
            // Closed by time: wipe the secrets. A succeeded pairing is kept for its status.
            s.open = None;
        }
        live
    }

    pub fn status(&self, now_ms: u64) -> Status {
        let s = self.lock();
        let mut st = Status {
            code_off: s.code_off(now_ms),
            failed_code_tries_in_a_row: s.failed_in_a_row,
            active_connections: s.active,
            ..Status::default()
        };
        if let Some(o) = &s.open {
            st.open = o.is_live(now_ms);
            st.succeeded = o.succeeded;
            st.tries_code = o.tries_code;
            st.tries_qr = o.tries_qr;
            st.failed_code_tries = o.failed_code;
            st.code_dead = o.failed_code >= CODE_TRIES;
            st.holds_secrets = o.code.is_some() || o.qr.is_some();
        }
        st
    }

    /// A new pairing connection. Checks that a pairing is open, the LAN rule and the connection
    /// limit, in that order.
    pub fn accept(
        &self,
        t: &dyn PairTransport,
        ifaces: &[Iface],
        own_key: Key,
        now_ms: u64,
    ) -> Result<InviterRun, Refuse> {
        let mut s = self.lock();
        let Some(o) = s.open.as_ref().filter(|o| o.is_live(now_ms)) else {
            return Err(Refuse::NotOpen);
        };
        let serial = o.serial;
        let (ip, scope) = t.remote();
        if !is_lan_peer(ip, scope, ifaces) {
            return Err(Refuse::NotLocal);
        }
        if s.active >= MAX_CONNECTIONS {
            return Err(Refuse::Busy);
        }
        s.active += 1;
        Ok(InviterRun {
            host: self.clone(),
            serial,
            own_key,
            peer_key: t.peer_key(),
            exporter: t.exporter(),
            remote: ip,
            state: InvState::WaitHello,
            counted: None,
            conf: None,
            last_ms: now_ms,
        })
    }
}

#[derive(PartialEq, Eq)]
enum InvState {
    WaitHello,
    WaitConfirm,
    WaitDone,
    Done,
    Failed,
}

/// What the inviter sends after a frame.
#[derive(Debug)]
pub enum InviterStep {
    /// Send these bytes and wait for the next frame.
    Send(Vec<u8>),
    /// Send `PairState`; `added` is the new `add` change to spread to members (`None` when the
    /// joiner was already a member).
    SendState {
        frame: Vec<u8>,
        added: Option<Box<Change>>,
    },
    /// The joiner stored the state. Close the connection; the pairing is over.
    Finished,
    /// Send this `PairAbort` (if any) and close.
    Abort(Option<Vec<u8>>),
}

/// What the inviter needs to answer a correct `PairConfirm`.
pub struct GroupCtx<'a> {
    pub graph: &'a mut Graph,
    pub secret: &'a GroupSecret,
    pub key: &'a DeviceKey,
    /// Unix seconds, for the `add` change.
    pub time: u64,
    /// The address book in the `uc-net` format, at most 4,096 bytes.
    pub addr_book: &'a [u8],
}

/// The inviter side of one connection.
pub struct InviterRun {
    host: Host,
    serial: u64,
    own_key: Key,
    peer_key: Key,
    exporter: [u8; 32],
    /// The remote address; QR tries are counted per address (4.6).
    remote: IpAddr,
    state: InvState,
    /// The mode of a counted try; set just before `PairReply`.
    counted: Option<Mode>,
    conf: Option<Confirmation>,
    last_ms: u64,
}

impl InviterRun {
    /// The joiner's key, as the TLS layer gave it.
    pub fn peer_key(&self) -> &Key {
        &self.peer_key
    }

    fn fail(&mut self, r: Option<AbortReason>) -> InviterStep {
        self.finish_failed();
        InviterStep::Abort(r.map(frame::abort))
    }

    fn finish_failed(&mut self) {
        if self.state == InvState::Failed || self.state == InvState::Done {
            return;
        }
        self.state = InvState::Failed;
        self.conf = None;
        if let Some(m) = self.counted {
            let now = self.last_ms;
            self.host.lock().record_failure(self.serial, m, now);
        }
    }

    /// The step timed out or the connection dropped. A counted try stays counted.
    pub fn on_close(&mut self) {
        if !matches!(self.state, InvState::WaitDone | InvState::Done) {
            self.finish_failed();
        }
    }

    /// Handles one frame.
    pub fn on_frame<R: RngCore + CryptoRng>(
        &mut self,
        f: Frame,
        now_ms: u64,
        rng: &mut R,
        group: Option<GroupCtx<'_>>,
    ) -> InviterStep {
        self.last_ms = now_ms;
        match (&self.state, f.ty) {
            (_, T_ABORT) => self.fail(None),
            (InvState::WaitHello, T_HELLO) => self.on_hello(&f.payload, now_ms, rng),
            (InvState::WaitConfirm, T_CONFIRM) => match group {
                Some(g) => self.on_confirm(&f.payload, now_ms, g),
                None => self.fail(Some(AbortReason::Other)),
            },
            (InvState::WaitDone, T_DONE) if f.payload.is_empty() => {
                self.state = InvState::Done;
                InviterStep::Finished
            }
            _ => self.fail(Some(AbortReason::BadMessage)),
        }
    }

    fn on_hello<R: RngCore + CryptoRng>(
        &mut self,
        p: &[u8],
        now_ms: u64,
        rng: &mut R,
    ) -> InviterStep {
        let Some(h) = Hello::parse(p) else {
            return self.fail(Some(AbortReason::BadMessage));
        };
        if h.version != VERSION {
            return self.fail(Some(AbortReason::Version));
        }
        let Some(mode) = Mode::from_byte(h.mode) else {
            return self.fail(Some(AbortReason::BadMessage));
        };
        // Check the limit and count the try in one locked step (4.6).
        let secret = {
            let mut s = self.host.lock();
            let code_off = s.code_off(now_ms);
            let Some(o) = s
                .open
                .as_mut()
                .filter(|o| o.serial == self.serial && o.is_live(now_ms))
            else {
                drop(s);
                return self.fail(Some(AbortReason::NotOpen));
            };
            let secret = match mode {
                Mode::Code => match &o.code {
                    Some(c) if o.tries_code < CODE_TRIES && !code_off => {
                        o.tries_code += 1;
                        c.clone()
                    }
                    _ => {
                        drop(s);
                        return self.fail(Some(AbortReason::NotOpen));
                    }
                },
                Mode::Qr => {
                    // Counted per remote address, so junk hellos from one LAN host cannot use up
                    // the tries of the real joiner (4.6).
                    let allowed = match o.qr_by_source.get(&self.remote) {
                        Some(&n) => n < QR_TRIES,
                        None => o.qr_by_source.len() < QR_SOURCES_MAX,
                    };
                    let Some(qr) = o.qr.clone().filter(|_| allowed) else {
                        drop(s);
                        return self.fail(Some(AbortReason::NotOpen));
                    };
                    *o.qr_by_source.entry(self.remote).or_insert(0) += 1;
                    o.tries_qr += 1;
                    qr
                }
            };
            self.counted = Some(mode);
            secret
        };
        let pw = secret.password(mode);
        drop(secret);
        let (pake, msg_a) = Pake::start(true, &pw, &self.own_key, &self.peer_key, &mut *rng);
        drop(pw);
        // A bad msg_b is a counted try (4.4).
        let Some(k) = pake.finish(&h.msg_b) else {
            return self.fail(Some(AbortReason::BadMessage));
        };
        let conf = confirmation(
            &k,
            &TranscriptInput {
                version: VERSION,
                mode: mode as u8,
                inviter_key: &self.own_key,
                joiner_key: &self.peer_key,
                exporter: &self.exporter,
                msg_b: &h.msg_b,
                msg_a: &msg_a,
            },
        );
        let reply = Reply {
            msg_a,
            conf_a: conf.conf_a,
        }
        .encode();
        self.conf = Some(conf);
        self.state = InvState::WaitConfirm;
        InviterStep::Send(reply)
    }

    fn on_confirm(&mut self, p: &[u8], now_ms: u64, g: GroupCtx<'_>) -> InviterStep {
        let Some(c) = Confirm::parse(p) else {
            return self.fail(Some(AbortReason::BadMessage));
        };
        let conf = self.conf.take().expect("set in on_hello");
        if !verify_mac(&conf.kc_b, &conf.t, &c.conf_b) {
            return self.fail(Some(AbortReason::ConfirmFailed));
        }
        drop(conf);
        {
            let mut s = self.host.lock();
            match s
                .open
                .as_mut()
                .filter(|o| o.serial == self.serial && o.is_live(now_ms))
            {
                Some(o) => {
                    // A correct conf_b ends the pairing for both modes (4.6). Both secrets are
                    // wiped now (4.2 step 5); only the status stays.
                    o.succeeded = true;
                    o.code = None;
                    o.qr = None;
                    o.qr_by_source = HashMap::new();
                    s.failed_in_a_row = 0;
                }
                None => {
                    drop(s);
                    return self.fail(Some(AbortReason::NotOpen));
                }
            }
        }
        self.counted = None;
        // 4.7 steps 1 to 4.
        let added = match g.graph.sign_add(
            g.key,
            self.peer_key,
            c.record.device_type,
            &c.record.name,
            g.time,
        ) {
            Ok(a) => a,
            Err(AddError::Removed) => return self.fail(Some(AbortReason::Other)),
            Err(AddError::Full) => return self.fail(Some(AbortReason::GroupFull)),
            Err(AddError::Change(_)) => return self.fail(Some(AbortReason::Other)),
        };
        let st = State {
            epoch: g.secret.epoch,
            group_secret: g.secret.secret,
            changes: g.graph.canonical().map(Change::to_bytes).collect(),
            addr_book: g.addr_book.to_vec(),
        };
        let Some(frame) = st.encode() else {
            return self.fail(Some(AbortReason::Other));
        };
        self.state = InvState::WaitDone;
        InviterStep::SendState {
            frame,
            added: added.map(Box::new),
        }
    }
}

impl Drop for InviterRun {
    fn drop(&mut self) {
        if self.state != InvState::Done && self.state != InvState::WaitDone {
            self.finish_failed();
        }
        let mut s = self.host.lock();
        s.active = s.active.saturating_sub(1);
    }
}

/// Why the joiner stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinError {
    /// QR mode: the TLS key is not the key in the QR code. Close and send nothing.
    WrongInviterKey,
    /// The inviter's reply was bad or its `conf_a` was wrong. Close without sending anything more.
    CloseSilently,
    /// The inviter sent `PairAbort` with this reason.
    Aborted(AbortReason),
    /// The `PairState` failed a check (4.7). Store nothing and show an error.
    BadState(StateError),
    /// A message out of order or broken. Send `PairAbort(4)`.
    BadMessage,
}

/// Which check on `PairState` failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateError {
    Parse,
    /// A change is not well formed, or the first change is not the only genesis.
    Change,
    /// The inviter's TLS key is not a member.
    InviterNotMember,
    /// The joiner's own key is not a member.
    SelfNotMember,
    /// The secret does not match the current secret change.
    Secret,
}

/// What the joiner stores after a good `PairState`.
pub struct Joined {
    pub graph: Graph,
    pub secret: GroupSecret,
    pub addr_book: Vec<u8>,
    /// `PairDone`, to send after the state is stored.
    pub done_frame: Vec<u8>,
}

impl std::fmt::Debug for Joined {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Joined({:?})", self.graph)
    }
}

enum JoinState {
    WaitReply(Pake),
    WaitState,
    Done,
}

/// The joiner side of one connection.
pub struct JoinerRun {
    mode: Mode,
    password: Zeroizing<Vec<u8>>,
    own_key: Key,
    inviter_key: Key,
    exporter: [u8; 32],
    msg_b: [u8; crate::pake::MSG_LEN],
    record: DeviceRecord,
    state: JoinState,
}

impl JoinerRun {
    /// Starts a run and returns the `PairHello` frame.
    ///
    /// `expected_inviter` is the key from the QR code (QR mode). If the TLS key differs, nothing
    /// is sent.
    pub fn start<R: RngCore + CryptoRng>(
        secret: &PairSecret,
        mode: Mode,
        own_key: Key,
        t: &dyn PairTransport,
        expected_inviter: Option<&Key>,
        record: DeviceRecord,
        rng: R,
    ) -> Result<(JoinerRun, Vec<u8>), JoinError> {
        let inviter_key = t.peer_key();
        if let Some(k) = expected_inviter
            && *k != inviter_key
        {
            return Err(JoinError::WrongInviterKey);
        }
        if mode == Mode::Qr && expected_inviter.is_none() {
            return Err(JoinError::WrongInviterKey);
        }
        let password = secret.password(mode);
        let (pake, msg_b) = Pake::start(false, &password, &inviter_key, &own_key, rng);
        let hello = Hello {
            version: VERSION,
            mode: mode as u8,
            msg_b,
        }
        .encode();
        Ok((
            JoinerRun {
                mode,
                password,
                own_key,
                inviter_key,
                exporter: t.exporter(),
                msg_b,
                record,
                state: JoinState::WaitReply(pake),
            },
            hello,
        ))
    }

    /// Handles `PairReply`: returns `PairConfirm` only if `conf_a` is correct.
    pub fn on_frame(&mut self, f: Frame) -> Result<JoinerStep, JoinError> {
        if f.ty == T_ABORT {
            self.state = JoinState::Done;
            let r = f
                .payload
                .first()
                .and_then(|&b| AbortReason::from_byte(b))
                .unwrap_or(AbortReason::Other);
            return Err(JoinError::Aborted(r));
        }
        match std::mem::replace(&mut self.state, JoinState::Done) {
            JoinState::WaitReply(pake) if f.ty == T_REPLY => {
                let Some(r) = Reply::parse(&f.payload) else {
                    return Err(JoinError::CloseSilently);
                };
                let Some(k) = pake.finish(&r.msg_a) else {
                    return Err(JoinError::CloseSilently);
                };
                let conf = confirmation(
                    &k,
                    &TranscriptInput {
                        version: VERSION,
                        mode: self.mode as u8,
                        inviter_key: &self.inviter_key,
                        joiner_key: &self.own_key,
                        exporter: &self.exporter,
                        msg_b: &self.msg_b,
                        msg_a: &r.msg_a,
                    },
                );
                if !verify_mac(&conf.kc_a, &conf.t, &r.conf_a) {
                    return Err(JoinError::CloseSilently);
                }
                self.password = Zeroizing::new(Vec::new());
                self.state = JoinState::WaitState;
                Ok(JoinerStep::Send(
                    Confirm {
                        conf_b: conf.conf_b,
                        record: self.record.clone(),
                    }
                    .encode(),
                ))
            }
            JoinState::WaitReply(_) => Err(JoinError::CloseSilently),
            JoinState::WaitState if f.ty == T_STATE => self
                .on_state(&f.payload)
                .map(|j| JoinerStep::Joined(Box::new(j))),
            _ => Err(JoinError::BadMessage),
        }
    }

    fn on_state(&self, p: &[u8]) -> Result<Joined, JoinError> {
        let bad = |e| JoinError::BadState(e);
        let st = State::parse(p).ok_or(bad(StateError::Parse))?;
        let mut changes = st.changes.iter();
        let first = changes.next().ok_or(bad(StateError::Change))?;
        let g0 = Change::parse(first).map_err(|_| bad(StateError::Change))?;
        let mut graph = Graph::from_genesis(g0).map_err(|_| bad(StateError::Change))?;
        for c in changes {
            let c = Change::parse(c).map_err(|_| bad(StateError::Change))?;
            match graph.insert(c) {
                Ok(crate::chain::Inserted::Stored) => {}
                _ => return Err(bad(StateError::Change)),
            }
        }
        let members = graph.members();
        if !members.contains(&self.inviter_key) {
            return Err(bad(StateError::InviterNotMember));
        }
        if !members.contains(&self.own_key) {
            return Err(bad(StateError::SelfNotMember));
        }
        let secret = GroupSecret {
            epoch: st.epoch,
            secret: st.group_secret,
        };
        if !graph.secret_matches(&secret) {
            return Err(bad(StateError::Secret));
        }
        Ok(Joined {
            graph,
            secret,
            addr_book: st.addr_book.clone(),
            done_frame: frame::encode(T_DONE, &[]),
        })
    }
}

/// What the joiner does after a frame.
#[derive(Debug)]
pub enum JoinerStep {
    /// Send these bytes (the `PairConfirm`).
    Send(Vec<u8>),
    /// Store this, then send `done_frame` and start normal sync.
    Joined(Box<Joined>),
}
