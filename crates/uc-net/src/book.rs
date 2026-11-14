//! `LanBook`: the remembered LAN addresses of every member (plan 3.4).
//!
//! Every discovery method feeds it: the address the peer connected from, signed gossip, mDNS
//! answers, and the QR payload or `PairState` at pairing. Reconnect dials every remembered
//! address directly, so no multicast is needed after the first contact.
//!
//! Only LAN-class addresses are stored. Per device at most [`MAX_PER_DEVICE`] addresses; the
//! least recently seen one goes first. The book also keeps each device's newest signed
//! announcement, so it can pass it on (gossip) and put it into `PairState` (protocol 4.7).
//!
//! The agent persists the book with [`LanBook::encode`] and [`LanBook::decode`].

use std::collections::BTreeMap;
use std::net::SocketAddr;

use serde::{Deserialize, Serialize};
use uc_core::DeviceId;

use crate::addr::{self, Policy};
use crate::gossip::{self, Announcement};

/// Most addresses kept per device.
pub const MAX_PER_DEVICE: usize = 16;
const BOOK_VERSION: u8 = 1;

/// Where an address came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Source {
    /// The peer connected to us from it, or we reached it.
    Connected,
    /// A signed announcement.
    Gossip,
    /// mDNS.
    Mdns,
    /// QR code or pairing state.
    Pairing,
}

/// One remembered address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookAddr {
    /// The address and port.
    pub addr: SocketAddr,
    /// Where it came from last.
    pub source: Source,
    /// Last time it was learned or seen (wall clock, ms).
    pub seen_ms: u64,
    /// Last time a connection over it worked (wall clock, ms). 0 if never.
    pub ok_ms: u64,
}

/// What the book knows about one device.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Addresses.
    pub addrs: Vec<BookAddr>,
    /// The newest verified signed announcement (raw bytes), if any.
    pub announcement: Option<Vec<u8>>,
    /// Its sequence number.
    pub seq: u64,
    /// The device said a VPN is up (from its newest announcement).
    pub vpn_up: bool,
    /// Last time a connection to this device worked (wall clock, ms). 0 if never.
    pub connected_ms: u64,
}

/// The address book.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanBook {
    entries: BTreeMap<DeviceId, Entry>,
}

/// What [`LanBook::apply_announcement`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Applied {
    /// It is newer than what we had: stored, pass it on.
    New(Announcement),
    /// Same or older: ignored, do not pass on.
    Old,
}

impl LanBook {
    /// An empty book.
    pub fn new() -> Self {
        Self::default()
    }

    /// The entry of a device.
    pub fn get(&self, d: &DeviceId) -> Option<&Entry> {
        self.entries.get(d)
    }

    /// Every device in the book.
    pub fn devices(&self) -> impl Iterator<Item = &DeviceId> {
        self.entries.keys()
    }

    /// Forget a device (it was removed from the group).
    pub fn forget(&mut self, d: &DeviceId) {
        self.entries.remove(d);
    }

    /// Keep only these devices.
    pub fn retain(&mut self, keep: &[DeviceId]) {
        self.entries.retain(|d, _| keep.contains(d));
    }

    /// Remember an address. Non-LAN addresses are ignored. Returns true if it is new.
    pub fn add(
        &mut self,
        d: DeviceId,
        a: SocketAddr,
        source: Source,
        now_ms: u64,
        policy: &Policy,
    ) -> bool {
        let a = SocketAddr::new(addr::canonical(a.ip()), a.port());
        if !addr::is_lan_class(a.ip(), policy) || a.port() == 0 {
            return false;
        }
        let e = self.entries.entry(d).or_default();
        if let Some(x) = e.addrs.iter_mut().find(|x| same(&x.addr, &a)) {
            x.seen_ms = x.seen_ms.max(now_ms);
            x.source = source;
            return false;
        }
        e.addrs.push(BookAddr {
            addr: a,
            source,
            seen_ms: now_ms,
            ok_ms: 0,
        });
        if e.addrs.len() > MAX_PER_DEVICE {
            // Drop the least useful: never worked, then oldest seen.
            if let Some((i, _)) = e
                .addrs
                .iter()
                .enumerate()
                .min_by_key(|(_, x)| (x.ok_ms, x.seen_ms))
            {
                e.addrs.remove(i);
            }
        }
        true
    }

