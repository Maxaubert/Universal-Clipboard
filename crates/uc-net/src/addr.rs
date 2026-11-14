//! The LAN rule (protocol page 4.8, plan 4.1): which remote addresses may talk to us at all.
//!
//! The rule applies to pairing AND to sync connections, to every address we dial, and to every
//! address we store (gossip, mDNS, pairing). It is checked on the first packet of a connection,
//! before the TLS handshake starts. A remote address that fails it is dropped without an answer.
//!
//! A remote address passes if it is:
//! - IPv4 inside `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16` or `169.254.0.0/16`, **and**
//!   inside the subnet of an address on the physical interface the packet came in on (or, for a
//!   dial, on one of the physical interfaces);
//! - IPv6 link-local (`fe80::/10`) on a physical interface;
//! - IPv6 unique-local (`fc00::/7`) inside an on-link prefix of a physical interface.
//!
//! Everything else fails: public IPv4, public (global) IPv6, CGNAT `100.64.0.0/10` (Tailscale),
//! loopback, multicast, unspecified. Loopback passes only with [`Policy::allow_loopback`], which
//! exists for the loopback tests and is never set by the agent.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use crate::ifaces::{IfAddr, Interface, Kind};

/// Settings of the LAN rule.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Policy {
    /// Let loopback addresses pass (tests only). Off in every real build of the agent.
    pub allow_loopback: bool,
}

/// The address class, as the LAN rule sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddrClass {
    /// RFC 1918 IPv4.
    PrivateV4,
    /// `169.254.0.0/16`.
    LinkLocalV4,
    /// `fe80::/10`.
    LinkLocalV6,
    /// `fc00::/7`.
    UniqueLocalV6,
    /// `127.0.0.0/8` or `::1`.
    Loopback,
    /// Anything else: public, CGNAT, multicast, unspecified, documentation ranges...
    NotLan,
}

/// Turns an IPv4-mapped IPv6 address (`::ffff:a.b.c.d`) into plain IPv4.
pub fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
        v4 => v4,
    }
}

/// Classify an address.
pub fn classify(ip: IpAddr) -> AddrClass {
    match canonical(ip) {
        IpAddr::V4(v4) => classify_v4(v4),
        IpAddr::V6(v6) => classify_v6(v6),
    }
}

fn classify_v4(ip: Ipv4Addr) -> AddrClass {
    if ip.is_loopback() {
        AddrClass::Loopback
    } else if ip.is_private() {
        AddrClass::PrivateV4
    } else if ip.is_link_local() {
        AddrClass::LinkLocalV4
    } else {
        AddrClass::NotLan
    }
}

fn classify_v6(ip: Ipv6Addr) -> AddrClass {
    let seg0 = ip.segments()[0];
    if ip.is_loopback() {
        AddrClass::Loopback
    } else if seg0 & 0xffc0 == 0xfe80 {
        AddrClass::LinkLocalV6
    } else if seg0 & 0xfe00 == 0xfc00 {
        AddrClass::UniqueLocalV6
    } else {
        AddrClass::NotLan
    }
}

/// Is this address ever allowed to be stored or dialed (class check only, no subnet check)?
/// Used to filter gossip and mDNS answers before they reach the address book.
pub fn is_lan_class(ip: IpAddr, policy: &Policy) -> bool {
    match classify(ip) {
        AddrClass::NotLan => false,
        AddrClass::Loopback => policy.allow_loopback,
        _ => true,
    }
}

/// Is `ip` inside the on-link subnet of `a`?
pub fn in_subnet(ip: IpAddr, a: &IfAddr) -> bool {
    match (canonical(ip), canonical(a.ip)) {
        (IpAddr::V4(x), IpAddr::V4(y)) => {
            let p = u32::from(a.prefix.min(32));
            if p == 0 {
                return false;
            }
            let mask = u32::MAX << (32 - p);
            u32::from(x) & mask == u32::from(y) & mask
        }
        (IpAddr::V6(x), IpAddr::V6(y)) => {
            let p = u32::from(a.prefix.min(128));
            if p == 0 {
                return false;
            }
            let mask = u128::MAX << (128 - p);
            u128::from(x) & mask == u128::from(y) & mask
        }
        _ => false,
    }
}

