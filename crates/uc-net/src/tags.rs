//! Rotating discovery tags (plan 3.4, protocol 5.8).
//!
//! The mDNS service instance name is a tag that only group members can recognise:
//!
//! ```text
//! tag = HMAC-SHA256(group_secret, "uclip-mdns-v1" || slot u64 || device(32))[..12]
//! slot = unix seconds / 600   (10-minute slots)
//! instance name = 24 lowercase hex characters
//! ```
//!
//! The device key is in the HMAC so two members never announce the same instance name, and a
//! member that sees a tag learns which member it is. A stranger sees 12 random-looking bytes that
//! change every 10 minutes and differ per device: no stable ID, no device name, no key.
//!
//! A device advertises with the newest secret it holds. It recognises tags made with the current
//! and the previous secret epoch, for the slot before, the current one and the one after (so
//! clocks may be up to about 10 minutes apart).

use hmac::{Hmac, Mac};
use sha2::Sha256;
use uc_core::DeviceId;

const DOMAIN: &[u8] = b"uclip-mdns-v1";
/// Slot length in seconds.
pub const SLOT_SECS: u64 = 600;
/// Tag length in bytes.
pub const TAG_LEN: usize = 12;

/// A discovery tag.
pub type Tag = [u8; TAG_LEN];

/// The slot of a wall-clock time in milliseconds.
pub fn slot(now_ms: u64) -> u64 {
    now_ms / 1000 / SLOT_SECS
}

/// The tag of `device` for `slot` under `secret`.
pub fn tag(secret: &[u8; 32], slot: u64, device: &DeviceId) -> Tag {
    // HMAC accepts keys of any length, so this cannot fail.
    let Ok(mut m) = Hmac::<Sha256>::new_from_slice(secret) else {
        return [0; TAG_LEN];
    };
    m.update(DOMAIN);
    m.update(&slot.to_be_bytes());
    m.update(&device.0);
    let out = m.finalize().into_bytes();
    let mut t = [0u8; TAG_LEN];
    t.copy_from_slice(&out[..TAG_LEN]);
    t
}

/// The mDNS instance name of a tag.
pub fn instance_name(t: &Tag) -> String {
    t.iter().map(|b| format!("{b:02x}")).collect()
}

/// Parse an instance name back into a tag (`None` if it is not one of ours).
pub fn parse_instance(name: &str) -> Option<Tag> {
    if name.len() != TAG_LEN * 2 {
        return None;
    }
    let mut t = [0u8; TAG_LEN];
    for (i, b) in t.iter_mut().enumerate() {
        *b = u8::from_str_radix(name.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    // Lowercase only, so one tag has one name.
    (instance_name(&t) == name).then_some(t)
}

/// The secrets this device knows: the newest one and, if any, the one before it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Secrets {
    /// The newest secret (used to advertise).
    pub current: Option<[u8; 32]>,
    /// The previous epoch's secret (recognised only).
    pub previous: Option<[u8; 32]>,
}

/// Recognise a tag: which member made it? `members` must not include this device.
pub fn recognise(
    t: &Tag,
    secrets: &Secrets,
    members: &[DeviceId],
    now_ms: u64,
) -> Option<DeviceId> {
    let s = slot(now_ms);
    let slots = [s.saturating_sub(1), s, s + 1];
    for secret in [secrets.current.as_ref(), secrets.previous.as_ref()]
        .into_iter()
        .flatten()
    {
        for m in members {
            for sl in slots {
                if constant_eq(&tag(secret, sl, m), t) {
                    return Some(*m);
                }
            }
        }
    }
    None
}

fn constant_eq(a: &Tag, b: &Tag) -> bool {
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: [u8; 32] = [9; 32];

    #[test]
    fn rotates_and_differs_per_device() {
        let a = DeviceId([1; 32]);
        let b = DeviceId([2; 32]);
        assert_ne!(tag(&S, 1, &a), tag(&S, 2, &a));
        assert_ne!(tag(&S, 1, &a), tag(&S, 1, &b));
        assert_ne!(tag(&S, 1, &a), tag(&[8; 32], 1, &a));
        assert_eq!(tag(&S, 1, &a), tag(&S, 1, &a));
    }

    #[test]
    fn vector() {
        // Fixed vector so other implementations (Swift, Kotlin) can check theirs.
        // Computed independently with Python's hmac module.
        let t = tag(&[0u8; 32], 0, &DeviceId([0u8; 32]));
        assert_eq!(instance_name(&t), "2d74fe95931a5fcdd7a2754b");
        assert_eq!(parse_instance(&instance_name(&t)), Some(t));
        let t = tag(&[9u8; 32], 2_963_733, &DeviceId([1u8; 32]));
        assert_eq!(instance_name(&t), "01f8d3001d9a73a3912bf04c");
    }

    #[test]
    fn names() {
        assert_eq!(
            parse_instance("00112233445566778899aabb").map(|t| t[11]),
            Some(0xbb)
        );
        assert_eq!(parse_instance("00112233445566778899AABB"), None);
        assert_eq!(parse_instance("0011"), None);
        assert_eq!(parse_instance("zz112233445566778899aabb"), None);
        assert_eq!(parse_instance("printer"), None);
    }

    #[test]
    fn recognise_window() {
        let a = DeviceId([1; 32]);
        let b = DeviceId([2; 32]);
        let now = 1_000 * SLOT_SECS * 1000 + 5;
        let s = slot(now);
        let secrets = Secrets {
            current: Some(S),
            previous: Some([7; 32]),
        };
        assert_eq!(recognise(&tag(&S, s, &a), &secrets, &[a, b], now), Some(a));
        assert_eq!(
            recognise(&tag(&S, s - 1, &b), &secrets, &[a, b], now),
            Some(b)
        );
        assert_eq!(
            recognise(&tag(&S, s + 1, &a), &secrets, &[a, b], now),
            Some(a)
        );
        assert_eq!(recognise(&tag(&S, s + 2, &a), &secrets, &[a, b], now), None);
        assert_eq!(
            recognise(&tag(&[7; 32], s, &a), &secrets, &[a, b], now),
            Some(a)
        );
        assert_eq!(
            recognise(&tag(&[6; 32], s, &a), &secrets, &[a, b], now),
            None
        );
        // Not a member.
        assert_eq!(recognise(&tag(&S, s, &a), &secrets, &[b], now), None);
    }
}
