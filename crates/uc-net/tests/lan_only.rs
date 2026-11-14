//! "Never reachable from outside the local network" (plan 4.1): relay, address lookup (n0 DNS,
//! pkarr), hole punching and the port mapper (UPnP, NAT-PMP, PCP) are all OFF.
//!
//! They are off by construction: the transport is plain QUIC (`quinn`) with none of those
//! parts in it. This test makes that a checked fact:
//!
//! 1. No crate that implements any of them is anywhere in the workspace's `Cargo.lock` (so no
//!    crate of the app can switch one on by accident, now or after a dependency update).
//! 2. The crate's own statement ([`uc_net::LAN_ONLY`]) says off for each.
//! 3. The loopback tests check the runtime side: only LAN addresses are bound, a non-LAN
//!    source is dropped before the handshake, non-LAN dials are refused.

use std::path::Path;

/// Crates that would bring a relay, an address lookup service, hole punching or port mapping.
const FORBIDDEN: &[(&str, &str)] = &[
    (
        "iroh",
        "relay, n0 discovery, QUIC NAT traversal, port mapper",
    ),
    ("iroh-relay", "relay"),
    ("iroh-dns", "n0 DNS address lookup"),
    ("iroh-dns-server", "n0 DNS address lookup"),
    ("iroh-mdns-address-lookup", "iroh address lookup"),
    ("noq", "iroh's QUIC fork with NAT traversal"),
    ("noq-proto", "iroh's QUIC fork with NAT traversal"),
    ("portmapper", "UPnP, NAT-PMP, PCP"),
    ("igd", "UPnP port mapping"),
    ("igd-next", "UPnP port mapping"),
    ("natpmp", "NAT-PMP port mapping"),
    ("rust-natpmp", "NAT-PMP port mapping"),
    ("pcp", "PCP port mapping"),
    ("easy-upnp", "UPnP port mapping"),
    ("pkarr", "pkarr address lookup"),
    ("mainline", "DHT address lookup"),
    ("libp2p", "relay, hole punching, DHT"),
    ("stun", "STUN (public address discovery)"),
    ("stun-rs", "STUN (public address discovery)"),
    ("stunclient", "STUN (public address discovery)"),
    ("webrtc", "ICE, STUN, TURN"),
    ("turn", "TURN relay"),
    ("hickory-resolver", "DNS lookups"),
    ("trust-dns-resolver", "DNS lookups"),
    (
        "netwatch",
        "iroh network monitor (pulls the port mapper stack)",
    ),
];

fn lock_packages() -> Vec<String> {
    let lock = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock");
    let text = std::fs::read_to_string(&lock).expect("read Cargo.lock");
    text.lines()
        .filter_map(|l| l.strip_prefix("name = \""))
        .filter_map(|l| l.strip_suffix('"'))
        .map(str::to_string)
        .collect()
}

#[test]
fn no_relay_lookup_holepunch_or_portmap_crate_in_the_lockfile() {
    let pkgs = lock_packages();
    assert!(
        pkgs.iter().any(|p| p == "quinn"),
        "lockfile parse looks wrong"
    );
    let found: Vec<String> = FORBIDDEN
        .iter()
        .filter(|(n, _)| pkgs.iter().any(|p| p == n))
        .map(|(n, why)| format!("{n} ({why})"))
        .collect();
    assert!(
        found.is_empty(),
        "these crates must never be in the app: {found:?}"
    );
}

#[test]
fn each_internet_feature_is_off() {
    let f = uc_net::LAN_ONLY;
    assert!(!f.relay, "relay");
    assert!(!f.address_lookup, "address lookup (n0 DNS, pkarr)");
    assert!(!f.hole_punching, "hole punching / NAT traversal");
    assert!(!f.port_mapping, "port mapper (UPnP, NAT-PMP, PCP)");
    assert!(!f.wildcard_bind, "wildcard bind");
    assert!(!f.migration, "connection migration");
}

#[test]
fn source_has_no_internet_endpoints() {
    // A cheap guard against someone adding a STUN server, a relay URL or a DNS lookup by hand.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let words = [
        "stun.",
        "relay.",
        "n0.computer",
        "iroh.link",
        "dns.",
        "to_socket_addrs",
        "lookup_host",
        "239.255.255.250",
        ":5351",
        ":1900",
    ];
    for entry in std::fs::read_dir(dir).expect("src dir") {
        let p = entry.expect("entry").path();
        let text = std::fs::read_to_string(&p).expect("read");
        let code: String = text
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for w in words {
            assert!(!code.contains(w), "{} contains {w:?}", p.display());
        }
    }
}
