//! The LAN rule (protocol 4.8).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// One address of a local network interface. `uc-net` fills these in (M2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Iface {
    /// OS interface index (the IPv6 scope ID).
    pub index: u32,
    /// Physical interface as plan section 3.5 defines it (not VPN, not virtual).
    pub physical: bool,
    pub addr: IpAddr,
    /// On-link prefix length of `addr`.
    pub prefix_len: u8,
}

fn private_v4(a: Ipv4Addr) -> bool {
    let o = a.octets();
    o[0] == 10
        || (o[0] == 172 && (o[1] & 0xf0) == 16)
        || (o[0] == 192 && o[1] == 168)
        || (o[0] == 169 && o[1] == 254)
}

fn same_prefix(a: &[u8], b: &[u8], bits: u8) -> bool {
    let bits = bits as usize;
    if bits > a.len() * 8 {
        return false;
    }
    let full = bits / 8;
    if a[..full] != b[..full] {
        return false;
    }
    let rem = bits % 8;
    if rem == 0 {
        return true;
    }
    let mask = 0xffu8 << (8 - rem);
    (a[full] & mask) == (b[full] & mask)
}

fn link_local_v6(a: &Ipv6Addr) -> bool {
    (a.segments()[0] & 0xffc0) == 0xfe80
}

fn ula_v6(a: &Ipv6Addr) -> bool {
    (a.segments()[0] & 0xfe00) == 0xfc00
}

/// True if a pairing connection from (or to) `remote` passes the LAN rule. `scope` is the IPv6
/// scope ID (interface index) of a link-local address; it is ignored for other addresses.
pub fn is_lan_peer(remote: IpAddr, scope: Option<u32>, ifaces: &[Iface]) -> bool {
    let remote = match remote {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
        v4 => v4,
    };
    let phys = ifaces.iter().filter(|i| i.physical);
    match remote {
        IpAddr::V4(r) => {
            if !private_v4(r) {
                return false;
            }
            phys.into_iter().any(|i| match i.addr {
                IpAddr::V4(a) => {
                    i.prefix_len <= 32 && same_prefix(&a.octets(), &r.octets(), i.prefix_len)
                }
                IpAddr::V6(_) => false,
            })
        }
        IpAddr::V6(r) => {
            if link_local_v6(&r) {
                let Some(s) = scope else { return false };
                phys.into_iter().any(|i| i.index == s)
            } else if ula_v6(&r) {
                phys.into_iter().any(|i| match i.addr {
                    IpAddr::V6(a) => {
                        ula_v6(&a)
                            && i.prefix_len >= 7
                            && i.prefix_len <= 128
                            && same_prefix(&a.octets(), &r.octets(), i.prefix_len)
                    }
                    IpAddr::V4(_) => false,
                })
            } else {
                false
            }
        }
    }
}
