//! Pairing frames (protocol 4.4): `u8 type || u32 length || payload`.

use crate::chain::{DEVICE_TYPE_MAX, check_name};
use crate::pake::MSG_LEN;

pub const T_HELLO: u8 = 0x01;
pub const T_REPLY: u8 = 0x02;
pub const T_CONFIRM: u8 = 0x03;
pub const T_STATE: u8 = 0x04;
pub const T_DONE: u8 = 0x05;
pub const T_ABORT: u8 = 0x7F;

/// Length limit of every frame but `PairState`.
pub const MAX_LEN: usize = 256;
/// Length limit of `PairState`.
pub const MAX_STATE_LEN: usize = 1 << 20;
/// Limit of the address book in `PairState`.
pub const MAX_ADDR_BOOK: usize = 4096;

/// `PairAbort` reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbortReason {
    Version = 1,
    NotOpen = 2,
    Busy = 3,
    BadMessage = 4,
    ConfirmFailed = 5,
    GroupFull = 6,
    Other = 7,
}

impl AbortReason {
    pub fn from_byte(b: u8) -> Option<AbortReason> {
        Some(match b {
            1 => AbortReason::Version,
            2 => AbortReason::NotOpen,
            3 => AbortReason::Busy,
            4 => AbortReason::BadMessage,
            5 => AbortReason::ConfirmFailed,
            6 => AbortReason::GroupFull,
            7 => AbortReason::Other,
            _ => return None,
        })
    }
}

/// One frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub ty: u8,
    pub payload: Vec<u8>,
}

/// Encodes a frame.
pub fn encode(ty: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(5 + payload.len());
    v.push(ty);
    v.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    v.extend_from_slice(payload);
    v
}

/// A `PairAbort` frame.
pub fn abort(r: AbortReason) -> Vec<u8> {
    encode(T_ABORT, &[r as u8])
}

/// The stream is broken: the pairing ends with `PairAbort(4)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameError;

fn limit(ty: u8) -> Option<usize> {
    match ty {
        T_HELLO | T_REPLY | T_CONFIRM | T_DONE | T_ABORT => Some(MAX_LEN),
        T_STATE => Some(MAX_STATE_LEN),
        _ => None,
    }
}

/// Splits a byte stream into frames. Refuses unknown types and frames over the limit as soon as
/// the header is read, before the payload arrives.
#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next whole frame, `Ok(None)` if more bytes are needed.
    pub fn next_frame(&mut self) -> Result<Option<Frame>, FrameError> {
        if self.buf.is_empty() {
            return Ok(None);
        }
        let ty = self.buf[0];
        let max = limit(ty).ok_or(FrameError)?;
        if self.buf.len() < 5 {
            return Ok(None);
        }
        let len = u32::from_be_bytes(self.buf[1..5].try_into().expect("4")) as usize;
        if len > max {
            return Err(FrameError);
        }
        if self.buf.len() < 5 + len {
            return Ok(None);
        }
        let payload = self.buf[5..5 + len].to_vec();
        self.buf.drain(..5 + len);
        Ok(Some(Frame { ty, payload }))
    }
}

/// `PairHello` payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    pub version: u8,
    pub mode: u8,
    pub msg_b: [u8; MSG_LEN],
}

impl Hello {
    pub fn encode(&self) -> Vec<u8> {
        let mut p = vec![self.version, self.mode];
        p.extend_from_slice(&self.msg_b);
        encode(T_HELLO, &p)
    }

    /// Only the size is checked here; version and mode are checked by the inviter, which answers
    /// with the right abort reason.
    pub fn parse(p: &[u8]) -> Option<Hello> {
        if p.len() != 2 + MSG_LEN {
            return None;
        }
        Some(Hello {
            version: p[0],
            mode: p[1],
            msg_b: p[2..].try_into().ok()?,
        })
    }
}

/// `PairReply` payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub msg_a: [u8; MSG_LEN],
    pub conf_a: [u8; 32],
}

impl Reply {
    pub fn encode(&self) -> Vec<u8> {
        let mut p = self.msg_a.to_vec();
        p.extend_from_slice(&self.conf_a);
        encode(T_REPLY, &p)
    }

