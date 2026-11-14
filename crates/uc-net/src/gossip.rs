//! Signed address announcements (plan 3.4, method 2), carried in `Message::AddrUpdate`.
//!
//! A device signs its own LAN addresses and UDP port with its device key. Peers check the
//! signature, keep only the newest announcement per device (by `seq`), and pass a new one on to
//! their other members. Because each announcement is signed by its device, a member that passes
//! it on cannot change it.
//!
//! Byte format (big-endian, like `docs/protocol.md`):
//!
//! ```text
//! body   = "uclip-addr-v1" || device(32) || seq u64 || port u16 || flags u8 || n u8
//!          || n × ( family u8 (4 or 6) || ip (4 or 16) || prefix u8 )
//! signed = body || Ed25519 signature over body (64)
//! ```
//!
//! `flags` bit 0: a VPN interface is up on that device (input of the VPN-blocks-LAN rule).
//! `seq` is the sender's wall clock in milliseconds, made strictly larger than its last one.
//! At most [`MAX_ADDRS`] addresses. Addresses that are not LAN-class (public IPv4 or IPv6,
//! CGNAT) make the whole announcement invalid: an honest device never sends them.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use uc_core::DeviceId;

use crate::addr::{self, Policy};
use crate::ifaces::IfAddr;

const DOMAIN: &[u8] = b"uclip-addr-v1";
/// Most addresses in one announcement.
pub const MAX_ADDRS: usize = 8;

/// A verified address announcement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Announcement {
    /// The device.
    pub device: DeviceId,
    /// Sequence (sender's wall clock, ms).
    pub seq: u64,
    /// UDP port.
    pub port: u16,
    /// A VPN interface is up on that device.
    pub vpn_up: bool,
    /// LAN addresses with prefix length.
    pub addrs: Vec<IfAddr>,
}

/// Why an announcement was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GossipError {
    /// Bad length or field.
    Malformed,
    /// More than [`MAX_ADDRS`] addresses.
    TooMany,
    /// A non-LAN address.
    NotLan,
    /// Bad signature.
    BadSignature,
}

impl std::fmt::Display for GossipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            GossipError::Malformed => "malformed address announcement",
            GossipError::TooMany => "too many addresses",
            GossipError::NotLan => "non-LAN address in announcement",
            GossipError::BadSignature => "bad signature on address announcement",
        })
    }
}

impl std::error::Error for GossipError {}

fn body(a: &Announcement) -> Vec<u8> {
    let mut b = Vec::with_capacity(64 + a.addrs.len() * 18);
    b.extend_from_slice(DOMAIN);
    b.extend_from_slice(&a.device.0);
    b.extend_from_slice(&a.seq.to_be_bytes());
    b.extend_from_slice(&a.port.to_be_bytes());
    b.push(u8::from(a.vpn_up));
    b.push(a.addrs.len() as u8);
    for x in &a.addrs {
        match x.ip {
            IpAddr::V4(v4) => {
                b.push(4);
                b.extend_from_slice(&v4.octets());
            }
            IpAddr::V6(v6) => {
                b.push(6);
                b.extend_from_slice(&v6.octets());
            }
        }
        b.push(x.prefix);
    }
    b
}

/// Sign an announcement. The device in `a` must be `key`'s public key, and the addresses must
/// already be LAN-class (the caller takes them from physical interfaces); extra addresses past
/// [`MAX_ADDRS`] are dropped.
pub fn sign(key: &SigningKey, a: &Announcement) -> Vec<u8> {
    let mut a = a.clone();
    a.device = DeviceId(key.verifying_key().to_bytes());
    a.addrs.truncate(MAX_ADDRS);
    let mut b = body(&a);
    let sig = key.sign(&b);
    b.extend_from_slice(&sig.to_bytes());
    b
}

