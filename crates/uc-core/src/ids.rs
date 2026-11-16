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
id32!(
    /// The ID of one file copy (file sync spec 3): BLAKE3 over the manifest hash and the clock
    /// stamp, so every copy has its own ID, even of the same folder.
    FilesId
);

impl FilesId {
    /// The first 8 hex characters, for the short name of the receive folder (spec 2.2.8).
    pub fn short(&self) -> String {
        self.0[..4].iter().map(|b| format!("{b:02x}")).collect()
    }
}
