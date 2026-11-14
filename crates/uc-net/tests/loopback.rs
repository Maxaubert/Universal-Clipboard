//! Multi-endpoint tests on loopback (plan 6.2 for the network layer). Every node is a full
//! [`uc_net::Net`] with real QUIC sockets on 127.0.0.x, fake mDNS, and (where it says so) a
//! `uc-core` sync engine with a fake clipboard. Nothing touches a real clipboard or the LAN.

mod common;

use std::net::SocketAddr;
use std::time::Duration;

use common::*;
use uc_net::endpoint::{DialError, LanEndpoint};
use uc_net::{ALPN_SYNC, AddressBinder, FakeLan, NoPath, PathState, Policy};

const T: Duration = Duration::from_secs(10);

fn addr(n: &Node) -> SocketAddr {
    SocketAddr::new(n.ips()[0], n.net_port())
}

trait PortExt {
    fn net_port(&self) -> u16;
}

impl PortExt for Node {
    fn net_port(&self) -> u16 {
        let rt = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| rt.block_on(self.net.status()))
            .map(|s| s.port)
            .unwrap_or(0)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_nodes_find_each_other_by_mdns_and_exchange_frames() {
    let lan = FakeLan::new();
    let mut a = Node::start(1, &ip(1), 0, &lan);
    let mut b = Node::start(2, &ip(2), 0, &lan);
    a.join(&[1, 2]);
    b.join(&[1, 2]);
    let took = until(&mut [&mut a, &mut b], T, |n| {
        n[0].links.contains_key(&id(2)) && n[1].links.contains_key(&id(1))
    })
    .await
    .expect("connected");
    assert!(
        took < Duration::from_secs(1),
        "same-LAN connect took {took:?}"
    );

    a.links[&id(2)].send(b"hello".to_vec()).unwrap();
    until(&mut [&mut a, &mut b], T, |n| {
        n[1].frames
            .iter()
            .any(|(p, f)| *p == id(1) && f == b"hello")
    })
    .await
    .expect("frame arrived");
    assert_eq!(
        a.paths.get(&id(2)),
        Some(&PathState::Lan),
        "path state is LAN"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_engines_converge_over_quic() {
    let lan = FakeLan::new();
    let mut a = Node::start(1, &ip(1), 0, &lan).with_engine();
    let mut b = Node::start(2, &ip(2), 0, &lan).with_engine();
    let mut c = Node::start(3, &ip(3), 0, &lan).with_engine();
    for n in [&a, &b, &c] {
        n.join(&[1, 2, 3]);
    }
    until(&mut [&mut a, &mut b, &mut c], T, |n| {
        n.iter().all(|x| x.links.len() == 2)
    })
    .await
    .expect("full mesh");
    settle(&mut [&mut a, &mut b, &mut c], Duration::from_millis(300)).await;

    a.copy("from a");
    until(&mut [&mut a, &mut b, &mut c], T, |n| {
        n[1].clip.text().as_deref() == Some("from a")
            && n[2].clip.text().as_deref() == Some("from a")
    })
    .await
    .expect("a's copy reached b and c");

    c.copy("from c");
    until(&mut [&mut a, &mut b, &mut c], T, |n| {
        n[0].clip.text().as_deref() == Some("from c")
            && n[1].clip.text().as_deref() == Some("from c")
    })
    .await
    .expect("c's copy reached a and b");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unknown_key_gets_no_link() {
    let lan = FakeLan::new();
    let ifaces = std::sync::Arc::new(std::sync::Mutex::new(vec![lo(&ip(1), 8)]));
    let mut a = Node::start_cfg(1, 0, &lan, ifaces, |c| c.probation = false);
    let mut x = Node::start(9, &ip(9), 0, &lan);
    a.join(&[1]); // A knows nobody.
    x.join(&[1, 9]); // X wants A.
    x.net.add_addr(id(1), addr(&a));
    settle(&mut [&mut a, &mut x], Duration::from_secs(2)).await;
    assert_eq!(a.ups, 0, "A must not open a link to an unknown key");
    assert!(a.links.is_empty());
    assert!(x.links.is_empty(), "X has no usable link");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[cfg_attr(
    target_os = "macos",
    ignore = "macOS has only 127.0.0.1 on lo0; Windows and Linux CI run it"
)]
async fn non_lan_source_is_dropped_before_the_handshake() {
    // Server on 127.0.0.1/32: 127.0.0.2 is "not on link" for it.
    let server_if = vec![lo("127.0.0.1", 32)];
    let client_if = vec![lo("127.0.0.2", 8)];
    let p = Policy {
        allow_loopback: true,
    };
    let alpns = vec![ALPN_SYNC.to_vec()];
    let (server, mut accepted) =
        LanEndpoint::bind(key(1), &server_if, 0, p, timing(), &AddressBinder, &alpns).unwrap();
    let (client, _rx) =
        LanEndpoint::bind(key(2), &client_if, 0, p, timing(), &AddressBinder, &alpns).unwrap();
    let target: SocketAddr = format!("127.0.0.1:{}", server.port()).parse().unwrap();
    // The client's own rule allows the target (it is on its /8)...
    let r = client.dial(id(1), target, ALPN_SYNC).await;
    // ...but the server ignores the first packet, so the dial times out.
    assert!(matches!(r, Err(DialError::Timeout)), "{r:?}");
    assert!(server.refused() >= 1, "the server counted the drop");
    assert!(accepted.try_recv().is_err(), "nothing was accepted");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dials_outside_the_lan_are_refused_at_once() {
    let p = Policy {
        allow_loopback: true,
    };
    let (ep, _rx) = LanEndpoint::bind(
        key(1),
        &[lo("127.0.0.1", 8)],
        0,
        p,
        timing(),
        &AddressBinder,
        &[ALPN_SYNC.to_vec()],
    )
    .unwrap();
    for a in [
        "8.8.8.8:4000",
        "[2a02:1234::1]:4000",
        "192.168.77.1:4000",
        "100.64.0.1:4000",
    ] {
        let r = ep.dial(id(2), a.parse().unwrap(), ALPN_SYNC).await;
        assert!(matches!(r, Err(DialError::NotLan(_))), "{a}: {r:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wrong_key_is_refused_by_the_dialer() {
    let p = Policy {
        allow_loopback: true,
    };
    let alpns = vec![ALPN_SYNC.to_vec()];
    let (server, _a) = LanEndpoint::bind(
        key(1),
        &[lo("127.0.0.1", 8)],
        0,
        p,
        timing(),
        &AddressBinder,
        &alpns,
    )
    .unwrap();
    let (client, _b) = LanEndpoint::bind(
        key(2),
        &[lo(&ip(2), 8)],
        0,
        p,
        timing(),
        &AddressBinder,
        &alpns,
    )
    .unwrap();
    let target: SocketAddr = format!("127.0.0.1:{}", server.port()).parse().unwrap();
    // Expect device 3 at device 1's address: the TLS check fails.
    let r = client.dial(id(3), target, ALPN_SYNC).await;
    assert!(r.is_err(), "{r:?}");
    // The right key works.
    let ok = client.dial(id(1), target, ALPN_SYNC).await.expect("dial");
    assert_eq!(ok.peer, id(1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn binds_only_lan_addresses_never_a_wildcard() {
    let p = Policy::default();
    let eth = uc_net::Interface {
        name: "eth-test".into(),
        index: 1,
        description: String::new(),
        kind: uc_net::Kind::Physical,
        up: true,
        addrs: vec![
            uc_net::ifaces::IfAddr {
                ip: "203.0.113.5".parse().unwrap(),
                prefix: 24,
            },
            uc_net::ifaces::IfAddr {
                ip: "2001:db8::5".parse().unwrap(),
                prefix: 64,
            },
        ],
    };
    let (ep, _rx) = LanEndpoint::bind(
        key(1),
        &[eth],
        0,
        p,
        timing(),
        &AddressBinder,
        &[ALPN_SYNC.to_vec()],
    )
    .unwrap();
    assert!(
        ep.local_addrs().is_empty(),
        "public addresses are never bound: {:?}",
        ep.local_addrs()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gossip_lets_two_strangers_find_each_other_through_a_third() {
    // A and C never see each other by mDNS (both blocked) and know no address of each other.
    // Both know B. B passes on their signed announcements, then A and C connect directly.
    let lan = FakeLan::new();
    let mut a = Node::start(1, &ip(1), 0, &lan);
    let mut b = Node::start(2, &ip(2), 0, &lan);
    let mut c = Node::start(3, &ip(3), 0, &lan);
    a.mdns.set_blocked(true);
    c.mdns.set_blocked(true);
    b.mdns.set_blocked(true);
    for n in [&a, &b, &c] {
        n.join(&[1, 2, 3]);
    }
    let ab = addr(&b);
    a.net.add_addr(id(2), ab);
    c.net.add_addr(id(2), ab);
    let took = until(&mut [&mut a, &mut b, &mut c], T, |n| {
        n[0].links.contains_key(&id(3)) && n[2].links.contains_key(&id(1))
    })
    .await
    .expect("A and C connected through gossip");
    assert!(took < Duration::from_secs(5), "took {took:?}");
    let st = a.net.status().await.unwrap();
    assert!(
        st.book
            .get(&id(3))
            .is_some_and(|e| e.announcement.is_some()),
        "A stored C's signed announcement"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remembered_address_works_without_multicast() {
    let lan = FakeLan::new();
    let mut a = Node::start(1, &ip(1), 0, &lan);
    let mut b = Node::start(2, &ip(2), 0, &lan);
    a.mdns.set_blocked(true);
    b.mdns.set_blocked(true);
    a.join(&[1, 2]);
    b.join(&[1, 2]);
    // Never seen: no path, and the reason is "not found" (not a VPN false alarm).
    until(&mut [&mut a, &mut b], T, |n| {
        n[0].paths.get(&id(2)) == Some(&PathState::NoPath(NoPath::NotFound))
    })
    .await
    .expect("not found");
    a.net.add_addr(id(2), addr(&b));
    until(&mut [&mut a, &mut b], T, |n| {
        n[0].links.contains_key(&id(2))
    })
    .await
    .expect("found by remembered address");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[cfg_attr(
    target_os = "macos",
    ignore = "macOS has only 127.0.0.1 on lo0; Windows and Linux CI run it"
)]
async fn address_change_reconnects_quickly() {
    let lan = FakeLan::new();
    let mut a = Node::start(1, &ip(1), 0, &lan);
    let mut b = Node::start(2, &ip(2), 0, &lan);
    a.join(&[1, 2]);
    b.join(&[1, 2]);
    until(&mut [&mut a, &mut b], T, |n| {
        n[0].links.contains_key(&id(2))
    })
    .await
    .expect("up");
    // B moves to another address.
    *b.ifaces.lock().unwrap() = vec![lo("127.0.0.22", 8)];
    let took = until(&mut [&mut a, &mut b], T, |n| {
        n[0].links
            .get(&id(2))
            .is_some_and(|l| l.remote.ip().to_string() == "127.0.0.22")
    })
    .await
    .expect("reconnected on the new address");
    assert!(took < Duration::from_secs(5), "took {took:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn path_states_and_the_vpn_rule() {
    let lan = FakeLan::new();
    let mut a = Node::start(1, &ip(1), 0, &lan);
    let mut b = Node::start(2, &ip(2), 0, &lan);
    a.join(&[1, 2]);
    b.join(&[1, 2]);
    until(&mut [&mut a, &mut b], T, |n| {
        n[0].links.contains_key(&id(2))
    })
    .await
    .expect("up");
    let b_port = b.net_port();
    b.net.shutdown();
    drop(b);
    // No VPN: unreachable after the delay, never "blocked by VPN".
    until(&mut [&mut a], T, |n| {
        n[0].paths.get(&id(2)) == Some(&PathState::NoPath(NoPath::Unreachable))
    })
    .await
    .expect("unreachable");
    // A VPN comes up on A: now it is the VPN rule, a "Needs attention" reason.
    a.ifaces.lock().unwrap().push(vpn_iface());
    until(&mut [&mut a], T, |n| {
        n[0].paths.get(&id(2)) == Some(&PathState::NoPath(NoPath::VpnBlocksLan))
    })
    .await
    .expect("VPN rule fired");
    assert_eq!(
        a.paths[&id(2)].attention(),
        Some(uc_net::Attention::VpnBlocksLan)
    );
    // B comes back on the same address and port (an agent restart): A heals.
    let mut b = Node::start(2, &ip(2), b_port, &lan);
    b.join(&[1, 2]);
    let took = until(&mut [&mut a, &mut b], T, |n| {
        n[0].paths.get(&id(2)) == Some(&PathState::Lan)
    })
    .await
    .expect("healed");
    assert!(took < Duration::from_secs(5), "took {took:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn simultaneous_dials_settle_on_one_link() {
    let lan = FakeLan::new();
    let mut a = Node::start(1, &ip(1), 0, &lan);
    let mut b = Node::start(2, &ip(2), 0, &lan);
    // Both know each other's address and dial at the same moment.
    a.mdns.set_blocked(true);
    b.mdns.set_blocked(true);
    let (pa, pb) = (addr(&a), addr(&b));
    a.join(&[1, 2]);
    b.join(&[1, 2]);
    a.net.add_addr(id(2), pb);
    b.net.add_addr(id(1), pa);
    until(&mut [&mut a, &mut b], T, |n| {
        n[0].links.contains_key(&id(2)) && n[1].links.contains_key(&id(1))
    })
    .await
    .expect("up");
    settle(&mut [&mut a, &mut b], Duration::from_secs(2)).await;
    assert!(a.ups <= 2 && b.ups <= 2, "no flapping: {} {}", a.ups, b.ups);
    // Both sides hold the same connection: frames flow both ways.
    a.links[&id(2)].send(b"x".to_vec()).unwrap();
    b.links[&id(1)].send(b"y".to_vec()).unwrap();
    until(&mut [&mut a, &mut b], T, |n| {
        n[0].frames.iter().any(|f| f.1 == b"y") && n[1].frames.iter().any(|f| f.1 == b"x")
    })
    .await
    .expect("one shared link");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wake_keeps_working_links_and_redials() {
    let lan = FakeLan::new();
    let mut a = Node::start(1, &ip(1), 0, &lan);
    let mut b = Node::start(2, &ip(2), 0, &lan);
    a.join(&[1, 2]);
    b.join(&[1, 2]);
    until(&mut [&mut a, &mut b], T, |n| {
        n[0].links.contains_key(&id(2))
    })
    .await
    .expect("up");
    a.net.wake();
    b.net.wake();
    settle(&mut [&mut a, &mut b], Duration::from_millis(500)).await;
    until(&mut [&mut a, &mut b], T, |n| {
        n[0].links.contains_key(&id(2))
    })
    .await
    .expect("still up after wake");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn taken_port_gives_a_new_port() {
    let blocker = std::net::UdpSocket::bind(format!("{}:0", ip(5))).unwrap();
    let taken = blocker.local_addr().unwrap().port();
    let lan = FakeLan::new();
    let mut a = Node::start(1, &ip(5), taken, &lan);
    until(&mut [&mut a], T, |n| !n[0].port_changes.is_empty())
        .await
        .expect("port change reported");
    assert_ne!(a.port_changes[0], taken);
    assert_eq!(a.net_port(), a.port_changes[0]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pairing_connection_is_handed_out_with_matching_exporter() {
    let lan = FakeLan::new();
    let mut a = Node::start(1, &ip(1), 0, &lan);
    let b = Node::start(2, &ip(2), 0, &lan);
    let target = addr(&a);
    let conn = b.net.dial_pair(id(1), target).await.expect("pair dial");
    assert_eq!(conn.alpn, uc_net::ALPN_PAIR);
    until(&mut [&mut a], T, |n| !n[0].pairing.is_empty())
        .await
        .expect("A got the pairing connection");
    let inv = &a.pairing[0];
    assert_eq!(inv.peer, id(2));
    let mut x = [0u8; 32];
    let mut y = [0u8; 32];
    conn.conn
        .export_keying_material(&mut x, b"uclip-pair-v1", b"")
        .unwrap();
    inv.conn
        .export_keying_material(&mut y, b"uclip-pair-v1", b"")
        .unwrap();
    assert_eq!(x, y, "both sides see the same TLS exporter (protocol 4.5)");
}

/// Protocol 5.9 admission hook: an unknown key gets one probation link that carries no
/// addresses, is never dialed back, ends when the agent says so or after 10 s, and becomes a
/// normal link when membership sync makes the key a member. A removed key gets nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unknown_key_gets_a_probation_link_only() {
    let lan = FakeLan::new();
    let mut a = Node::start(1, &ip(1), 0, &lan);
    let mut x = Node::start(9, &ip(9), 0, &lan);
    a.join(&[1]); // A knows nobody.
    x.join(&[1, 9]); // X wants A (it was added while A was away).
    x.net.add_addr(id(1), addr(&a));
    let up = until(&mut [&mut a, &mut x], Duration::from_secs(5), |n| {
        n[0].links.contains_key(&id(9))
    })
    .await;
    assert!(up.is_some(), "A gives X a probation link");
    // No address went to the probation peer, and A has no path state for it.
    let st = a.net.status().await.unwrap();
    assert!(!st.paths.contains_key(&id(9)));
    assert!(
        !x.frames.iter().any(|(_, f)| matches!(
            uc_core::msg::decode(f),
            Ok(Some(uc_core::Message::AddrUpdate { .. }))
        )),
        "no address on a probation link"
    );
    // The agent decides: not a member. The link closes.
    a.net.end_probation(id(9));
    let gone = until(&mut [&mut a, &mut x], Duration::from_secs(5), |n| {
        !n[0].links.contains_key(&id(9))
    })
    .await;
    assert!(gone.is_some(), "end_probation closes the link");

    // A removed key gets no link at all.
    a.net.set_removed(vec![id(9)]);
    let ups = a.ups;
    settle(&mut [&mut a, &mut x], Duration::from_secs(2)).await;
    assert_eq!(a.ups, ups, "a removed key never gets a probation link");

    // Not removed any more (a new test group): probation, then membership makes it normal.
    a.net.set_removed(vec![]);
    let up = until(&mut [&mut a, &mut x], Duration::from_secs(5), |n| {
        n[0].links.contains_key(&id(9))
    })
    .await;
    assert!(up.is_some());
    a.join(&[1, 9]);
    settle(&mut [&mut a, &mut x], Duration::from_millis(500)).await;
    let st = a.net.status().await.unwrap();
    assert!(st.paths.contains_key(&id(9)), "a member now");
    assert!(a.links.contains_key(&id(9)), "the same link stays up");
}
