//! Physical interface detection (plan 3.5).
//!
//! The OS gives a list of raw interface records ([`RawIface`]). [`classify`] turns each into a
//! [`Kind`]. Only [`Kind::Physical`] interfaces carry sync traffic and mDNS; [`Kind::Vpn`]
//! interfaces only feed the "VPN is on" signal of the VPN-blocks-LAN rule.
//!
//! The classifier is a pure function, so it is tested with recorded adapter lists from each OS
//! (`tests/fixtures/ifaces-*.json`). Detection on the running machine ([`detect`]) reads the OS
//! with the `netdev` crate, plus `/sys/class/net` on Linux.
//!
//! | OS | Physical means |
//! |---|---|
//! | Windows | adapter type Ethernet or IEEE 802.11, up, and no tunnel, VPN or virtual name or description (WireGuard, Wintun, TAP, OpenVPN, Tailscale, vEthernet, Hyper-V virtual switch, VMware, VirtualBox host-only, Wi-Fi Direct, Bluetooth, Npcap) |
//! | macOS, iOS | `en*` with type Ethernet or Wi-Fi. `utun*`, `ipsec*`, `ppp*`, `bridge*`, `awdl*`, `llw*`, `anpi*`, `ap*` are never physical |
//! | Linux, Android | `/sys/class/net/<if>/device` exists, the link type is Ethernet (ARPHRD 1), and DEVTYPE is not bridge, wireguard, vlan or similar; no docker, veth, virbr, tailscale or wg name |
//!
//! **The host's own adapter on a virtual switch** (2026-11-14, Max's PC). When a physical NIC
//! is bound to a Hyper-V *external* switch, the NIC itself has no address any more; the host's
//! LAN address lives on `vEthernet (<switch>)`, a "Hyper-V Virtual Ethernet Adapter". The same
//! happens on Linux when the NIC is enslaved to a host bridge (`br0`, Proxmox `vmbr0`) or a
//! bond. Such an adapter counts as [`Kind::Physical`] when it has an IPv4 default gateway on a
//! private address inside its own private IPv4 subnet ([`lan_gateway`]). Internal switches
//! (Default Switch, WSL) and container or NAT bridges (docker, virbr, lxc, cni) have no default
//! gateway, so they stay [`Kind::Virtual`]. VPN detection runs first, so a VPN never qualifies.
//! Not applied on macOS: VM bridging there keeps the host address on `en*`, and `bridge*` is
//! the Thunderbolt bridge or Internet Sharing.

use std::net::IpAddr;

use serde::{Deserialize, Serialize};

/// What kind of interface this is, for our purpose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Kind {
    /// A real Ethernet or Wi-Fi adapter. Carries sync traffic and mDNS.
    Physical,
    /// A VPN or tunnel (WireGuard, Wintun, TAP, utun, Tailscale, PPP...). Never carries our
    /// traffic. Its presence feeds the VPN-blocks-LAN rule.
    Vpn,
    /// A virtual switch, bridge or container link (docker, veth, vEthernet, Hyper-V, VMware).
    Virtual,
    /// Loopback.
    Loopback,
    /// Anything else (cellular, Bluetooth PAN, unknown).
    Other,
}

/// One address on an interface, with its on-link prefix length.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct IfAddr {
    /// The address.
    pub ip: IpAddr,
    /// Prefix length (for example 24 for `255.255.255.0`).
    pub prefix: u8,
}

/// One classified interface.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Interface {
    /// OS name (`eth0`, `en0`, on Windows the adapter GUID name).
    pub name: String,
    /// OS interface index (also the IPv6 scope ID).
    pub index: u32,
    /// Friendly name and description (Windows), for logs and diagnostics.
    pub description: String,
    /// The kind.
    pub kind: Kind,
    /// Is the interface up?
    pub up: bool,
    /// Its addresses.
    pub addrs: Vec<IfAddr>,
}