/// The LAN rule for a packet that came in on (or a dial that goes out of) interface `on`.
///
/// `on` must be a physical interface; any other kind fails (except loopback under
/// [`Policy::allow_loopback`]).
pub fn allowed_on(remote: SocketAddr, on: &Interface, policy: &Policy) -> bool {
    let ip = canonical(remote.ip());
    if remote.port() == 0 {
        return false;
    }
    match classify(ip) {
        AddrClass::NotLan => false,
        AddrClass::Loopback => {
            policy.allow_loopback
                && (on.kind == Kind::Loopback || on.kind == Kind::Physical)
                && on.addrs.iter().any(|a| in_subnet(ip, a))
        }
        AddrClass::LinkLocalV6 => {
            if on.kind != Kind::Physical {
                return false;
            }
            // A scope ID, when the OS gives one, must name this interface.
            let scope = match remote {
                SocketAddr::V6(v6) => v6.scope_id(),
                SocketAddr::V4(_) => 0,
            };
            (scope == 0 || scope == on.index)
                && on
                    .addrs
                    .iter()
                    .any(|a| classify(a.ip) == AddrClass::LinkLocalV6)
        }
        AddrClass::PrivateV4 | AddrClass::LinkLocalV4 | AddrClass::UniqueLocalV6 => {
            on.kind == Kind::Physical && on.addrs.iter().any(|a| in_subnet(ip, a))
        }
    }
}