/// Parse and verify a signed announcement. `policy` decides whether loopback counts as LAN
/// (tests only).
pub fn verify(signed: &[u8], policy: &Policy) -> Result<Announcement, GossipError> {
    let fixed = DOMAIN.len() + 32 + 8 + 2 + 1 + 1;
    if signed.len() < fixed + 64 {
        return Err(GossipError::Malformed);
    }
    let (b, sig) = signed.split_at(signed.len() - 64);
    if !b.starts_with(DOMAIN) {
        return Err(GossipError::Malformed);
    }
    let mut p = DOMAIN.len();
    let mut take = |n: usize| -> Result<&[u8], GossipError> {
        let s = b.get(p..p + n).ok_or(GossipError::Malformed)?;
        p += n;
        Ok(s)
    };
    let device: [u8; 32] = take(32)?.try_into().map_err(|_| GossipError::Malformed)?;
    let seq = u64::from_be_bytes(take(8)?.try_into().map_err(|_| GossipError::Malformed)?);
    let port = u16::from_be_bytes(take(2)?.try_into().map_err(|_| GossipError::Malformed)?);
    let flags = take(1)?[0];
    let n = take(1)?[0] as usize;
    if flags > 1 {
        return Err(GossipError::Malformed);
    }
    if n > MAX_ADDRS {
        return Err(GossipError::TooMany);
    }
    let mut addrs = Vec::with_capacity(n);
    for _ in 0..n {
        let fam = take(1)?[0];
        let ip = match fam {
            4 => {
                let o: [u8; 4] = take(4)?.try_into().map_err(|_| GossipError::Malformed)?;
                IpAddr::V4(Ipv4Addr::from(o))
            }
            6 => {
                let o: [u8; 16] = take(16)?.try_into().map_err(|_| GossipError::Malformed)?;
                IpAddr::V6(Ipv6Addr::from(o))
            }
            _ => return Err(GossipError::Malformed),
        };
        let prefix = take(1)?[0];
        let max = if ip.is_ipv4() { 32 } else { 128 };
        if prefix > max {
            return Err(GossipError::Malformed);
        }
        if !addr::is_lan_class(ip, policy) {
            return Err(GossipError::NotLan);
        }
        addrs.push(IfAddr { ip, prefix });
    }
    if p != b.len() || port == 0 {
        return Err(GossipError::Malformed);
    }
    let vk = VerifyingKey::from_bytes(&device).map_err(|_| GossipError::BadSignature)?;
    let sig = Signature::from_slice(sig).map_err(|_| GossipError::BadSignature)?;
    vk.verify_strict(b, &sig)
        .map_err(|_| GossipError::BadSignature)?;
    Ok(Announcement {
        device: DeviceId(device),
        seq,
        port,
        vpn_up: flags & 1 == 1,
        addrs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u8) -> SigningKey {
        SigningKey::from_bytes(&[n; 32])
    }

    fn ann(k: &SigningKey) -> Announcement {
        Announcement {
            device: DeviceId(k.verifying_key().to_bytes()),
            seq: 1_700_000_000_000,
            port: 41_641,
            vpn_up: true,
            addrs: vec![
                IfAddr {
                    ip: "192.168.1.20".parse().unwrap(),
                    prefix: 24,
                },
                IfAddr {
                    ip: "fe80::1234".parse().unwrap(),
                    prefix: 64,
                },
            ],
        }
    }

    #[test]
    fn round_trip() {
        let k = key(3);
        let a = ann(&k);
        let s = sign(&k, &a);
        assert_eq!(verify(&s, &Policy::default()), Ok(a));
    }

    #[test]
    fn every_flipped_bit_is_refused() {
        let k = key(3);
        let s = sign(&k, &ann(&k));
        for i in 0..s.len() {
            for bit in 0..8 {
                let mut m = s.clone();
                m[i] ^= 1 << bit;
                assert!(
                    verify(&m, &Policy::default()).is_err(),
                    "byte {i} bit {bit}"
                );
            }
        }
    }

    #[test]
    fn signed_by_another_key_is_refused() {
        let k = key(3);
        let mut a = ann(&k);
        let s = sign(&key(4), &a);
        // `sign` forces the device to the signer; claim to be device 3 instead.
        let mut forged = s.clone();
        forged[DOMAIN.len()..DOMAIN.len() + 32].copy_from_slice(&a.device.0);
        assert_eq!(
            verify(&forged, &Policy::default()),
            Err(GossipError::BadSignature)
        );
        a.device = DeviceId(key(4).verifying_key().to_bytes());
        assert_eq!(verify(&s, &Policy::default()), Ok(a));
    }

    #[test]
    fn public_addresses_are_refused() {
        let k = key(3);
        for bad in ["8.8.8.8", "2a02:1234::1", "100.64.1.1", "127.0.0.1"] {
            let mut a = ann(&k);
            a.addrs.push(IfAddr {
                ip: bad.parse().unwrap(),
                prefix: 24,
            });
            assert_eq!(
                verify(&sign(&k, &a), &Policy::default()),
                Err(GossipError::NotLan),
                "{bad}"
            );
        }
    }

    #[test]
    fn too_many_and_truncation() {
        let k = key(3);
        let mut a = ann(&k);
        a.addrs = (1..=12)
            .map(|i| IfAddr {
                ip: IpAddr::V4(Ipv4Addr::new(10, 0, 0, i)),
                prefix: 8,
            })
            .collect();
        let v = verify(&sign(&k, &a), &Policy::default()).unwrap();
        assert_eq!(v.addrs.len(), MAX_ADDRS);
    }

    proptest::proptest! {
        #[test]
        fn random_bytes_never_panic(b in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..400)) {
            let _ = verify(&b, &Policy::default());
        }
    }
}
