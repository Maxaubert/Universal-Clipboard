//! Interface detection against recorded adapter lists (plan 7, M2 accept: "Interface detection
//! has unit tests with recorded adapter lists"). Each fixture is the `--raw` output of
//! `cargo run -p uc-net --example ifaces` on a real machine, with addresses and adapter GUIDs
//! replaced by documentation values.

use std::path::Path;

use uc_net::ifaces::{self, ClassifyOptions, Interface, Kind, Os, RawIface};

fn load(name: &str, os: Os) -> Vec<Interface> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let text = std::fs::read_to_string(&p).expect("fixture");
    let raw: Vec<RawIface> = serde_json::from_str(&text).expect("fixture json");
    ifaces::classify_all(os, &raw, &ClassifyOptions::default())
}

fn kind_of(list: &[Interface], pred: impl Fn(&Interface) -> bool) -> Vec<Kind> {
    list.iter().filter(|i| pred(i)).map(|i| i.kind).collect()
}

fn usable_names(list: &[Interface]) -> Vec<String> {
    ifaces::usable(list)
        .into_iter()
        .map(|i| i.description.clone() + &i.name)
        .collect()
}

#[test]
fn windows_pc_with_hyperv_wsl_openvpn_surfshark_bluetooth() {
    // Max's PC, 2026-11-14: Realtek 5GbE (up), three MediaTek Wi-Fi 7 adapters (down),
    // two OpenVPN DCO adapters (down), Bluetooth PAN, two Hyper-V vEthernet switches (WSL and
    // Default Switch).
    let l = load("ifaces-windows-pc.json", Os::Windows);
    let usable = ifaces::usable(&l);
    assert_eq!(usable.len(), 1, "{:?}", usable_names(&l));
    assert!(usable[0].description.contains("Realtek"));
    assert_eq!(
        kind_of(&l, |i| i.description.contains("vEthernet")),
        vec![Kind::Virtual, Kind::Virtual]
    );
    assert_eq!(
        kind_of(&l, |i| i.description.contains("OpenVPN")),
        vec![Kind::Vpn, Kind::Vpn]
    );
    assert_eq!(
        kind_of(&l, |i| i.description.contains("Bluetooth")),
        vec![Kind::Virtual]
    );
    assert_eq!(
        kind_of(&l, |i| i.description.contains("MediaTek")),
        vec![Kind::Physical; 3]
    );
    assert_eq!(kind_of(&l, |i| i.index == 1), vec![Kind::Loopback]);
    assert!(!ifaces::vpn_up(&l), "the VPN adapters are down");
}

#[test]
fn macos_mini_with_system_utuns_and_thunderbolt_bridge() {
    // Max's Mac mini (macOS 27.2), 2026-11-14: en0 Ethernet and en1 Wi-Fi up, Thunderbolt
    // ports en2/en3/en7 in bridge0, eight utun system tunnels with only link-local addresses,
    // awdl0, llw0, anpi*, ap1, nan0, gif0, stf0.
    let l = load("ifaces-macos-mini.json", Os::Apple);
    let mut names: Vec<String> = ifaces::usable(&l).into_iter().map(|i| i.name).collect();
    names.sort();
    assert_eq!(names, vec!["en0", "en1"]);
    for n in ["bridge0", "awdl0", "llw0", "anpi0", "ap1"] {
        assert_eq!(kind_of(&l, |i| i.name == n), vec![Kind::Virtual], "{n}");
    }
    assert!(
        l.iter()
            .filter(|i| i.name.starts_with("utun"))
            .all(|i| i.kind == Kind::Vpn)
    );
    assert_eq!(kind_of(&l, |i| i.name == "lo0"), vec![Kind::Loopback]);
    // System utuns with only fe80:: are not "a VPN is on" (no false VPN alarm on every Mac).
    assert!(!ifaces::vpn_up(&l));
}

#[test]
fn linux_wsl() {
    let l = load("ifaces-linux-wsl.json", Os::Linux);
    let names: Vec<String> = ifaces::usable(&l).into_iter().map(|i| i.name).collect();
    assert_eq!(names, vec!["eth0"]);
    assert!(!ifaces::vpn_up(&l));
}

#[test]
fn linux_netns_with_wireguard_and_bridge() {
    // Recorded inside a WSL network namespace of the 6.3 lab (tools/netsim): a veth "LAN"
    // link, a WireGuard tunnel with an address, a bridge and a dummy.
    let l = load("ifaces-linux-netns-wg.json", Os::Linux);
    assert_eq!(kind_of(&l, |i| i.name == "wg0"), vec![Kind::Vpn]);
    assert!(
        ifaces::vpn_up(&l),
        "a WireGuard tunnel with an address is a VPN that is on"
    );
    assert!(
        ifaces::usable(&l).is_empty(),
        "veth is not physical unless the test forces it"
    );
    let forced = ifaces::classify_all(
        Os::Linux,
        &serde_json::from_str::<Vec<RawIface>>(
            &std::fs::read_to_string(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/ifaces-linux-netns-wg.json"),
            )
            .unwrap(),
        )
        .unwrap(),
        &ClassifyOptions {
            force_physical: vec!["lan0".into()],
        },
    );
    let names: Vec<String> = ifaces::usable(&forced)
        .into_iter()
        .map(|i| i.name)
        .collect();
    assert_eq!(names, vec!["lan0"]);
}
