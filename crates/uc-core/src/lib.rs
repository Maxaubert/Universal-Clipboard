//! Clip model, wire codec, clock and sync state machine.
//!
//! No OS calls. Everything here runs against the [`uc_clip::Clipboard`] trait and the
//! [`sync::Peer`] trait, so it is fully testable with a fake clipboard and a fake network
//! (`uc-testkit`).
//!
//! | Module | Plan section |
//! |---|---|
//! | [`clip`] | clip model, wire ID, content hash (3.8) |
//! | [`text`] | line-ending rule (3.8) |
//! | [`image`] | PNG safety checks, pixel hash (3.11) |
//! | [`caps`] | size caps (3.11) |
//! | [`files`] | file copies: manifest, path rules, hashes (file sync spec 2.5, 3) |
//! | [`hlc`] | hybrid logical clock, future-stamp limit, conflict rule (3.8) |
//! | [`msg`] | messages and codec with version byte (3.8) |
//! | [`sync`] | the state machine, both roles, echo and loop layers (3.8) |

pub mod caps;
pub mod clip;
pub mod files;
pub mod hlc;
pub mod ids;
pub mod image;
pub mod msg;
pub mod sync;
pub mod text;

pub use clip::{Clip, ClipError, ClipKind, PartMeta};
pub use files::{Entry, EntryKind, FilesSummary, Manifest, ManifestError};
pub use hlc::{Hlc, Stamp};
pub use ids::{ClipId, ContentHash, DeviceId, FilesId};
pub use msg::{ClipHeader, DeviceKind, DeviceRecord, FilesOffer, HelloExt, Message, Role};
pub use sync::{
    CancelReason, Config, Engine, Event, FetchOutcome, Peer, PeerGone, SkipReason, TransferFailure,
};
pub use uc_clip::{Clipboard, Format};
