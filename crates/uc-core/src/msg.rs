//! Messages on `uclip/1` and their codec (plan 3.8).
//!
//! Frame layout: `[protocol version: u8][message type: u8][postcard body]`.
//!
//! - Every frame starts with the protocol version byte. A receiver accepts the current and the
//!   previous version and refuses others.
//! - An unknown message type is skipped (decodes to `Ok(None)`), so a newer peer can add types.
//! - Bytes after a known body are ignored, so a newer peer can append fields at the end.
//! - Decoding never panics on any input (property tested).

use serde::{Deserialize, Serialize};

use crate::clip::PartMeta;
use crate::hlc::Stamp;
use crate::ids::{ClipId, DeviceId};

/// The protocol version this build speaks.
pub const PROTOCOL_VERSION: u8 = 1;
/// The oldest version this build accepts (the current and the previous one).
pub const MIN_PROTOCOL_VERSION: u8 = if PROTOCOL_VERSION > 1 {
    PROTOCOL_VERSION - 1
} else {
    1
};
/// The largest frame accepted: all three parts at their caps plus room for the header.
pub const MAX_FRAME: usize =
    crate::caps::TEXT_MAX + crate::caps::HTML_MAX + crate::caps::IMAGE_MAX + 64 * 1024;

/// What part a device plays in sync (plan 3.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// Desktop agent, Android service: always on, pushes and receives.
    Full,
    /// iOS app and extensions: connects on demand, pulls the newest clip.
    PullOnly,
}

/// The kind of device, for the icon in the device list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceKind {
    /// Windows computer.
    Windows,
    /// Mac.
    Mac,
    /// Linux computer.
    Linux,
    /// Android phone or tablet.
    Android,
    /// iPhone.
    IPhone,
    /// iPad.
    IPad,
}

/// A device as other members see it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRecord {
    /// Public key.
    pub id: DeviceId,
    /// Display name, for example "Max's Mac".
    pub name: String,
    /// Kind.
    pub kind: DeviceKind,
}

/// First message on every connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    /// Protocol version of the sender.
    pub protocol: u8,
    /// App version of the sender, for "Update needed".
    pub app_version: String,
    /// The sender.
    pub device: DeviceRecord,
    /// Full member or pull-only client.
    pub role: Role,
    /// Membership chain heads (filled in M3).
    pub chain_heads: Vec<[u8; 32]>,
    /// Group secret epoch.
    pub epoch: u64,
    /// The sender's wall clock in ms. Lets a device learn that its own clock is wrong.
    pub wall_ms: u64,
}

/// The header of a clip: everything except the bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipHeader {
    /// Wire ID.
    pub id: ClipId,
    /// The device where the copy happened.
    pub origin: DeviceId,
    /// Clock stamp (its device is the origin).
    pub stamp: Stamp,
    /// Format, size and hash of each part, in canonical order.
    pub parts: Vec<PartMeta>,
}

impl ClipHeader {
    /// Total size of all parts.
    pub fn size(&self) -> u64 {
        self.parts.iter().map(|p| u64::from(p.size)).sum()
    }

    /// Is the header self-consistent? (Wire ID matches the part list, origin matches stamp.)
    pub fn is_consistent(&self) -> bool {
        self.origin == self.stamp.device
            && !self.parts.is_empty()
            && self.parts.len() <= 3
            && crate::clip::wire_id(&self.parts) == self.id
    }
}

/// One message on `uclip/1`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// First message on a connection.
    Hello(Hello),
    /// Missing membership changes (opaque here, M3 defines them).
    Membership {
        /// Encoded, signed changes.
        changes: Vec<Vec<u8>>,
    },
    /// A device's signed LAN addresses and port (opaque here, M2 defines it).
    AddrUpdate {
        /// Encoded, signed address list.
        signed: Vec<u8>,
    },
    /// Metadata of the newest clip the sender holds.
    Latest {
        /// `None` if the sender holds no clip.
        header: Option<ClipHeader>,
    },
    /// A whole clip.
    Clip {
        /// The header.
        header: ClipHeader,
        /// The bytes of each part, in header order.
        data: Vec<Vec<u8>>,
    },
    /// "Send me clip X."
    Want {
        /// The wire ID.
        id: ClipId,
    },
    /// "You were removed from the group."
    Removed,
}

const T_HELLO: u8 = 0;
const T_MEMBERSHIP: u8 = 1;
const T_ADDR: u8 = 2;
const T_LATEST: u8 = 3;
const T_CLIP: u8 = 4;
const T_WANT: u8 = 5;
const T_REMOVED: u8 = 6;