    pub fn parse(p: &[u8]) -> Option<Reply> {
        if p.len() != MSG_LEN + 32 {
            return None;
        }
        Some(Reply {
            msg_a: p[..MSG_LEN].try_into().ok()?,
            conf_a: p[MSG_LEN..].try_into().ok()?,
        })
    }
}

/// The joiner's device record (4.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRecord {
    pub device_type: u8,
    pub name: String,
}

impl DeviceRecord {
    /// Checks the type (0 to 7) and the name (1 to 64 bytes of UTF-8, no control characters).
    pub fn new(device_type: u8, name: &str) -> Option<DeviceRecord> {
        if device_type > DEVICE_TYPE_MAX || check_name(name.as_bytes()).is_none() {
            return None;
        }
        Some(DeviceRecord {
            device_type,
            name: name.to_string(),
        })
    }
}

/// `PairConfirm` payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm {
    pub conf_b: [u8; 32],
    pub record: DeviceRecord,
}

impl Confirm {
    pub fn encode(&self) -> Vec<u8> {
        let mut p = self.conf_b.to_vec();
        p.push(self.record.device_type);
        p.push(self.record.name.len() as u8);
        p.extend_from_slice(self.record.name.as_bytes());
        encode(T_CONFIRM, &p)
    }

    pub fn parse(p: &[u8]) -> Option<Confirm> {
        if p.len() < 34 {
            return None;
        }
        let conf_b = p[..32].try_into().ok()?;
        let n = p[33] as usize;
        if p.len() != 34 + n {
            return None;
        }
        let name = check_name(&p[34..])?;
        Some(Confirm {
            conf_b,
            record: DeviceRecord::new(p[32], name)?,
        })
    }
}

/// `PairState` payload (4.7).
#[derive(Clone, PartialEq, Eq)]
pub struct State {
    pub epoch: u32,
    pub group_secret: [u8; 32],
    /// Each change as `body || signature`, in canonical order.
    pub changes: Vec<Vec<u8>>,
    pub addr_book: Vec<u8>,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("State")
            .field("epoch", &self.epoch)
            .field("changes", &self.changes.len())
            .finish_non_exhaustive()
    }
}

impl Drop for State {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.group_secret);
    }
}

impl State {
    /// Encodes the frame. `None` if a part is over its limit.
    pub fn encode(&self) -> Option<Vec<u8>> {
        if self.changes.len() > u16::MAX as usize || self.addr_book.len() > MAX_ADDR_BOOK {
            return None;
        }
        let mut p = Vec::new();
        p.extend_from_slice(&self.epoch.to_be_bytes());
        p.extend_from_slice(&self.group_secret);
        p.extend_from_slice(&(self.changes.len() as u16).to_be_bytes());
        for c in &self.changes {
            if c.len() > u16::MAX as usize {
                return None;
            }
            p.extend_from_slice(&(c.len() as u16).to_be_bytes());
            p.extend_from_slice(c);
        }
        p.extend_from_slice(&(self.addr_book.len() as u16).to_be_bytes());
        p.extend_from_slice(&self.addr_book);
        if p.len() > MAX_STATE_LEN {
            return None;
        }
        let f = encode(T_STATE, &p);
        zeroize::Zeroize::zeroize(&mut p);
        Some(f)
    }

    pub fn parse(p: &[u8]) -> Option<State> {
        fn take<'a>(p: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
            if p.len() < n {
                return None;
            }
            let (a, b) = p.split_at(n);
            *p = b;
            Some(a)
        }
        let mut r = p;
        let epoch = u32::from_be_bytes(take(&mut r, 4)?.try_into().ok()?);
        let group_secret: [u8; 32] = take(&mut r, 32)?.try_into().ok()?;
        let mut st = State {
            epoch,
            group_secret,
            changes: Vec::new(),
            addr_book: Vec::new(),
        };
        let n = u16::from_be_bytes(take(&mut r, 2)?.try_into().ok()?) as usize;
        for _ in 0..n {
            let len = u16::from_be_bytes(take(&mut r, 2)?.try_into().ok()?) as usize;
            st.changes.push(take(&mut r, len)?.to_vec());
        }
        let al = u16::from_be_bytes(take(&mut r, 2)?.try_into().ok()?) as usize;
        if al > MAX_ADDR_BOOK {
            return None;
        }
        st.addr_book = take(&mut r, al)?.to_vec();
        if !r.is_empty() {
            return None;
        }
        Some(st)
    }
}