impl Interface {
    /// Is this a physical interface that is up and has an address?
    pub fn usable(&self) -> bool {
        self.kind == Kind::Physical && self.up && !self.addrs.is_empty()
    }
}

/// The OS link type, as far as the OS tells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkType {
    /// Ethernet (also most virtual adapters say this).
    Ethernet,
    /// IEEE 802.11.
    Wifi,
    /// A tunnel type.
    Tunnel,
    /// Point-to-point protocol.
    Ppp,
    /// Loopback.
    Loopback,
    /// Anything else, or unknown.
    Other,
}

/// The OS family a raw record came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Os {
    /// Windows.
    Windows,
    /// macOS or iOS.
    Apple,
    /// Linux or Android.
    Linux,
    /// Anything else (no interface is physical).
    Other,
}

impl Os {
    /// The OS this build runs on.
    pub fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_vendor = "apple") {
            Os::Apple
        } else if cfg!(any(target_os = "linux", target_os = "android")) {
            Os::Linux
        } else {
            Os::Other
        }
    }
}

/// One interface as the OS reports it, before classification. Recorded lists of these are the
/// test fixtures.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawIface {
    /// OS name.
    pub name: String,
    /// OS index.
    pub index: u32,
    /// Windows: friendly name and description joined with " | ". Else empty.
    #[serde(default)]
    pub description: String,
    /// Link type.
    pub link: LinkType,
    /// Up flag.
    pub up: bool,
    /// Point-to-point flag (tunnels set it).
    #[serde(default)]
    pub point_to_point: bool,
    /// Linux: does `/sys/class/net/<if>/device` exist? `None` elsewhere.
    #[serde(default)]
    pub has_device: Option<bool>,
    /// Linux: `DEVTYPE` from `/sys/class/net/<if>/uevent` (`wlan`, `bridge`, `wireguard`...).
    #[serde(default)]
    pub devtype: Option<String>,
    /// Linux: ARPHRD link type from `/sys/class/net/<if>/type` (1 = Ethernet).
    #[serde(default)]
    pub arphrd: Option<u32>,
    /// Default gateway addresses on this interface (Windows: `FirstGatewayAddress` of
    /// `GetAdaptersAddresses`; Linux and macOS: the default route). Empty when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gateways: Vec<IpAddr>,
    /// Addresses.
    pub addrs: Vec<IfAddr>,
}

/// Classifier options.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClassifyOptions {
    /// Interface names to treat as physical whatever they look like. Only the network
    /// namespace tests set this (their "LAN" is a veth pair). Never set by the agent.
    pub force_physical: Vec<String>,
}

const VPN_NAME_PREFIXES: &[&str] = &[
    "wg",
    "tun",
    "tap",
    "utun",
    "ipsec",
    "ppp",
    "tailscale",
    "zt",
    "nordlynx",
    "proton",
    "mullvad",
    "wt",
    "gpd",
    "cscotun",
    "nebula",
];

const VPN_DESC_WORDS: &[&str] = &[
    "wireguard",
    "wintun",
    "tap-windows",
    "tap-protonvpn",
    "tap adapter",
    "openvpn",
    "tailscale",
    "zerotier",
    "nordlynx",
    "mullvad",
    "anyconnect",
    "globalprotect",
    "pangp",
    "fortinet",
    "forticlient",
    "juniper",
    "pulse secure",
    "sonicwall",
    "vpn",
    "wan miniport",
    "netbird",
    "nebula",
    "hamachi",
    "openconnect",
    "tunnel",
];

const VIRTUAL_NAME_PREFIXES: &[&str] = &[
    "docker",
    "br-",
    "veth",
    "virbr",
    "vethernet",
    "vmnet",
    "vboxnet",
    "lxc",
    "lxd",
    "cni",
    "flannel",
    "cali",
    "kube",
    "podman",
    "awdl",
    "llw",
    "anpi",
    "bridge",
    "gif",
    "stf",
    "vmenet",
    "feth",
    "dummy",
    "ifb",
    "bnep",
    "ap",
    "xhc",
    "macvtap",
    "vnet",
    "tunl",
    "sit",
    "ip6tnl",
    "erspan",
    "gre",
    "nlmon",
];