#[derive(Serialize, Deserialize)]
struct MembershipBody {
    changes: Vec<Vec<u8>>,
}
#[derive(Serialize, Deserialize)]
struct AddrBody {
    signed: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
struct LatestBody {
    header: Option<ClipHeader>,
}
#[derive(Serialize, Deserialize)]
struct ClipBody {
    header: ClipHeader,
    data: Vec<Vec<u8>>,
}
#[derive(Serialize, Deserialize)]
struct WantBody {
    id: ClipId,
}

/// Why a frame could not be decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodecError {
    /// Fewer than 2 bytes.
    Short,
    /// Larger than [`MAX_FRAME`].
    TooLarge(usize),
    /// A protocol version this build does not speak.
    Version(u8),
    /// The body is not valid for its type.
    Malformed,
}

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodecError::Short => f.write_str("frame too short"),
            CodecError::TooLarge(n) => write!(f, "frame of {n} bytes is too large"),
            CodecError::Version(v) => write!(f, "unsupported protocol version {v}"),
            CodecError::Malformed => f.write_str("malformed message body"),
        }
    }
}

impl std::error::Error for CodecError {}

fn body<T: Serialize>(v: &T) -> Vec<u8> {
    // Serializing our own plain structs into a Vec cannot fail.
    postcard::to_allocvec(v).unwrap_or_default()
}

/// Encode a message as one frame with the current protocol version.
pub fn encode(msg: &Message) -> Vec<u8> {
    encode_with_version(PROTOCOL_VERSION, msg)
}

/// Encode a message with an explicit protocol version byte (for talking to an older peer).
pub fn encode_with_version(version: u8, msg: &Message) -> Vec<u8> {
    let (tag, payload) = match msg {
        Message::Hello(h) => (T_HELLO, body(h)),
        Message::Membership { changes } => (
            T_MEMBERSHIP,
            body(&MembershipBody {
                changes: changes.clone(),
            }),
        ),
        Message::AddrUpdate { signed } => (
            T_ADDR,
            body(&AddrBody {
                signed: signed.clone(),
            }),
        ),
        Message::Latest { header } => (
            T_LATEST,
            body(&LatestBody {
                header: header.clone(),
            }),
        ),
        Message::Clip { header, data } => (
            T_CLIP,
            body(&ClipBody {
                header: header.clone(),
                data: data.clone(),
            }),
        ),
        Message::Want { id } => (T_WANT, body(&WantBody { id: *id })),
        Message::Removed => (T_REMOVED, Vec::new()),
    };
    let mut out = Vec::with_capacity(2 + payload.len());
    out.push(version);
    out.push(tag);
    out.extend_from_slice(&payload);
    out
}

fn parse<'a, T: Deserialize<'a>>(b: &'a [u8]) -> Result<T, CodecError> {
    postcard::take_from_bytes(b)
        .map(|(v, _rest)| v)
        .map_err(|_| CodecError::Malformed)
}

