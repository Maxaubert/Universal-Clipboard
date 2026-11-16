//! Messages on `uclip/1` and their codec (plan 3.8).
//!
//! Frame layout: `[protocol version: u8][message type: u8][postcard body]`.
//!
//! - Every frame starts with the protocol version byte. A receiver accepts the current and the
//!   previous version and refuses others.
//! - An unknown message type is skipped (decodes to `Ok(None)`), so a newer peer can add types.
//! - Bytes after a known body are ignored, so a newer peer can append fields at the end.
//! - Decoding never panics on any input (property tested).
//!
//! File sync (0.2) keeps every 0.1.x frame byte for byte. It adds [`HelloExt`] after the
//! `Hello` body (old peers ignore it; a missing one reads as no features) and three message
//! types (`FilesOffer`, `FilesWant`, `FilesCancel`, tags 9 to 11) that old peers skip. Files
//! never appear in `Latest` or `Clip`: an unknown `Format` there would make an old peer close
//! the link.

use serde::{Deserialize, Serialize};

use crate::clip::PartMeta;
use crate::hlc::Stamp;
use crate::ids::{ClipId, DeviceId, FilesId};

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

/// Feature bit: this device can receive file copies (file sync spec 3).
pub const FEATURE_FILES: u32 = 1;
/// The features this build supports.
pub const FEATURES: u32 = FEATURE_FILES;

/// Fields after the 0.1.x `Hello` body (file sync spec 3). Every future `Hello` field goes in
/// here, at the end. A `Hello` without it (from 0.1.x) reads as [`HelloExt::default`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloExt {
    /// Feature bits, see [`FEATURE_FILES`].
    pub features: u32,
}

impl HelloExt {
    /// Does the peer accept file copies?
    pub fn files(&self) -> bool {
        self.features & FEATURE_FILES != 0
    }
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
    /// The sender's group ID (protocol 5.9 step 2): the change ID of its genesis, or all
    /// zeros when it has no group. Added in M7.
    pub group: [u8; 32],
    /// The extension after the 0.1.x body. The codec writes it by hand after the other
    /// fields, so serde skips it.
    #[serde(skip)]
    pub ext: HelloExt,
}

/// A file copy on offer (file sync spec 3): the header only. The files come on a data stream
/// after a `FilesWant`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesOffer {
    /// The copy's ID (see [`crate::files::files_id`]).
    pub id: FilesId,
    /// The device where the copy happened. Only it offers and serves the copy.
    pub origin: DeviceId,
    /// Clock stamp (its device is the origin).
    pub stamp: Stamp,
    /// Total bytes of all files.
    pub total: u64,
    /// Number of files.
    pub count: u32,
    /// BLAKE3 of the manifest that starts the data stream.
    pub manifest_hash: [u8; 32],
    /// Picked with "Copy files..." or `uc copy` (no byte limit).
    pub picked: bool,
}

impl FilesOffer {
    /// Is the offer self-consistent? (Origin matches stamp, ID matches manifest and stamp.)
    pub fn is_consistent(&self) -> bool {
        self.origin == self.stamp.device
            && crate::files::files_id(&self.manifest_hash, &self.stamp) == self.id
    }
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
    /// "Send me the group secret of this epoch" (protocol 5.8). The agent handles it, not the
    /// engine.
    WantSecret {
        /// The epoch.
        epoch: u32,
    },
    /// The group secret of one epoch (protocol 5.8). Sent only to members, after membership
    /// sync. The agent handles it, not the engine.
    Secret {
        /// The epoch.
        epoch: u32,
        /// The secret.
        secret: [u8; 32],
    },
    /// "I hold this file copy." Sent only by the origin, only to peers whose `Hello` has
    /// [`FEATURE_FILES`].
    FilesOffer(FilesOffer),
    /// "Send me file copy X on a data stream."
    FilesWant {
        /// The copy.
        id: FilesId,
    },
    /// "Stop file copy X" (either side). `reason` is
    /// [`crate::files::CODE_SUPERSEDED`] or [`crate::files::CODE_ABORTED`].
    FilesCancel {
        /// The copy.
        id: FilesId,
        /// Why.
        reason: u8,
    },
}