const VIRTUAL_DESC_WORDS: &[&str] = &[
    "hyper-v virtual",
    "vethernet",
    "virtual ethernet",
    "virtual adapter",
    "vmware virtual",
    "virtualbox host-only",
    "host-only",
    "loopback",
    "npcap",
    "wi-fi direct",
    "teredo",
    "6to4",
    "isatap",
    "kernel debug",
    "bluetooth",
    "docker",
    "parallels",
];

const NON_PHYSICAL_DEVTYPES: &[&str] = &[
    "bridge",
    "wireguard",
    "vlan",
    "bond",
    "veth",
    "tun",
    "tap",
    "macvlan",
    "ipvlan",
    "vxlan",
    "dsa",
];

fn starts_with_any(name: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|p| name.starts_with(p))
}

fn contains_any(text: &str, words: &[&str]) -> bool {
    words.iter().any(|w| text.contains(w))
}

/// Does this interface carry the default route of a private LAN? True when it has an IPv4
/// default gateway that is a private (RFC 1918) address inside one of the interface's own
/// private IPv4 subnets. This tells a host's external-switch or host-bridge adapter (the LAN)
/// from an internal switch or a NAT bridge (no default gateway).
pub fn lan_gateway(raw: &RawIface) -> bool {
    raw.gateways.iter().any(|g| match g {
        IpAddr::V4(g4) if g4.is_private() => raw.addrs.iter().any(|a| {
            matches!(a.ip, IpAddr::V4(v4) if v4.is_private()) && crate::addr::in_subnet(*g, a)
        }),
        _ => false,
    })
}

/// A Hyper-V host adapter (`vEthernet (<switch>)`, "Hyper-V Virtual Ethernet Adapter").
fn hyperv_host_adapter(desc: &str) -> bool {
    desc.starts_with("vethernet") || desc.contains("hyper-v virtual ethernet")
}

/// Classify one raw record from OS `os`.
pub fn classify(os: Os, raw: &RawIface, opts: &ClassifyOptions) -> Kind {
    if opts.force_physical.iter().any(|n| n == &raw.name) {
        return Kind::Physical;
    }
    let name = raw.name.to_ascii_lowercase();
    let desc = raw.description.to_ascii_lowercase();
    if raw.link == LinkType::Loopback || name == "lo" || name == "lo0" {
        return Kind::Loopback;
    }
    let devtype = raw.devtype.as_deref().unwrap_or("").to_ascii_lowercase();
    if matches!(raw.link, LinkType::Tunnel | LinkType::Ppp)
        || devtype == "wireguard"
        || starts_with_any(&name, VPN_NAME_PREFIXES)
        || contains_any(&desc, VPN_DESC_WORDS)
    {
        return Kind::Vpn;
    }
    let ether_or_wifi = matches!(raw.link, LinkType::Ethernet | LinkType::Wifi);
    // The host's adapter on a Hyper-V external switch carries the LAN (module text).
    if os == Os::Windows
        && hyperv_host_adapter(&desc)
        && ether_or_wifi
        && !raw.point_to_point
        && lan_gateway(raw)
    {
        return Kind::Physical;
    }
    if starts_with_any(&name, VIRTUAL_NAME_PREFIXES) || contains_any(&desc, VIRTUAL_DESC_WORDS) {
        return Kind::Virtual;
    }
    match os {
        Os::Windows => {
            if ether_or_wifi && !raw.point_to_point {
                Kind::Physical
            } else {
                Kind::Other
            }
        }
        Os::Apple => {
            if name.starts_with("en") && ether_or_wifi && !raw.point_to_point {
                Kind::Physical
            } else {
                Kind::Other
            }
        }
        Os::Linux => {
            if NON_PHYSICAL_DEVTYPES.contains(&devtype.as_str()) {
                // A host bridge or bond that holds the NIC and the LAN default route
                // (module text). Container and NAT bridges were already caught by name.
                let host_link = matches!(devtype.as_str(), "bridge" | "bond");
                if host_link && raw.arphrd.is_none_or(|t| t == 1) && lan_gateway(raw) {
                    return Kind::Physical;
                }
                return Kind::Virtual;
            }
            let hw = raw.has_device == Some(true);
            let ether = raw.arphrd.map_or(ether_or_wifi, |t| t == 1);
            if hw && ether && !raw.point_to_point {
                Kind::Physical
            } else if raw.point_to_point || raw.arphrd == Some(65534) {
                // ARPHRD_NONE: tun and wireguard devices with other names.
                Kind::Vpn
            } else if !hw {
                Kind::Virtual
            } else {
                Kind::Other
            }
        }
        Os::Other => Kind::Other,
    }
}

