//! Fuzz-style property tests of every parser: random and mutated input never panics, and valid
//! input round-trips.

mod common;

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use common::*;
use curve25519_dalek::constants::EIGHT_TORSION;
use proptest::prelude::*;
use uc_keys::DeviceKey;
use uc_pair::chain::{Body, Change, Payload};
use uc_pair::frame::{Confirm, Decoder, Hello, Reply, State};
use uc_pair::lan::{Iface, is_lan_peer};
use uc_pair::pake::{SIDE_A, SIDE_B, check_peer_msg};
use uc_pair::qr::QrPayload;

fn addr() -> impl Strategy<Value = IpAddr> {
    prop_oneof![
        any::<[u8; 4]>().prop_map(|b| IpAddr::V4(Ipv4Addr::from(b))),
        any::<[u8; 16]>().prop_map(|b| IpAddr::V6(Ipv6Addr::from(b))),
    ]
}

fn name() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 éøå'-]{1,20}".prop_filter("1 to 64 bytes", |s| !s.is_empty() && s.len() <= 64)
}

fn payload() -> impl Strategy<Value = Payload> {
    prop_oneof![
        (any::<[u8; 16]>(), 0u8..=7, name(), any::<[u8; 32]>()).prop_map(|(n, t, s, c)| {
            Payload::Genesis {
                group_nonce: n,
                device_type: t,
                name: s,
                secret_commit: c,
            }
        }),
        (any::<[u8; 32]>(), 0u8..=7, name()).prop_map(|(d, t, n)| Payload::Add {
            device: d,
            device_type: t,
            name: n
        }),
        any::<[u8; 32]>().prop_map(|d| Payload::Remove { device: d }),
        (any::<[u8; 32]>(), name()).prop_map(|(d, n)| Payload::Rename { device: d, name: n }),
        (any::<u32>(), any::<[u8; 32]>()).prop_map(|(e, c)| Payload::Rotate {
            epoch: e,
            secret_commit: c
        }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    #[test]
    fn random_bytes_never_panic(b in proptest::collection::vec(any::<u8>(), 0..400)) {
        let _ = Body::parse(&b);
        let _ = Change::parse(&b);
        let _ = QrPayload::from_bytes(&b);
        let _ = State::parse(&b);
        let _ = Confirm::parse(&b);
        let _ = Hello::parse(&b);
        let _ = Reply::parse(&b);
        let _ = check_peer_msg(&b, SIDE_A);
        let _ = check_peer_msg(&b, SIDE_B);
        let mut d = Decoder::new();
        d.push(&b);
        while let Ok(Some(_)) = d.next_frame() {}
    }

    #[test]
    fn random_text_never_panics(s in "UCLIP1:[A-Z2-7a-z=0-9]{0,120}") {
        let _ = QrPayload::parse(&s);
    }

    #[test]
    fn qr_roundtrip_and_mutation(
        key in any::<[u8; 32]>(),
        secret in any::<[u8; 16]>(),
        port in any::<u16>(),
        addrs in proptest::collection::vec(addr(), 1..=4),
        pos in any::<usize>(),
        ch in "[A-Z2-7a-z=!]",
    ) {
        let p = QrPayload { inviter_key: key, qr_secret: secret, port, addrs };
        let text = p.to_text();
        let q = QrPayload::parse(&text).unwrap();
        prop_assert_eq!(q.to_bytes(), p.to_bytes());
        prop_assert!(text.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b':'));
        // Change one character: refused, or a different payload; never a panic.
        let mut t: Vec<char> = text.chars().collect();
        let i = 7 + pos % (t.len() - 7);
        t[i] = ch.chars().next().unwrap();
        let t: String = t.into_iter().collect();
        if let Ok(m) = QrPayload::parse(&t) {
            prop_assert!(t == text || m.to_bytes() != p.to_bytes());
        }
    }

    #[test]
    fn body_roundtrip(
        signer in any::<[u8; 32]>(),
        time in any::<u64>(),
        mut parents in proptest::collection::btree_set(any::<[u8; 32]>(), 1..=8),
        payload in payload(),
    ) {
        let parents: Vec<[u8; 32]> = if matches!(payload, Payload::Genesis { .. }) {
            parents.clear();
            vec![]
        } else {
            parents.into_iter().collect()
        };
        let b = Body { signer, time, parents, payload };
        let bytes = b.encode();
        prop_assert_eq!(Body::parse(&bytes).unwrap(), b);
        // Any shorter or longer body is refused.
        prop_assert!(Body::parse(&bytes[..bytes.len() - 1]).is_err());
        let mut longer = bytes.clone();
        longer.push(0);
        prop_assert!(Body::parse(&longer).is_err());
    }

    #[test]
    fn signed_change_mutations_refused(i in any::<usize>(), bit in 0u8..8) {
        let k = DeviceKey::from_seed(&arr32("mut"));
        let c = Change::sign(
            Body {
                signer: k.public(),
                time: 5,
                parents: vec![[1; 32]],
                payload: Payload::Remove { device: [2; 32] },
            },
            &k,
        );
        let mut b = c.to_bytes();
        prop_assert_eq!(Change::parse(&b).unwrap(), c);
        let i = i % b.len();
        b[i] ^= 1 << bit;
        prop_assert!(Change::parse(&b).is_err());
    }

    #[test]
    fn state_roundtrip(
        epoch in any::<u32>(),
        secret in any::<[u8; 32]>(),
        changes in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..200), 0..10),
        addr_book in proptest::collection::vec(any::<u8>(), 0..300),
    ) {
        let s = State { epoch, group_secret: secret, changes, addr_book };
        let f = s.encode().unwrap();
        let mut d = Decoder::new();
        d.push(&f);
        let fr = d.next_frame().unwrap().unwrap();
        let p = State::parse(&fr.payload).unwrap();
        prop_assert_eq!(&p, &s);
        prop_assert!(State::parse(&fr.payload[..fr.payload.len() - 1]).is_none());
    }

    #[test]
    fn lan_rule_never_panics(
        remote in addr(),
        scope in proptest::option::of(any::<u32>()),
        ifs in proptest::collection::vec((any::<u32>(), any::<bool>(), addr(), any::<u8>()), 0..4),
    ) {
        let ifaces: Vec<Iface> = ifs
            .into_iter()
            .map(|(index, physical, addr, prefix_len)| Iface { index, physical, addr, prefix_len })
            .collect();
        let ok = is_lan_peer(remote, scope, &ifaces);
        if ok {
            prop_assert!(ifaces.iter().any(|i| i.physical));
        }
    }
}

