//! Fixed-size identifiers: device ID, clip wire ID, content hash.

use serde::{Deserialize, Serialize};

macro_rules! id32 {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
        pub struct $name(pub [u8; 32]);

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}(", stringify!($name))?;
                for b in &self.0[..6] {
                    write!(f, "{b:02x}")?;
                }
                f.write_str(")")
            }
        }

        impl From<[u8; 32]> for $name {
            fn from(b: [u8; 32]) -> Self {
                Self(b)
            }
        }
    };
}

id32!(
    /// A device's Ed25519 public key. It is the device's ID and its iroh address (plan 3.7).
    DeviceId
);
id32!(
    /// Wire ID: BLAKE3 over the parts exactly as sent (plan 3.8).
    ClipId
);
id32!(
    /// Content hash: BLAKE3 over a normalized form of the clip (plan 3.8). Used by echo layer 4.
    ContentHash
);