impl RawIface {
    /// Classify this record.
    pub fn classify(&self, os: Os, opts: &ClassifyOptions) -> Interface {
        let mut addrs = self.addrs.clone();
        addrs.sort();
        addrs.dedup();
        Interface {
            name: self.name.clone(),
            index: self.index,
            description: self.description.clone(),
            kind: classify(os, self, opts),
            up: self.up,
            addrs,
        }
    }
}

/// Classify a whole list. The result is sorted by index, then name.
pub fn classify_all(os: Os, raw: &[RawIface], opts: &ClassifyOptions) -> Vec<Interface> {
    let mut v: Vec<Interface> = raw.iter().map(|r| r.classify(os, opts)).collect();
    v.sort_by(|a, b| (a.index, &a.name).cmp(&(b.index, &b.name)));
    v
}

/// Is a VPN interface up with a real address? (One input of the VPN-blocks-LAN rule.)
///
/// Link-local addresses do not count: macOS always has several `utun` interfaces up with only
/// an `fe80::` address (iCloud Private Relay, Continuity and other system tunnels), and Windows
/// gives a down VPN adapter a `169.254` address. A real VPN has an IPv4 or a routable IPv6
/// address on its tunnel (recorded in `tests/fixtures/ifaces-macos-mini.json`).
pub fn vpn_up(ifaces: &[Interface]) -> bool {
    use crate::addr::{AddrClass, classify};
    ifaces.iter().any(|i| {
        i.kind == Kind::Vpn
            && i.up
            && i.addrs.iter().any(|a| {
                !matches!(
                    classify(a.ip),
                    AddrClass::LinkLocalV4 | AddrClass::LinkLocalV6 | AddrClass::Loopback
                )
            })
    })
}

/// The physical interfaces that are up and have an address.
pub fn usable(ifaces: &[Interface]) -> Vec<Interface> {
    ifaces.iter().filter(|i| i.usable()).cloned().collect()
}

impl Interface {
    /// A short name for logs: the Windows friendly name, else the OS name.
    pub fn short_name(&self) -> &str {
        match self.description.split(" | ").next() {
            Some(f) if !f.is_empty() => f,
            _ => &self.name,
        }
    }

    /// Why this interface carries no sync traffic, or `None` when it is usable.
    pub fn skip_reason(&self) -> Option<&'static str> {
        if self.usable() {
            return None;
        }
        Some(match self.kind {
            Kind::Loopback => "loopback",
            Kind::Vpn => "VPN or tunnel",
            Kind::Virtual => "virtual, no LAN default gateway",
            Kind::Other => "not Ethernet or Wi-Fi",
            Kind::Physical if !self.up => "down",
            Kind::Physical => "no address",
        })
    }
}