#[test]
fn low_order_points_refused() {
    for p in EIGHT_TORSION.iter() {
        for side in [SIDE_A, SIDE_B] {
            let mut m = vec![side];
            m.extend_from_slice(p.compress().as_bytes());
            assert!(!check_peer_msg(&m, side), "torsion point accepted");
        }
    }
    // A point that does not decompress.
    let mut bad = vec![SIDE_B];
    bad.extend_from_slice(&[0xff; 32]);
    let _ = check_peer_msg(&bad, SIDE_B);
    let mut not_on_curve = vec![SIDE_B, 2];
    not_on_curve.extend_from_slice(&[0; 31]);
    assert!(!check_peer_msg(&not_on_curve, SIDE_B));
}

#[test]
fn lan_rule_cases() {
    let ifs = vec![
        Iface {
            index: 4,
            physical: true,
            addr: "172.16.5.1".parse().unwrap(),
            prefix_len: 16,
        },
        Iface {
            index: 4,
            physical: true,
            addr: "169.254.3.3".parse().unwrap(),
            prefix_len: 16,
        },
        Iface {
            index: 6,
            physical: false,
            addr: "192.168.50.2".parse().unwrap(),
            prefix_len: 24,
        },
        Iface {
            index: 7,
            physical: true,
            addr: "100.64.0.2".parse().unwrap(),
            prefix_len: 10,
        },
    ];
    let ok = |s: &str| is_lan_peer(s.parse().unwrap(), None, &ifs);
    assert!(ok("172.16.200.9"));
    assert!(!ok("172.17.0.1"), "outside the interface subnet");
    assert!(ok("169.254.99.1"));
    assert!(!ok("192.168.50.7"), "only on a virtual interface");
    assert!(!ok("100.64.0.9"), "CGNAT is not a private range");
    assert!(!ok("127.0.0.1"));
    assert!(!ok("::1"));
    assert!(!ok("2001:db8::5"));
    assert!(!ok("fd00::5"), "no ULA prefix on any interface");
    assert!(!ok("fe80::5"), "link-local needs a scope");
}