/// The LAN rule against a whole interface list: true if any physical interface allows `remote`.
pub fn allowed_any(remote: SocketAddr, ifaces: &[Interface], policy: &Policy) -> bool {
    ifaces.iter().any(|i| allowed_on(remote, i, policy))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ifaces::{IfAddr, Interface, Kind};

    fn iface(kind: Kind, addrs: &[(&str, u8)]) -> Interface {
        Interface {
            name: "eth0".into(),
            index: 7,
            description: String::new(),
            kind,
            up: true,
            addrs: addrs
                .iter()
                .map(|(a, p)| IfAddr {
                    ip: a.parse().unwrap(),
                    prefix: *p,
                })
                .collect(),
        }
    }

    fn sa(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn classes() {
        let cases = [
            ("10.1.2.3", AddrClass::PrivateV4),
            ("172.16.0.1", AddrClass::PrivateV4),
            ("172.31.255.255", AddrClass::PrivateV4),
            ("172.32.0.1", AddrClass::NotLan),
            ("192.168.1.1", AddrClass::PrivateV4),
            ("169.254.3.4", AddrClass::LinkLocalV4),
            ("100.64.0.1", AddrClass::NotLan),
            ("100.101.102.103", AddrClass::NotLan),
            ("8.8.8.8", AddrClass::NotLan),
            ("224.0.0.251", AddrClass::NotLan),
            ("0.0.0.0", AddrClass::NotLan),
            ("255.255.255.255", AddrClass::NotLan),
            ("127.0.0.1", AddrClass::Loopback),
            ("::1", AddrClass::Loopback),
            ("fe80::1", AddrClass::LinkLocalV6),
            ("febf::1", AddrClass::LinkLocalV6),
            ("fec0::1", AddrClass::NotLan),
            ("fd12:3456::1", AddrClass::UniqueLocalV6),
            ("fc00::1", AddrClass::UniqueLocalV6),
            ("2001:db8::1", AddrClass::NotLan),
            ("2a02:1234::1", AddrClass::NotLan),
            ("ff02::fb", AddrClass::NotLan),
            ("::", AddrClass::NotLan),
            ("::ffff:192.168.1.5", AddrClass::PrivateV4),
            ("::ffff:8.8.8.8", AddrClass::NotLan),
        ];
        for (a, c) in cases {
            assert_eq!(classify(a.parse().unwrap()), c, "{a}");
        }
    }

    #[test]
    fn private_v4_must_be_on_link() {
        let eth = iface(Kind::Physical, &[("192.168.1.10", 24)]);
        let p = Policy::default();
        assert!(allowed_on(sa("192.168.1.20:5000"), &eth, &p));
        assert!(allowed_on(sa("[::ffff:192.168.1.20]:5000"), &eth, &p));
        // Private, but another subnet (could be routed from far away).
        assert!(!allowed_on(sa("192.168.2.20:5000"), &eth, &p));
        assert!(!allowed_on(sa("10.0.0.5:5000"), &eth, &p));
        // Port 0 is never a real peer.
        assert!(!allowed_on(sa("192.168.1.20:0"), &eth, &p));
    }

    #[test]
    fn public_addresses_always_fail() {
        // Even if the interface itself has a public address and the peer is "on link".
        let eth = iface(
            Kind::Physical,
            &[
                ("203.0.113.10", 24),
                ("2a02:1234::10", 64),
                ("fe80::10", 64),
            ],
        );
        let p = Policy {
            allow_loopback: true,
        };
        assert!(!allowed_on(sa("203.0.113.20:5000"), &eth, &p));
        assert!(!allowed_on(sa("[2a02:1234::20]:5000"), &eth, &p));
        assert!(!allowed_on(sa("[2a02:1234::20%7]:5000"), &eth, &p));
    }

    #[test]
    fn cgnat_and_tailscale_fail() {
        let ts = iface(Kind::Vpn, &[("100.100.1.2", 10)]);
        let eth = iface(Kind::Physical, &[("100.64.0.2", 10)]);
        let p = Policy::default();
        assert!(!allowed_on(sa("100.100.1.3:5000"), &ts, &p));
        assert!(!allowed_on(sa("100.64.0.3:5000"), &eth, &p));
    }

    #[test]
    fn non_physical_interfaces_fail() {
        for k in [Kind::Vpn, Kind::Virtual, Kind::Other, Kind::Loopback] {
            let i = iface(k, &[("10.8.0.2", 24), ("fe80::2", 64), ("fd00::2", 64)]);
            let p = Policy::default();
            assert!(!allowed_on(sa("10.8.0.3:5000"), &i, &p), "{k:?}");
            assert!(!allowed_on(sa("[fe80::3]:5000"), &i, &p), "{k:?}");
            assert!(!allowed_on(sa("[fd00::3]:5000"), &i, &p), "{k:?}");
        }
    }

    #[test]
    fn link_local_v6() {
        let eth = iface(Kind::Physical, &[("192.168.1.10", 24), ("fe80::10", 64)]);
        let p = Policy::default();
        assert!(allowed_on(sa("[fe80::20]:5000"), &eth, &p));
        assert!(allowed_on(sa("[fe80::20%7]:5000"), &eth, &p));
        // Scope of another interface.
        assert!(!allowed_on(sa("[fe80::20%9]:5000"), &eth, &p));
        // No IPv6 on this interface.
        let v4only = iface(Kind::Physical, &[("192.168.1.10", 24)]);
        assert!(!allowed_on(sa("[fe80::20]:5000"), &v4only, &p));
    }

    #[test]
    fn unique_local_v6_must_be_on_link() {
        let eth = iface(Kind::Physical, &[("fd12:3456:789a:1::10", 64)]);
        let p = Policy::default();
        assert!(allowed_on(sa("[fd12:3456:789a:1::20]:5000"), &eth, &p));
        assert!(!allowed_on(sa("[fd12:3456:789a:2::20]:5000"), &eth, &p));
    }

    #[test]
    fn loopback_only_with_policy() {
        let lo = iface(Kind::Loopback, &[("127.0.0.1", 8), ("::1", 128)]);
        assert!(!allowed_on(sa("127.0.0.1:5000"), &lo, &Policy::default()));
        let p = Policy {
            allow_loopback: true,
        };
        assert!(allowed_on(sa("127.0.0.1:5000"), &lo, &p));
        assert!(allowed_on(sa("[::1]:5000"), &lo, &p));
        // The subnet still counts: a /32 loopback "LAN" refuses its neighbour.
        let narrow = iface(Kind::Loopback, &[("127.0.0.1", 32)]);
        assert!(!allowed_on(sa("127.0.0.2:5000"), &narrow, &p));
    }

    #[test]
    fn any_interface() {
        let list = vec![
            iface(Kind::Vpn, &[("10.64.0.2", 32)]),
            iface(Kind::Physical, &[("192.168.1.10", 24)]),
        ];
        let p = Policy::default();
        assert!(allowed_any(sa("192.168.1.30:1"), &list, &p));
        assert!(!allowed_any(sa("10.64.0.1:1"), &list, &p));
    }

    #[test]
    fn subnet_edges() {
        let a = IfAddr {
            ip: "10.0.0.1".parse().unwrap(),
            prefix: 0,
        };
        assert!(!in_subnet("10.0.0.2".parse().unwrap(), &a));
        let a = IfAddr {
            ip: "10.0.0.1".parse().unwrap(),
            prefix: 32,
        };
        assert!(in_subnet("10.0.0.1".parse().unwrap(), &a));
        assert!(!in_subnet("10.0.0.2".parse().unwrap(), &a));
        assert!(!in_subnet("fe80::1".parse().unwrap(), &a));
    }

    proptest::proptest! {
        #[test]
        fn global_v4_never_passes(a in proptest::prelude::any::<u32>(), port in 1u16..) {
            let ip = Ipv4Addr::from(a);
            let eth = iface(Kind::Physical, &[(&ip.to_string(), 1)]);
            let p = Policy { allow_loopback: true };
            let ok = allowed_on(SocketAddr::new(IpAddr::V4(ip), port), &eth, &p);
            if ok {
                proptest::prop_assert!(ip.is_private() || ip.is_link_local() || ip.is_loopback());
            }
        }

        #[test]
        fn global_v6_never_passes(a in proptest::prelude::any::<u128>(), port in 1u16..) {
            let ip = Ipv6Addr::from(a);
            let eth = iface(Kind::Physical, &[(&ip.to_string(), 1)]);
            let p = Policy { allow_loopback: true };
            let ok = allowed_on(SocketAddr::new(IpAddr::V6(ip), port), &eth, &p);
            if ok {
                let s0 = ip.segments()[0];
                let mapped_private = ip.to_ipv4_mapped().is_some_and(|v| v.is_private() || v.is_link_local() || v.is_loopback());
                proptest::prop_assert!(s0 & 0xffc0 == 0xfe80 || s0 & 0xfe00 == 0xfc00 || ip.is_loopback() || mapped_private);
            }
        }
    }
}