/// One line for the "no usable network" warning: every skipped interface and why, by name
/// only (`vEthernet (WSL): virtual, no LAN default gateway; Ethernet: no address`). Loopback
/// is left out.
pub fn skipped_summary(ifaces: &[Interface]) -> String {
    let parts: Vec<String> = ifaces
        .iter()
        .filter(|i| i.kind != Kind::Loopback)
        .filter_map(|i| i.skip_reason().map(|r| format!("{}: {r}", i.short_name())))
        .collect();
    if parts.is_empty() {
        "no interfaces".into()
    } else {
        parts.join("; ")
    }
}

/// Read the raw interface list of this machine.
pub fn raw_interfaces() -> Vec<RawIface> {
    netdev::get_interfaces()
        .into_iter()
        .map(|i| {
            use netdev::interface::types::InterfaceType as T;
            let link = match i.if_type {
                T::Ethernet
                | T::GigabitEthernet
                | T::FastEthernetT
                | T::FastEthernetFx
                | T::Ethernet3Megabit => LinkType::Ethernet,
                T::Wireless80211 => LinkType::Wifi,
                T::Tunnel => LinkType::Tunnel,
                T::Ppp => LinkType::Ppp,
                T::Loopback => LinkType::Loopback,
                _ => LinkType::Other,
            };
            let description = if cfg!(windows) {
                [i.friendly_name.clone(), i.description.clone()]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" | ")
            } else {
                String::new()
            };
            let mut addrs: Vec<IfAddr> = i
                .ipv4
                .iter()
                .map(|n| IfAddr {
                    ip: IpAddr::V4(n.addr()),
                    prefix: n.prefix_len(),
                })
                .collect();
            addrs.extend(i.ipv6.iter().map(|n| IfAddr {
                ip: IpAddr::V6(n.addr()),
                prefix: n.prefix_len(),
            }));
            let (has_device, devtype, arphrd) = linux_sysfs(&i.name);
            let gateways: Vec<IpAddr> = i
                .gateway
                .as_ref()
                .map(|g| {
                    g.ipv4
                        .iter()
                        .map(|a| IpAddr::V4(*a))
                        .chain(g.ipv6.iter().map(|a| IpAddr::V6(*a)))
                        .collect()
                })
                .unwrap_or_default();
            RawIface {
                up: i.is_up() && !matches!(i.oper_state, netdev::interface::state::OperState::Down),
                point_to_point: i.is_point_to_point(),
                name: i.name,
                index: i.index,
                description,
                link,
                has_device,
                devtype,
                arphrd,
                gateways,
                addrs,
            }
        })
        .collect()
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn linux_sysfs(name: &str) -> (Option<bool>, Option<String>, Option<u32>) {
    let base = std::path::Path::new("/sys/class/net").join(name);
    let has_device = Some(base.join("device").exists());
    let devtype = std::fs::read_to_string(base.join("uevent"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("DEVTYPE=").map(str::to_string))
        });
    let arphrd = std::fs::read_to_string(base.join("type"))
        .ok()
        .and_then(|s| s.trim().parse().ok());
    (has_device, devtype, arphrd)
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn linux_sysfs(_name: &str) -> (Option<bool>, Option<String>, Option<u32>) {
    (None, None, None)
}