/// Decode one frame. `Ok(None)` means a message type this build does not know: skip it.
pub fn decode(frame: &[u8]) -> Result<Option<Message>, CodecError> {
    if frame.len() > MAX_FRAME {
        return Err(CodecError::TooLarge(frame.len()));
    }
    let [version, tag, rest @ ..] = frame else {
        return Err(CodecError::Short);
    };
    if !(MIN_PROTOCOL_VERSION..=PROTOCOL_VERSION).contains(version) {
        return Err(CodecError::Version(*version));
    }
    let msg = match *tag {
        T_HELLO => Message::Hello(parse(rest)?),
        T_MEMBERSHIP => {
            let b: MembershipBody = parse(rest)?;
            Message::Membership { changes: b.changes }
        }
        T_ADDR => {
            let b: AddrBody = parse(rest)?;
            Message::AddrUpdate { signed: b.signed }
        }
        T_LATEST => {
            let b: LatestBody = parse(rest)?;
            Message::Latest { header: b.header }
        }
        T_CLIP => {
            let b: ClipBody = parse(rest)?;
            Message::Clip {
                header: b.header,
                data: b.data,
            }
        }
        T_WANT => {
            let b: WantBody = parse(rest)?;
            Message::Want { id: b.id }
        }
        T_REMOVED => Message::Removed,
        _ => return Ok(None),
    };
    Ok(Some(msg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::collection::vec;
    use proptest::prelude::*;
    use uc_clip::Format;

    fn arb_id() -> impl Strategy<Value = [u8; 32]> {
        proptest::array::uniform32(any::<u8>())
    }

    fn arb_stamp() -> impl Strategy<Value = Stamp> {
        (any::<u64>(), any::<u32>(), arb_id()).prop_map(|(w, c, d)| Stamp {
            wall_ms: w,
            counter: c,
            device: DeviceId(d),
        })
    }

    fn arb_format() -> impl Strategy<Value = Format> {
        prop_oneof![
            Just(Format::TextPlain),
            Just(Format::TextHtml),
            Just(Format::ImagePng)
        ]
    }

    fn arb_header() -> impl Strategy<Value = ClipHeader> {
        (
            arb_id(),
            arb_stamp(),
            vec((arb_format(), any::<u32>(), arb_id()), 0..4),
        )
            .prop_map(|(id, stamp, parts)| ClipHeader {
                id: ClipId(id),
                origin: stamp.device,
                stamp,
                parts: parts
                    .into_iter()
                    .map(|(format, size, hash)| PartMeta { format, size, hash })
                    .collect(),
            })
    }

    fn arb_kind() -> impl Strategy<Value = DeviceKind> {
        prop_oneof![
            Just(DeviceKind::Windows),
            Just(DeviceKind::Mac),
            Just(DeviceKind::Linux),
            Just(DeviceKind::Android),
            Just(DeviceKind::IPhone),
            Just(DeviceKind::IPad)
        ]
    }

    fn arb_message() -> impl Strategy<Value = Message> {
        let bytes = || vec(any::<u8>(), 0..64);
        prop_oneof![
            (
                any::<u8>(),
                ".{0,20}",
                arb_id(),
                ".{0,20}",
                arb_kind(),
                any::<bool>(),
                vec(arb_id(), 0..3),
                any::<u64>(),
                any::<u64>()
            )
                .prop_map(|(p, av, id, name, kind, full, heads, epoch, wall)| {
                    Message::Hello(Hello {
                        protocol: p,
                        app_version: av,
                        device: DeviceRecord {
                            id: DeviceId(id),
                            name,
                            kind,
                        },
                        role: if full { Role::Full } else { Role::PullOnly },
                        chain_heads: heads,
                        epoch,
                        wall_ms: wall,
                    })
                }),
            vec(bytes(), 0..4).prop_map(|changes| Message::Membership { changes }),
            bytes().prop_map(|signed| Message::AddrUpdate { signed }),
            proptest::option::of(arb_header()).prop_map(|header| Message::Latest { header }),
            (arb_header(), vec(bytes(), 0..4))
                .prop_map(|(header, data)| Message::Clip { header, data }),
            arb_id().prop_map(|id| Message::Want { id: ClipId(id) }),
            Just(Message::Removed),
        ]
    }

    proptest! {
        #[test]
        fn every_message_round_trips(m in arb_message()) {
            let f = encode(&m);
            prop_assert_eq!(f[0], PROTOCOL_VERSION);
            prop_assert_eq!(decode(&f), Ok(Some(m)));
        }

        #[test]
        fn random_bytes_never_panic(b in vec(any::<u8>(), 0..512)) {
            let _ = decode(&b);
        }

        #[test]
        fn random_bodies_with_valid_prefix_never_panic(tag in 0u8..8, b in vec(any::<u8>(), 0..512)) {
            let mut f = vec![PROTOCOL_VERSION, tag];
            f.extend(b);
            let _ = decode(&f);
        }

        #[test]
        fn mutated_frames_never_panic(m in arb_message(), pos in any::<usize>(), byte in any::<u8>(), cut in any::<usize>()) {
            let mut f = encode(&m);
            let i = pos % f.len();
            f[i] = byte;
            let _ = decode(&f);
            let c = cut % (f.len() + 1);
            let _ = decode(&f[..c]);
        }

        #[test]
        fn trailing_bytes_are_ignored(m in arb_message(), extra in vec(any::<u8>(), 1..16)) {
            let mut f = encode(&m);
            f.extend(extra);
            prop_assert_eq!(decode(&f), Ok(Some(m)));
        }
    }

    #[test]
    fn unknown_type_is_skipped() {
        assert_eq!(decode(&[PROTOCOL_VERSION, 200, 1, 2, 3]), Ok(None));
    }

    #[test]
    fn version_byte_is_checked() {
        let mut f = encode(&Message::Removed);
        f[0] = PROTOCOL_VERSION + 1;
        assert_eq!(decode(&f), Err(CodecError::Version(PROTOCOL_VERSION + 1)));
        f[0] = 0;
        assert_eq!(decode(&f), Err(CodecError::Version(0)));
        assert_eq!(decode(&[1]), Err(CodecError::Short));
    }

    #[test]
    fn previous_version_is_accepted() {
        let f = encode_with_version(MIN_PROTOCOL_VERSION, &Message::Removed);
        assert_eq!(decode(&f), Ok(Some(Message::Removed)));
    }

    #[test]
    fn oversized_frame_is_refused_before_parsing() {
        let f = vec![PROTOCOL_VERSION; MAX_FRAME + 1];
        assert_eq!(decode(&f), Err(CodecError::TooLarge(MAX_FRAME + 1)));
    }

    #[test]
    fn huge_length_prefix_does_not_allocate() {
        // A Membership body that claims u32::MAX entries.
        let f = [PROTOCOL_VERSION, T_MEMBERSHIP, 0xff, 0xff, 0xff, 0xff, 0x0f];
        assert_eq!(decode(&f), Err(CodecError::Malformed));
    }
}