    /// A connection to `d` over `a` worked.
    pub fn connected(&mut self, d: DeviceId, a: SocketAddr, now_ms: u64, policy: &Policy) {
        self.add(d, a, Source::Connected, now_ms, policy);
        let a = SocketAddr::new(addr::canonical(a.ip()), a.port());
        if let Some(e) = self.entries.get_mut(&d) {
            e.connected_ms = now_ms;
            if let Some(x) = e.addrs.iter_mut().find(|x| same(&x.addr, &a)) {
                x.ok_ms = now_ms;
            }
        }
    }

    /// The addresses to dial for `d`, best first (last worked, then last seen).
    pub fn dial_list(&self, d: &DeviceId) -> Vec<SocketAddr> {
        let Some(e) = self.entries.get(d) else {
            return Vec::new();
        };
        let mut v: Vec<&BookAddr> = e.addrs.iter().collect();
        v.sort_by_key(|a| std::cmp::Reverse((a.ok_ms, a.seen_ms)));
        v.into_iter().map(|x| x.addr).collect()
    }

    /// Verify a signed announcement from `members` and store it if it is newer. The announced
    /// device must be in `members`.
    pub fn apply_announcement(
        &mut self,
        signed: &[u8],
        members: &[DeviceId],
        now_ms: u64,
        policy: &Policy,
    ) -> Result<Applied, gossip::GossipError> {
        let a = gossip::verify(signed, policy)?;
        if !members.contains(&a.device) {
            return Ok(Applied::Old);
        }
        let e = self.entries.entry(a.device).or_default();
        if a.seq <= e.seq {
            return Ok(Applied::Old);
        }
        e.seq = a.seq;
        e.vpn_up = a.vpn_up;
        e.announcement = Some(signed.to_vec());
        for x in &a.addrs {
            self.add(
                a.device,
                SocketAddr::new(x.ip, a.port),
                Source::Gossip,
                now_ms,
                policy,
            );
        }
        Ok(Applied::New(a))
    }

    /// The newest signed announcement of every device (for `PairState`, protocol 4.7):
    /// `u16 count || count × (u16 length || signed)`.
    pub fn export_announcements(&self) -> Vec<u8> {
        let list: Vec<&Vec<u8>> = self
            .entries
            .values()
            .filter_map(|e| e.announcement.as_ref())
            .filter(|a| a.len() <= u16::MAX as usize)
            .collect();
        let mut out = Vec::new();
        out.extend_from_slice(&(list.len() as u16).to_be_bytes());
        for a in list {
            out.extend_from_slice(&(a.len() as u16).to_be_bytes());
            out.extend_from_slice(a);
        }
        out
    }

    /// Read what [`Self::export_announcements`] wrote and apply each announcement. Bad entries
    /// are skipped. Returns how many were stored.
    pub fn import_announcements(
        &mut self,
        bytes: &[u8],
        members: &[DeviceId],
        now_ms: u64,
        policy: &Policy,
    ) -> usize {
        let mut n_ok = 0;
        let Some(count) = bytes.get(..2) else {
            return 0;
        };
        let count = u16::from_be_bytes([count[0], count[1]]) as usize;
        let mut p = 2;
        for _ in 0..count {
            let Some(l) = bytes.get(p..p + 2) else {
                break;
            };
            let l = u16::from_be_bytes([l[0], l[1]]) as usize;
            p += 2;
            let Some(s) = bytes.get(p..p + l) else {
                break;
            };
            p += l;
            if matches!(
                self.apply_announcement(s, members, now_ms, policy),
                Ok(Applied::New(_))
            ) {
                n_ok += 1;
            }
        }
        n_ok
    }

    /// Encode for the agent's state file.
    pub fn encode(&self) -> Vec<u8> {
        let mut v = vec![BOOK_VERSION];
        v.extend(postcard::to_allocvec(self).unwrap_or_default());
        v
    }

    /// Decode from the agent's state file. A bad or unknown file gives an empty book.
    pub fn decode(b: &[u8]) -> Self {
        match b.split_first() {
            Some((&BOOK_VERSION, rest)) => postcard::from_bytes(rest).unwrap_or_default(),
            _ => Self::default(),
        }
    }
}