const T_HELLO: u8 = 0;
const T_MEMBERSHIP: u8 = 1;
const T_ADDR: u8 = 2;
const T_LATEST: u8 = 3;
const T_CLIP: u8 = 4;
const T_WANT: u8 = 5;
const T_REMOVED: u8 = 6;
const T_WANT_SECRET: u8 = 7;
const T_SECRET: u8 = 8;
const T_FILES_OFFER: u8 = 9;
const T_FILES_WANT: u8 = 10;
const T_FILES_CANCEL: u8 = 11;

/// The message type byte of a frame (the second byte), or `None` for a frame that is too
/// short. The agent uses it to route membership, address and secret messages before the
/// engine sees a frame.
pub fn frame_type(frame: &[u8]) -> Option<FrameType> {
    Some(match *frame.get(1)? {
        T_HELLO => FrameType::Hello,
        T_MEMBERSHIP => FrameType::Membership,
        T_ADDR => FrameType::AddrUpdate,
        T_WANT_SECRET | T_SECRET => FrameType::Secret,
        T_REMOVED => FrameType::Removed,
        _ => FrameType::Clip,
    })
}

/// Coarse frame classes, for routing in the agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameType {
    /// `Hello`.
    Hello,
    /// `Membership`.
    Membership,
    /// `AddrUpdate`.
    AddrUpdate,
    /// `WantSecret` or `Secret`.
    Secret,
    /// `Removed`.
    Removed,
    /// `Latest`, `Clip`, `Want`, the file messages and unknown types: the engine's business.
    Clip,
}

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
#[derive(Serialize, Deserialize)]
struct WantSecretBody {
    epoch: u32,
}
#[derive(Serialize, Deserialize)]
struct SecretBody {
    epoch: u32,
    secret: [u8; 32],
}
#[derive(Serialize, Deserialize)]
struct FilesWantBody {
    id: FilesId,
}
#[derive(Serialize, Deserialize)]
struct FilesCancelBody {
    id: FilesId,
    reason: u8,
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
        Message::Hello(h) => {
            let mut b = body(h);
            b.extend_from_slice(&body(&h.ext));
            (T_HELLO, b)
        }
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
        Message::WantSecret { epoch } => (T_WANT_SECRET, body(&WantSecretBody { epoch: *epoch })),
        Message::Secret { epoch, secret } => (
            T_SECRET,
            body(&SecretBody {
                epoch: *epoch,
                secret: *secret,
            }),
        ),
        Message::FilesOffer(o) => (T_FILES_OFFER, body(o)),
        Message::FilesWant { id } => (T_FILES_WANT, body(&FilesWantBody { id: *id })),
        Message::FilesCancel { id, reason } => (
            T_FILES_CANCEL,
            body(&FilesCancelBody {
                id: *id,
                reason: *reason,
            }),
        ),
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
        T_HELLO => {
            let (mut h, ext_bytes): (Hello, &[u8]) =
                postcard::take_from_bytes(rest).map_err(|_| CodecError::Malformed)?;
            // A 0.1.x peer sends no extension: no features. A broken one counts the same.
            h.ext = parse(ext_bytes).unwrap_or_default();
            Message::Hello(h)
        }
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
        T_WANT_SECRET => {
            let b: WantSecretBody = parse(rest)?;
            Message::WantSecret { epoch: b.epoch }
        }
        T_SECRET => {
            let b: SecretBody = parse(rest)?;
            Message::Secret {
                epoch: b.epoch,
                secret: b.secret,
            }
        }
        T_FILES_OFFER => Message::FilesOffer(parse(rest)?),
        T_FILES_WANT => {
            let b: FilesWantBody = parse(rest)?;
            Message::FilesWant { id: b.id }
        }
        T_FILES_CANCEL => {
            let b: FilesCancelBody = parse(rest)?;
            Message::FilesCancel {
                id: b.id,
                reason: b.reason,
            }
        }
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
                any::<u64>(),
                (arb_id(), any::<u32>())
            )
                .prop_map(
                    |(p, av, id, name, kind, full, heads, epoch, wall, (group, features))| {
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
                            group,
                            ext: HelloExt { features },
                        })
                    }
                ),
            vec(bytes(), 0..4).prop_map(|changes| Message::Membership { changes }),
            bytes().prop_map(|signed| Message::AddrUpdate { signed }),
            proptest::option::of(arb_header()).prop_map(|header| Message::Latest { header }),
            (arb_header(), vec(bytes(), 0..4))
                .prop_map(|(header, data)| Message::Clip { header, data }),
            arb_id().prop_map(|id| Message::Want { id: ClipId(id) }),
            Just(Message::Removed),
            any::<u32>().prop_map(|epoch| Message::WantSecret { epoch }),
            (any::<u32>(), arb_id()).prop_map(|(epoch, secret)| Message::Secret { epoch, secret }),
            (
                arb_id(),
                arb_stamp(),
                any::<u64>(),
                any::<u32>(),
                arb_id(),
                any::<bool>()
            )
                .prop_map(|(id, stamp, total, count, manifest_hash, picked)| {
                    Message::FilesOffer(FilesOffer {
                        id: FilesId(id),
                        origin: stamp.device,
                        stamp,
                        total,
                        count,
                        manifest_hash,
                        picked,
                    })
                }),
            arb_id().prop_map(|id| Message::FilesWant { id: FilesId(id) }),
            (arb_id(), any::<u8>()).prop_map(|(id, reason)| Message::FilesCancel {
                id: FilesId(id),
                reason
            }),
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
        fn random_bodies_with_valid_prefix_never_panic(tag in 0u8..13, b in vec(any::<u8>(), 0..512)) {
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

        /// A 0.1.x peer parses only the base `Hello` and ignores the rest: the new frame must
        /// start with exactly the 0.1.x bytes, and the 0.1.x frame must read as no features.
        #[test]
        fn new_hello_starts_with_the_old_hello(m in arb_message()) {
            if let Message::Hello(h) = &m {
                let f = encode(&m);
                let old = old_hello_frame(h);
                prop_assert!(f.starts_with(&old));
                let mut legacy = h.clone();
                legacy.ext = HelloExt::default();
                prop_assert_eq!(decode(&old), Ok(Some(Message::Hello(legacy))));
            }
        }
    }

    /// The 0.1.3 encoding of a `Hello`: version, tag, postcard body, nothing after.
    fn old_hello_frame(h: &Hello) -> Vec<u8> {
        let mut f = vec![PROTOCOL_VERSION, T_HELLO];
        f.extend(postcard::to_allocvec(h).unwrap());
        f
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    // Frames encoded by the 0.1.3 codec (captured 2026-11-16).
    const HELLO_013: &str = "01000105302e312e330707070707070707070707070707070707070707070707070707070707070707094d61782773204d616301000109090909090909090909090909090909090909090909090909090909090909090280d8c1a28c340505050505050505050505050505050505050505050505050505050505050505";
    const LATEST_013: &str = "01030101010101010101010101010101010101010101010101010101010101010101010707070707070707070707070707070707070707070707070707070707070707fbd8c1a28c340407070707070707070707070707070707070707070707070707070707070707070100050303030303030303030303030303030303030303030303030303030303030303";
    const LATEST_NONE_013: &str = "010300";
    const CLIP_013: &str = "010401010101010101010101010101010101010101010101010101010101010101010707070707070707070707070707070707070707070707070707070707070707fbd8c1a28c340407070707070707070707070707070707070707070707070707070707070707070100050303030303030303030303030303030303030303030303030303030303030303010568656c6c6f";
    const WANT_013: &str = "01050202020202020202020202020202020202020202020202020202020202020202";

    fn sample_hello(ext: HelloExt) -> Message {
        Message::Hello(Hello {
            protocol: 1,
            app_version: "0.1.3".into(),
            device: DeviceRecord {
                id: DeviceId([7; 32]),
                name: "Max's Mac".into(),
                kind: DeviceKind::Mac,
            },
            role: Role::Full,
            chain_heads: vec![[9; 32]],
            epoch: 2,
            wall_ms: 1_790_000_000_000,
            group: [5; 32],
            ext,
        })
    }

    fn sample_header() -> ClipHeader {
        let dev = DeviceId([7; 32]);
        ClipHeader {
            id: ClipId([1; 32]),
            origin: dev,
            stamp: Stamp {
                wall_ms: 1_790_000_000_123,
                counter: 4,
                device: dev,
            },
            parts: vec![PartMeta {
                format: Format::TextPlain,
                size: 5,
                hash: [3; 32],
            }],
        }
    }

    #[test]
    fn frames_are_byte_compatible_with_0_1_3() {
        // Unchanged messages encode to exactly the 0.1.3 bytes.
        let same = [
            (
                LATEST_013,
                Message::Latest {
                    header: Some(sample_header()),
                },
            ),
            (LATEST_NONE_013, Message::Latest { header: None }),
            (
                CLIP_013,
                Message::Clip {
                    header: sample_header(),
                    data: vec![b"hello".to_vec()],
                },
            ),
            (
                WANT_013,
                Message::Want {
                    id: ClipId([2; 32]),
                },
            ),
        ];
        for (old, m) in same {
            assert_eq!(encode(&m), hex(old), "{m:?}");
            assert_eq!(decode(&hex(old)), Ok(Some(m)));
        }
        // A 0.1.3 Hello reads as one with no features.
        assert_eq!(
            decode(&hex(HELLO_013)),
            Ok(Some(sample_hello(HelloExt::default())))
        );
        // A new Hello is the 0.1.3 bytes plus the extension (features 1 = one varint byte).
        let new = encode(&sample_hello(HelloExt { features: FEATURES }));
        let mut expect = hex(HELLO_013);
        expect.push(0x01);
        assert_eq!(new, expect);
        match decode(&new) {
            Ok(Some(Message::Hello(h))) => assert!(h.ext.files()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn hello_with_a_broken_extension_has_no_features() {
        let mut f = hex(HELLO_013);
        // An unfinished varint.
        f.push(0x80);
        match decode(&f) {
            Ok(Some(Message::Hello(h))) => assert_eq!(h.ext, HelloExt::default()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn file_messages_use_tags_9_to_11_and_route_to_the_engine() {
        let id = FilesId([4; 32]);
        let want = encode(&Message::FilesWant { id });
        let cancel = encode(&Message::FilesCancel { id, reason: 2 });
        assert_eq!(&want[..2], &[PROTOCOL_VERSION, 10]);
        assert_eq!(&cancel[..2], &[PROTOCOL_VERSION, 11]);
        assert_eq!(frame_type(&want), Some(FrameType::Clip));
        let stamp = Stamp {
            wall_ms: 5,
            counter: 0,
            device: DeviceId([7; 32]),
        };
        let offer = FilesOffer {
            id: crate::files::files_id(&[8; 32], &stamp),
            origin: stamp.device,
            stamp,
            total: 10,
            count: 1,
            manifest_hash: [8; 32],
            picked: false,
        };
        assert!(offer.is_consistent());
        let f = encode(&Message::FilesOffer(offer.clone()));
        assert_eq!(f[1], 9);
        assert_eq!(decode(&f), Ok(Some(Message::FilesOffer(offer.clone()))));
        let mut wrong = offer.clone();
        wrong.manifest_hash = [9; 32];
        assert!(!wrong.is_consistent());
        let mut other_origin = offer;
        other_origin.origin = DeviceId([1; 32]);
        assert!(!other_origin.is_consistent());
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
