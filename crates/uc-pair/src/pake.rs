//! SPAKE2 wrapper, transcript and confirmation (protocol 4.4 and 4.5).

use curve25519_dalek::edwards::CompressedEdwardsY;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand_core::{CryptoRng, RngCore};
use sha2::{Digest, Sha256};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::chain::Key;

/// ALPN and transcript label.
pub const ALPN: &[u8] = b"uclip-pair/1";
/// TLS exporter label. The context is empty and the output is 32 bytes.
pub const EXPORTER_LABEL: &[u8] = b"EXPORTER-uclip-pair-1";
/// Protocol version in `PairHello`.
pub const VERSION: u8 = 1;
/// Length of a SPAKE2 message: 1 side byte, 32 point bytes.
pub const MSG_LEN: usize = 33;
/// Side byte of the inviter's message.
pub const SIDE_A: u8 = 0x41;
/// Side byte of the joiner's message.
pub const SIDE_B: u8 = 0x42;

const INFO_A: &[u8] = b"uclip-pair/1 confirm inviter";
const INFO_B: &[u8] = b"uclip-pair/1 confirm joiner";

/// Pairing mode byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Code = 1,
    Qr = 2,
}

impl Mode {
    pub fn from_byte(b: u8) -> Option<Mode> {
        match b {
            1 => Some(Mode::Code),
            2 => Some(Mode::Qr),
            _ => None,
        }
    }
}

/// The shared secret of one open pairing, as one side knows it. Wiped on drop.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub enum PairSecret {
    /// Exactly 6 ASCII digits.
    Code([u8; 6]),
    Qr([u8; 16]),
}

impl std::fmt::Debug for PairSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PairSecret::Code(_) => f.write_str("PairSecret::Code(..)"),
            PairSecret::Qr(_) => f.write_str("PairSecret::Qr(..)"),
        }
    }
}

impl PairSecret {
    /// Parses a typed code: exactly 6 ASCII digits.
    pub fn code(s: &str) -> Option<PairSecret> {
        let b = s.as_bytes();
        if b.len() == 6 && b.iter().all(u8::is_ascii_digit) {
            let mut a = [0u8; 6];
            a.copy_from_slice(b);
            Some(PairSecret::Code(a))
        } else {
            None
        }
    }

    pub fn mode(&self) -> Mode {
        match self {
            PairSecret::Code(_) => Mode::Code,
            PairSecret::Qr(_) => Mode::Qr,
        }
    }

    /// The SPAKE2 password for a given mode. With a mode that does not match the secret's kind
    /// (a test of a wrong mode) the secret bytes are used with the other label.
    pub fn password(&self, mode: Mode) -> Zeroizing<Vec<u8>> {
        let mut p = Zeroizing::new(Vec::with_capacity(40));
        p.extend_from_slice(match mode {
            Mode::Code => b"uclip-pair/1 code:".as_slice(),
            Mode::Qr => b"uclip-pair/1 qr:".as_slice(),
        });
        match self {
            PairSecret::Code(c) => p.extend_from_slice(c),
            PairSecret::Qr(q) => p.extend_from_slice(q),
        }
        p
    }
}

/// Checks a peer's SPAKE2 message before `finish` (4.4): length 33, the peer's side byte, the 32
/// bytes decompress to a point, and the point is not of low order.
pub fn check_peer_msg(msg: &[u8], peer_side: u8) -> bool {
    if msg.len() != MSG_LEN || msg[0] != peer_side {
        return false;
    }
    let Ok(c) = CompressedEdwardsY::from_slice(&msg[1..]) else {
        return false;
    };
    match c.decompress() {
        Some(p) => !p.is_small_order(),
        None => false,
    }
}

/// One side's SPAKE2 state. Used once.
pub struct Pake {
    state: Spake2<Ed25519Group>,
    peer_side: u8,
}

