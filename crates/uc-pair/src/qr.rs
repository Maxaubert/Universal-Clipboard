//! QR payload (protocol 4.3): `UCLIP1:` plus strict base32 of the payload bytes.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use data_encoding::BASE32_NOPAD;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::chain::Key;

/// Text prefix of a QR code.
pub const PREFIX: &str = "UCLIP1:";
/// Payload version byte.
pub const QR_VERSION: u8 = 1;
/// Most addresses in one payload.
pub const MAX_ADDRS: usize = 4;

/// A parsed QR payload. The secret is wiped on drop.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct QrPayload {
    #[zeroize(skip)]
    pub inviter_key: Key,
    pub qr_secret: [u8; 16],
    #[zeroize(skip)]
    pub port: u16,
    /// 1 to 4 addresses.
    #[zeroize(skip)]
    pub addrs: Vec<IpAddr>,
}

impl std::fmt::Debug for QrPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QrPayload")
            .field("port", &self.port)
            .field("addrs", &self.addrs)
            .finish_non_exhaustive()
    }
}

/// Why a QR text was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QrError {
    Prefix,
    Base32,
    Version,
    Length,
    AddressCount,
    Family,
}

impl QrPayload {
    /// Payload bytes. IPv4 addresses come first. Panics if there are 0 or more than 4 addresses.
    pub fn to_bytes(&self) -> Vec<u8> {
        assert!(
            (1..=MAX_ADDRS).contains(&self.addrs.len()),
            "1 to 4 addresses"
        );
        let mut v = Vec::with_capacity(52 + 17 * self.addrs.len());
        v.push(QR_VERSION);
        v.extend_from_slice(&self.inviter_key);
        v.extend_from_slice(&self.qr_secret);
        v.extend_from_slice(&self.port.to_be_bytes());
        v.push(self.addrs.len() as u8);
        let v4 = self.addrs.iter().filter(|a| a.is_ipv4());
        let v6 = self.addrs.iter().filter(|a| a.is_ipv6());
        for a in v4.chain(v6) {
            match a {
                IpAddr::V4(a) => {
                    v.push(0x04);
                    v.extend_from_slice(&a.octets());
                }
                IpAddr::V6(a) => {
                    v.push(0x06);
                    v.extend_from_slice(&a.octets());
                }
            }
        }
        v
    }

    /// The QR text.
    pub fn to_text(&self) -> String {
        format!("{PREFIX}{}", BASE32_NOPAD.encode(&self.to_bytes()))
    }

    /// Strict parser of payload bytes.
    pub fn from_bytes(b: &[u8]) -> Result<QrPayload, QrError> {
        if b.len() < 52 {
            return Err(QrError::Length);
        }
        if b[0] != QR_VERSION {
            return Err(QrError::Version);
        }
        let mut inviter_key = [0u8; 32];
        inviter_key.copy_from_slice(&b[1..33]);
        let mut qr_secret = [0u8; 16];
        qr_secret.copy_from_slice(&b[33..49]);
        let port = u16::from_be_bytes([b[49], b[50]]);
        let n = b[51] as usize;
        if !(1..=MAX_ADDRS).contains(&n) {
            return Err(QrError::AddressCount);
        }
        let mut rest = &b[52..];
        let mut addrs = Vec::with_capacity(n);
        for _ in 0..n {
            let (&fam, r) = rest.split_first().ok_or(QrError::Length)?;
            let len = match fam {
                0x04 => 4,
                0x06 => 16,
                _ => return Err(QrError::Family),
            };
            if r.len() < len {
                return Err(QrError::Length);
            }
            let (a, r) = r.split_at(len);
            addrs.push(if len == 4 {
                IpAddr::V4(Ipv4Addr::from(<[u8; 4]>::try_from(a).expect("4")))
            } else {
                IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(a).expect("16")))
            });
            rest = r;
        }
        if !rest.is_empty() {
            return Err(QrError::Length);
        }
        Ok(QrPayload {
            inviter_key,
            qr_secret,
            port,
            addrs,
        })
    }

    /// Strict parser of the QR text: exact prefix, upper-case RFC 4648 base32 without padding,
    /// canonical trailing bits.
    pub fn parse(text: &str) -> Result<QrPayload, QrError> {
        let body = text.strip_prefix(PREFIX).ok_or(QrError::Prefix)?;
        let bytes = BASE32_NOPAD
            .decode(body.as_bytes())
            .map_err(|_| QrError::Base32)?;
        let mut bytes = zeroize::Zeroizing::new(bytes);
        let r = Self::from_bytes(&bytes);
        bytes.zeroize();
        r
    }
}