/// Detect and classify the interfaces of this machine.
pub fn detect(opts: &ClassifyOptions) -> Vec<Interface> {
    classify_all(Os::current(), &raw_interfaces(), opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(name: &str, desc: &str, link: LinkType) -> RawIface {
        RawIface {
            name: name.into(),
            index: 3,
            description: desc.into(),
            link,
            up: true,
            point_to_point: false,
            has_device: None,
            devtype: None,
            arphrd: None,
            gateways: Vec::new(),
            addrs: vec![IfAddr {
                ip: "192.168.1.2".parse().unwrap(),
                prefix: 24,
            }],
        }
    }

    fn k(os: Os, r: &RawIface) -> Kind {
        classify(os, r, &ClassifyOptions::default())
    }

    #[test]
    fn windows_examples() {
        use LinkType::*;
        let cases = [
            (
                "Ethernet | Intel(R) Ethernet Controller I226-V",
                Ethernet,
                Kind::Physical,
            ),
            (
                "Wi-Fi | Intel(R) Wi-Fi 7 BE200 320MHz",
                Wifi,
                Kind::Physical,
            ),
            (
                "Ethernet | Microsoft Hyper-V Network Adapter",
                Ethernet,
                Kind::Physical,
            ),
            (
                "vEthernet (WSL) | Hyper-V Virtual Ethernet Adapter",
                Ethernet,
                Kind::Virtual,
            ),
            (
                "vEthernet (Default Switch) | Hyper-V Virtual Ethernet Adapter #2",
                Ethernet,
                Kind::Virtual,
            ),
            ("Mullvad | WireGuard Tunnel", Other, Kind::Vpn),
            ("wg0 | WireGuard Tunnel", Other, Kind::Vpn),
            ("OpenVPN Wintun | Wintun Userspace Tunnel", Other, Kind::Vpn),
            ("Ethernet 3 | TAP-Windows Adapter V9", Ethernet, Kind::Vpn),
            ("Tailscale | Tailscale Tunnel", Other, Kind::Vpn),
            ("NordLynx | NordLynx Tunnel", Other, Kind::Vpn),
            (
                "Local Area Connection* 1 | Microsoft Wi-Fi Direct Virtual Adapter",
                Wifi,
                Kind::Virtual,
            ),
            (
                "Bluetooth Network Connection | Bluetooth Device (Personal Area Network)",
                Ethernet,
                Kind::Virtual,
            ),
            (
                "VMware Network Adapter VMnet8 | VMware Virtual Ethernet Adapter for VMnet8",
                Ethernet,
                Kind::Virtual,
            ),
            (
                "Ethernet 2 | VirtualBox Host-Only Ethernet Adapter",
                Ethernet,
                Kind::Virtual,
            ),
            (
                "Npcap Loopback Adapter | Npcap Loopback Adapter",
                Ethernet,
                Kind::Virtual,
            ),
            (
                "Loopback Pseudo-Interface 1 | Software Loopback Interface 1",
                Loopback,
                Kind::Loopback,
            ),
            (
                "Teredo Tunneling Pseudo-Interface | Microsoft Teredo Tunneling Adapter",
                Tunnel,
                Kind::Vpn,
            ),
            (
                "Cellular | Generic Mobile Broadband Adapter",
                Other,
                Kind::Other,
            ),
        ];
        for (d, l, want) in cases {
            let r = raw("{6B9E3C2A-0000-0000-0000-000000000000}", d, l);
            assert_eq!(k(Os::Windows, &r), want, "{d}");
        }
    }

    #[test]
    fn apple_examples() {
        use LinkType::*;
        let cases = [
            ("en0", Ethernet, false, Kind::Physical),
            ("en1", Wifi, false, Kind::Physical),
            ("utun3", Other, true, Kind::Vpn),
            ("ipsec0", Other, true, Kind::Vpn),
            ("ppp0", Ppp, true, Kind::Vpn),
            ("bridge0", Ethernet, false, Kind::Virtual),
            ("awdl0", Ethernet, false, Kind::Virtual),
            ("llw0", Ethernet, false, Kind::Virtual),
            ("anpi0", Ethernet, false, Kind::Virtual),
            ("ap1", Ethernet, false, Kind::Virtual),
            ("gif0", Other, false, Kind::Virtual),
            ("stf0", Other, false, Kind::Virtual),
            ("lo0", Loopback, false, Kind::Loopback),
            ("en5", Other, false, Kind::Other),
        ];
        for (n, l, p2p, want) in cases {
            let mut r = raw(n, "", l);
            r.point_to_point = p2p;
            assert_eq!(k(Os::Apple, &r), want, "{n}");
        }
    }

    #[test]
    fn linux_examples() {
        // name, has_device, devtype, arphrd, p2p, want
        type Case<'a> = (&'a str, bool, Option<&'a str>, u32, bool, Kind);
        let cases: [Case; 13] = [
            ("eth0", true, None, 1, false, Kind::Physical),
            ("enp5s0", true, None, 1, false, Kind::Physical),
            ("wlp2s0", true, Some("wlan"), 1, false, Kind::Physical),
            ("wg0", false, Some("wireguard"), 65534, true, Kind::Vpn),
            (
                "mullvad-wg",
                false,
                Some("wireguard"),
                65534,
                true,
                Kind::Vpn,
            ),
            ("tun0", false, None, 65534, true, Kind::Vpn),
            ("tailscale0", false, None, 65534, true, Kind::Vpn),
            ("docker0", false, Some("bridge"), 1, false, Kind::Virtual),
            ("br0", false, Some("bridge"), 1, false, Kind::Virtual),
            ("veth1a2b", false, None, 1, false, Kind::Virtual),
            ("virbr0", false, Some("bridge"), 1, false, Kind::Virtual),
            ("lo", false, None, 772, false, Kind::Loopback),
            ("eth0.10", true, Some("vlan"), 1, false, Kind::Virtual),
        ];
        for (n, dev, dt, arp, p2p, want) in cases {
            let mut r = raw(n, "", LinkType::Ethernet);
            if n == "lo" {
                r.link = LinkType::Loopback;
            }
            r.has_device = Some(dev);
            r.devtype = dt.map(str::to_string);
            r.arphrd = Some(arp);
            r.point_to_point = p2p;
            assert_eq!(k(Os::Linux, &r), want, "{n}");
        }
    }

    fn gw(mut r: RawIface, ip: &str, prefix: u8, gateway: Option<&str>) -> RawIface {
        r.addrs = vec![IfAddr {
            ip: ip.parse().unwrap(),
            prefix,
        }];
        r.gateways = gateway.into_iter().map(|g| g.parse().unwrap()).collect();
        r
    }

    #[test]
    fn hyperv_external_switch_host_adapter_is_lan() {
        // Max's PC, 2026-11-14: the Realtek NIC is bound to the external switch "uc-lan", so
        // the host's only LAN address is on vEthernet (uc-lan).
        use LinkType::*;
        let w = |d: &str, l, ip, p, g| gw(raw("{GUID}", d, l), ip, p, g);
        let uc_lan = w(
            "vEthernet (uc-lan) | Hyper-V Virtual Ethernet Adapter #3",
            Ethernet,
            "192.168.0.139",
            24,
            Some("192.168.0.1"),
        );
        assert_eq!(k(Os::Windows, &uc_lan), Kind::Physical);
        // Internal switches: no default gateway.
        for d in [
            "vEthernet (Default Switch) | Hyper-V Virtual Ethernet Adapter",
            "vEthernet (WSL (Hyper-V firewall)) | Hyper-V Virtual Ethernet Adapter #2",
        ] {
            let r = w(d, Ethernet, "172.20.16.1", 20, None);
            assert_eq!(k(Os::Windows, &r), Kind::Virtual, "{d}");
        }
        // A gateway that is public, outside the subnet, or only IPv6 does not count.
        for (ip, p, g) in [
            ("192.168.0.139", 24, "8.8.8.8"),
            ("192.168.0.139", 24, "10.0.0.1"),
            ("192.168.0.139", 24, "fe80::1"),
            ("100.64.0.5", 10, "100.64.0.1"),
        ] {
            let r = w(
                "vEthernet (x) | Hyper-V Virtual Ethernet Adapter",
                Ethernet,
                ip,
                p,
                Some(g),
            );
            assert_eq!(k(Os::Windows, &r), Kind::Virtual, "{ip} via {g}");
        }
        // A VPN with a LAN-looking gateway stays a VPN.
        for d in [
            "OpenVPN Data Channel Offload | OpenVPN Data Channel Offload",
            "Ethernet 4 | Cisco AnyConnect Secure Mobility Client Virtual Miniport Adapter",
            "vEthernet (vpn) | WireGuard Tunnel",
        ] {
            let r = w(d, Ethernet, "10.8.0.2", 24, Some("10.8.0.1"));
            assert_eq!(k(Os::Windows, &r), Kind::Vpn, "{d}");
        }
        // Other virtual adapters with a gateway stay virtual (only Hyper-V host adapters).
        let vmware = w(
            "VMware Network Adapter VMnet8 | VMware Virtual Ethernet Adapter for VMnet8",
            Ethernet,
            "192.168.80.1",
            24,
            Some("192.168.80.2"),
        );
        assert_eq!(k(Os::Windows, &vmware), Kind::Virtual);
        // A normal PC is unchanged.
        let nic = w(
            "Ethernet | Realtek Gaming 5GbE Family Controller",
            Ethernet,
            "192.168.0.20",
            24,
            Some("192.168.0.1"),
        );
        assert_eq!(k(Os::Windows, &nic), Kind::Physical);
    }

    #[test]
    fn linux_host_bridge_with_default_route_is_lan() {
        let lin = |n: &str, dt: &str, g| {
            let mut r = gw(raw(n, "", LinkType::Ethernet), "192.168.1.10", 24, g);
            r.has_device = Some(false);
            r.devtype = Some(dt.into());
            r.arphrd = Some(1);
            r
        };
        assert_eq!(
            k(Os::Linux, &lin("br0", "bridge", Some("192.168.1.1"))),
            Kind::Physical
        );
        assert_eq!(
            k(Os::Linux, &lin("vmbr0", "bridge", Some("192.168.1.1"))),
            Kind::Physical
        );
        assert_eq!(
            k(Os::Linux, &lin("bond0", "bond", Some("192.168.1.1"))),
            Kind::Physical
        );
        assert_eq!(k(Os::Linux, &lin("br0", "bridge", None)), Kind::Virtual);
        // Container and NAT bridges stay virtual even with a gateway.
        for n in ["docker0", "virbr0", "br-1a2b3c", "lxcbr0", "cni0"] {
            assert_eq!(
                k(Os::Linux, &lin(n, "bridge", Some("192.168.1.1"))),
                Kind::Virtual,
                "{n}"
            );
        }
        // Not on macOS.
        let mut b = gw(
            raw("bridge0", "", LinkType::Ethernet),
            "192.168.1.10",
            24,
            Some("192.168.1.1"),
        );
        b.point_to_point = false;
        assert_eq!(k(Os::Apple, &b), Kind::Virtual);
    }

    #[test]
    fn skipped_summary_names_and_reasons() {
        let o = ClassifyOptions::default();
        let list = classify_all(
            Os::Windows,
            &[
                raw(
                    "{A}",
                    "vEthernet (WSL) | Hyper-V Virtual Ethernet Adapter",
                    LinkType::Ethernet,
                ),
                {
                    let mut r = raw("{B}", "Ethernet | Realtek PCIe 5GbE", LinkType::Ethernet);
                    r.addrs.clear();
                    r
                },
                raw("{C}", "Loopback Pseudo-Interface 1 | x", LinkType::Loopback),
            ],
            &o,
        );
        assert_eq!(
            skipped_summary(&list),
            "vEthernet (WSL): virtual, no LAN default gateway; Ethernet: no address"
        );
    }

    #[test]
    fn force_physical_wins() {
        let r = raw("veth-a", "", LinkType::Ethernet);
        let o = ClassifyOptions {
            force_physical: vec!["veth-a".into()],
        };
        assert_eq!(classify(Os::Linux, &r, &o), Kind::Physical);
    }

    #[test]
    fn vpn_up_needs_address() {
        let mut r = raw("wg0", "", LinkType::Other);
        let o = ClassifyOptions::default();
        assert!(vpn_up(&[r.classify(Os::Linux, &o)]));
        r.addrs.clear();
        assert!(!vpn_up(&[r.classify(Os::Linux, &o)]));
    }
}