impl Pake {
    /// Starts side A (inviter) or B (joiner) with the given RNG.
    pub fn start<R: RngCore + CryptoRng>(
        inviter: bool,
        password: &[u8],
        id_a: &Key,
        id_b: &Key,
        rng: R,
    ) -> (Pake, [u8; MSG_LEN]) {
        let pw = Password::new(password);
        let (ia, ib) = (Identity::new(id_a), Identity::new(id_b));
        let (state, msg) = if inviter {
            Spake2::<Ed25519Group>::start_a_with_rng(&pw, &ia, &ib, rng)
        } else {
            Spake2::<Ed25519Group>::start_b_with_rng(&pw, &ia, &ib, rng)
        };
        let mut out = [0u8; MSG_LEN];
        out.copy_from_slice(&msg);
        let peer_side = if inviter { SIDE_B } else { SIDE_A };
        (Pake { state, peer_side }, out)
    }

    /// Checks the peer's message (low-order rejection) and finishes. `None` if the message is bad.
    pub fn finish(self, peer_msg: &[u8]) -> Option<Zeroizing<[u8; 32]>> {
        if !check_peer_msg(peer_msg, self.peer_side) {
            return None;
        }
        let k = Zeroizing::new(self.state.finish(peer_msg).ok()?);
        let mut out = Zeroizing::new([0u8; 32]);
        out.copy_from_slice(k.get(..32)?);
        Some(out)
    }
}

/// Transcript and confirmation values (4.5). Wiped on drop.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct Confirmation {
    pub tt: Vec<u8>,
    pub t: [u8; 32],
    pub kc_a: [u8; 32],
    pub kc_b: [u8; 32],
    pub conf_a: [u8; 32],
    pub conf_b: [u8; 32],
}

fn lp(v: &mut Vec<u8>, x: &[u8]) {
    v.extend_from_slice(&(x.len() as u32).to_be_bytes());
    v.extend_from_slice(x);
}

/// Inputs of the transcript.
pub struct TranscriptInput<'a> {
    pub version: u8,
    pub mode: u8,
    pub inviter_key: &'a Key,
    pub joiner_key: &'a Key,
    pub exporter: &'a [u8; 32],
    pub msg_b: &'a [u8],
    pub msg_a: &'a [u8],
}

/// Builds `TT`, `T`, the confirmation keys and both confirmation MACs from `K`.
pub fn confirmation(k: &[u8; 32], i: &TranscriptInput<'_>) -> Confirmation {
    let mut tt = Vec::with_capacity(220);
    lp(&mut tt, ALPN);
    lp(&mut tt, &[i.version]);
    lp(&mut tt, &[i.mode]);
    lp(&mut tt, i.inviter_key);
    lp(&mut tt, i.joiner_key);
    lp(&mut tt, i.exporter);
    lp(&mut tt, i.msg_b);
    lp(&mut tt, i.msg_a);
    let t: [u8; 32] = Sha256::digest(&tt).into();
    let hk = Hkdf::<Sha256>::new(Some(&t), k);
    let mut kc_a = [0u8; 32];
    let mut kc_b = [0u8; 32];
    hk.expand(INFO_A, &mut kc_a)
        .expect("32 bytes is a valid length");
    hk.expand(INFO_B, &mut kc_b)
        .expect("32 bytes is a valid length");
    let conf_a = mac(&kc_a, &t);
    let conf_b = mac(&kc_b, &t);
    Confirmation {
        tt,
        t,
        kc_a,
        kc_b,
        conf_a,
        conf_b,
    }
}

fn mac(key: &[u8; 32], t: &[u8; 32]) -> [u8; 32] {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("any key length");
    m.update(t);
    m.finalize().into_bytes().into()
}

/// Checks a received MAC in constant time.
pub fn verify_mac(key: &[u8; 32], t: &[u8; 32], received: &[u8]) -> bool {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("any key length");
    m.update(t);
    m.verify_slice(received).is_ok()
}

/// The 6-digit code (4.2): draw a `u32`; redraw if it is 4,294,000,000 or more; else modulo
/// 1,000,000.
pub fn random_code<R: RngCore + CryptoRng>(rng: &mut R) -> PairSecret {
    loop {
        let x = rng.next_u32();
        if x < 4_294_000_000 {
            let s = format!("{:06}", x % 1_000_000);
            return PairSecret::code(&s).expect("6 digits");
        }
    }
}