/// Two addresses are the same if IP and port match (the IPv6 scope is local, so ignored).
fn same(a: &SocketAddr, b: &SocketAddr) -> bool {
    a.ip() == b.ip() && a.port() == b.port()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gossip::sign;
    use crate::ifaces::IfAddr;
    use ed25519_dalek::SigningKey;

    fn d(n: u8) -> DeviceId {
        DeviceId(SigningKey::from_bytes(&[n; 32]).verifying_key().to_bytes())
    }

    fn sa(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn only_lan_addresses() {
        let mut b = LanBook::new();
        let p = Policy::default();
        assert!(b.add(d(1), sa("192.168.1.2:4000"), Source::Mdns, 1, &p));
        assert!(!b.add(d(1), sa("192.168.1.2:4000"), Source::Mdns, 2, &p));
        assert!(!b.add(d(1), sa("8.8.8.8:4000"), Source::Mdns, 1, &p));
        assert!(!b.add(d(1), sa("[2a02::1]:4000"), Source::Mdns, 1, &p));
        assert!(!b.add(d(1), sa("127.0.0.1:4000"), Source::Mdns, 1, &p));
        assert!(!b.add(d(1), sa("192.168.1.3:0"), Source::Mdns, 1, &p));
        assert_eq!(b.dial_list(&d(1)), vec![sa("192.168.1.2:4000")]);
    }

    #[test]
    fn worked_addresses_first_and_cap() {
        let mut b = LanBook::new();
        let p = Policy::default();
        for i in 0..20u8 {
            b.add(
                d(1),
                SocketAddr::new(std::net::Ipv4Addr::new(10, 0, 0, i + 1).into(), 4000),
                Source::Gossip,
                u64::from(i),
                &p,
            );
            if i == 3 {
                b.connected(d(1), sa("10.0.0.4:4000"), 100, &p);
            }
        }
        let l = b.dial_list(&d(1));
        assert_eq!(l.len(), MAX_PER_DEVICE);
        assert_eq!(l[0], sa("10.0.0.4:4000"));
        assert_eq!(l[1], sa("10.0.0.20:4000"));
    }

    #[test]
    fn announcements_newer_only_and_members_only() {
        let k = SigningKey::from_bytes(&[1; 32]);
        let p = Policy::default();
        let a = |seq| Announcement {
            device: d(1),
            seq,
            port: 5000,
            vpn_up: false,
            addrs: vec![IfAddr {
                ip: "192.168.5.5".parse().unwrap(),
                prefix: 24,
            }],
        };
        let mut b = LanBook::new();
        assert_eq!(
            b.apply_announcement(&sign(&k, &a(10)), &[d(2)], 1, &p),
            Ok(Applied::Old)
        );
        assert!(b.get(&d(1)).is_none());
        assert!(matches!(
            b.apply_announcement(&sign(&k, &a(10)), &[d(1)], 1, &p),
            Ok(Applied::New(_))
        ));
        assert_eq!(
            b.apply_announcement(&sign(&k, &a(10)), &[d(1)], 1, &p),
            Ok(Applied::Old)
        );
        assert_eq!(
            b.apply_announcement(&sign(&k, &a(9)), &[d(1)], 1, &p),
            Ok(Applied::Old)
        );
        assert_eq!(b.dial_list(&d(1)), vec![sa("192.168.5.5:5000")]);

        let exported = b.export_announcements();
        let mut c = LanBook::new();
        assert_eq!(c.import_announcements(&exported, &[d(1)], 5, &p), 1);
        assert_eq!(c.dial_list(&d(1)), vec![sa("192.168.5.5:5000")]);
        assert_eq!(c.import_announcements(&exported[..5], &[d(1)], 5, &p), 0);
    }

    #[test]
    fn persist_round_trip() {
        let mut b = LanBook::new();
        let p = Policy::default();
        b.add(d(1), sa("[fe80::1]:4000"), Source::Pairing, 7, &p);
        b.connected(d(2), sa("192.168.1.9:4000"), 9, &p);
        let e = b.encode();
        assert_eq!(LanBook::decode(&e), b);
        assert_eq!(LanBook::decode(&[9, 9, 9]), LanBook::new());
        assert_eq!(LanBook::decode(&[]), LanBook::new());
    }
}
