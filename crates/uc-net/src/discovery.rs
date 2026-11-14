//! mDNS / DNS-SD behind a trait (plan 3.4, method 3), so iOS (`NWBrowser`) and Android
//! (`NsdManager`) can plug in their own.
//!
//! Service type `_uclip._udp.local.`. The instance name is a rotating tag ([`crate::tags`]);
//! the host name is the tag too, so no device name ever goes out. The desktop implementation
//! ([`MdnsSd`], crate `mdns-sd`) sends and listens only on the physical interfaces: every other
//! interface (VPN, virtual, loopback) is disabled in the daemon.
//!
//! Interfaces are given to `mdns-sd` by **OS index**, never by name: on Windows our
//! [`Interface::name`] is the adapter GUID (from `netdev`), while `mdns-sd` (through
//! `if-addrs`) names an interface by its friendly name, so a name never matched there and
//! mDNS ran on no interface at all (found 2026-11-14). The index is the same number in
//! both crates on every OS (`IfIndex` / `Ipv6IfIndex` on Windows, `if_nametoindex` on
//! Linux and macOS).

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use crate::ifaces::Interface;

/// The DNS-SD service type.
pub const SERVICE_TYPE: &str = "_uclip._udp.local.";

/// The DNS-SD service type of an open pairing (protocol 4.2 step 4). Instance name: the
/// 8-byte pairing tag as 16 hex characters. One TXT entry, `v=1`.
pub const PAIR_SERVICE_TYPE: &str = "_uclip-pair._udp.local.";

/// One service instance seen on the LAN.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// The instance name (a tag, if it is one of ours).
    pub instance: String,
    /// Its addresses.
    pub addrs: Vec<IpAddr>,
    /// Its UDP port.
    pub port: u16,
}

/// A discovery error (the agent logs it and keeps going without mDNS).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveryError(pub String);

impl std::fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "mDNS: {}", self.0)
    }
}

impl std::error::Error for DiscoveryError {}

/// LAN service discovery.
pub trait Discovery: Send + Sync {
    /// Use only these interfaces (the physical ones). Called on every interface change.
    fn set_interfaces(&self, ifaces: &[Interface]) -> Result<(), DiscoveryError>;
    /// Advertise `instance` on `port`, replacing any earlier advertisement. `addrs` are this
    /// device's bound LAN addresses (implementations that find addresses themselves may
    /// ignore them).
    fn advertise(&self, instance: &str, port: u16, addrs: &[IpAddr]) -> Result<(), DiscoveryError>;
    /// Stop advertising.
    fn withdraw(&self);
    /// Everything resolved since the last call. Never blocks.
    fn poll(&self) -> Vec<Found>;

    /// Advertise an open pairing (protocol 4.2 step 4). Default: not supported (no pairing
    /// by mDNS; the joiner needs the QR code or a known address).
    fn advertise_pair(&self, _instance: &str, _port: u16) -> Result<(), DiscoveryError> {
        Ok(())
    }
    /// Stop advertising the pairing (4.2 step 5).
    fn withdraw_pair(&self) {}
    /// Pairing services resolved since the last call (the joiner side). The first call starts
    /// browsing. Never blocks.
    fn poll_pair(&self) -> Vec<Found> {
        Vec::new()
    }
}

/// No discovery at all (remembered addresses and gossip only).
#[derive(Clone, Copy, Debug, Default)]
pub struct NoDiscovery;

impl Discovery for NoDiscovery {
    fn set_interfaces(&self, _: &[Interface]) -> Result<(), DiscoveryError> {
        Ok(())
    }
    fn advertise(&self, _: &str, _: u16, _: &[IpAddr]) -> Result<(), DiscoveryError> {
        Ok(())
    }
    fn withdraw(&self) {}
    fn poll(&self) -> Vec<Found> {
        Vec::new()
    }
}

/// The desktop implementation on the `mdns-sd` crate.
pub struct MdnsSd {
    daemon: mdns_sd::ServiceDaemon,
    browse: mdns_sd::Receiver<mdns_sd::ServiceEvent>,
    state: Mutex<MdnsState>,
}

#[derive(Default)]
struct MdnsState {
    advertised: Option<String>,
    /// OS indexes of the interfaces enabled in the daemon, sorted.
    enabled: Vec<u32>,
    pair_advertised: Option<String>,
    pair_browse: Option<mdns_sd::Receiver<mdns_sd::ServiceEvent>>,
}

/// The OS indexes of the interfaces mDNS should run on: the usable (physical, up, with an
/// address) ones, sorted, without duplicates. Index 0 means "unknown" and is skipped.
pub fn mdns_indexes(ifaces: &[Interface]) -> Vec<u32> {
    let mut v: Vec<u32> = ifaces
        .iter()
        .filter(|i| i.usable() && i.index != 0)
        .map(|i| i.index)
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// The `mdns-sd` selectors for these interface indexes: IPv4 and IPv6 for each.
pub fn mdns_if_kinds(indexes: &[u32]) -> Vec<mdns_sd::IfKind> {
    indexes
        .iter()
        .flat_map(|&i| [mdns_sd::IfKind::IndexV4(i), mdns_sd::IfKind::IndexV6(i)])
        .collect()
}

/// What the daemon must never use, whatever an index selector says.
fn always_off() -> Vec<mdns_sd::IfKind> {
    vec![mdns_sd::IfKind::LoopbackV4, mdns_sd::IfKind::LoopbackV6]
}

impl MdnsSd {
    /// The underlying daemon, for diagnostics (`examples/mdns-check.rs`).
    pub fn daemon(&self) -> &mdns_sd::ServiceDaemon {
        &self.daemon
    }

    /// Start the daemon with every interface disabled. Call [`Discovery::set_interfaces`] next.
    pub fn new() -> Result<Self, DiscoveryError> {
        let e = |x: mdns_sd::Error| DiscoveryError(x.to_string());
        let daemon = mdns_sd::ServiceDaemon::new().map_err(e)?;
        daemon
            .disable_interface(vec![
                mdns_sd::IfKind::All,
                mdns_sd::IfKind::LoopbackV4,
                mdns_sd::IfKind::LoopbackV6,
            ])
            .map_err(e)?;
        let browse = daemon.browse(SERVICE_TYPE).map_err(e)?;
        Ok(Self {
            daemon,
            browse,
            state: Mutex::new(MdnsState::default()),
        })
    }
}

impl Drop for MdnsSd {
    fn drop(&mut self) {
        let _ = self.daemon.shutdown();
    }
}

impl Discovery for MdnsSd {
    fn set_interfaces(&self, ifaces: &[Interface]) -> Result<(), DiscoveryError> {
        let e = |x: mdns_sd::Error| DiscoveryError(x.to_string());
        let mut st = self
            .state
            .lock()
            .map_err(|_| DiscoveryError("lock".into()))?;
        let want = mdns_indexes(ifaces);
        if want == st.enabled {
            return Ok(());
        }
        let off: Vec<u32> = st
            .enabled
            .iter()
            .copied()
            .filter(|n| !want.contains(n))
            .collect();
        if !off.is_empty() {
            self.daemon
                .disable_interface(mdns_if_kinds(&off))
                .map_err(e)?;
        }
        let on: Vec<u32> = want
            .iter()
            .copied()
            .filter(|n| !st.enabled.contains(n))
            .collect();
        if !on.is_empty() {
            self.daemon
                .enable_interface(mdns_if_kinds(&on))
                .map_err(e)?;
            // The daemon applies selections in order and the last match wins: keep loopback
            // off even if an index ever pointed at it.
            self.daemon.disable_interface(always_off()).map_err(e)?;
        }
        tracing::debug!(indexes = ?want, "mDNS interfaces");
        st.enabled = want;
        Ok(())
    }

    fn advertise(
        &self,
        instance: &str,
        port: u16,
        _addrs: &[IpAddr],
    ) -> Result<(), DiscoveryError> {
        let e = |x: mdns_sd::Error| DiscoveryError(x.to_string());
        let mut st = self
            .state
            .lock()
            .map_err(|_| DiscoveryError("lock".into()))?;
        let host = format!("{instance}.local.");
        let mut info = mdns_sd::ServiceInfo::new(
            SERVICE_TYPE,
            instance,
            &host,
            &[] as &[IpAddr],
            port,
            None::<std::collections::HashMap<String, String>>,
        )
        .map_err(e)?
        .enable_addr_auto();
        // The instance name is a per-device HMAC tag, unique by construction: skip the
        // 750 ms name probe (RFC 6762 8.1) so a new device is found faster.
        info.set_requires_probe(false);
        info.set_interfaces(mdns_if_kinds(&st.enabled));
        let full = info.get_fullname().to_string();
        if st.advertised.as_deref() == Some(full.as_str()) {
            return Ok(());
        }
        if let Some(old) = st.advertised.take() {
            let _ = self.daemon.unregister(&old);
        }
        self.daemon.register(info).map_err(e)?;
        st.advertised = Some(full);
        Ok(())
    }

    fn withdraw(&self) {
        if let Ok(mut st) = self.state.lock()
            && let Some(old) = st.advertised.take()
        {
            let _ = self.daemon.unregister(&old);
        }
    }

    fn advertise_pair(&self, instance: &str, port: u16) -> Result<(), DiscoveryError> {
        let e = |x: mdns_sd::Error| DiscoveryError(x.to_string());
        let mut st = self
            .state
            .lock()
            .map_err(|_| DiscoveryError("lock".into()))?;
        let host = format!("{instance}.local.");
        let txt: std::collections::HashMap<String, String> =
            [("v".to_string(), "1".to_string())].into();
        let mut info = mdns_sd::ServiceInfo::new(
            PAIR_SERVICE_TYPE,
            instance,
            &host,
            &[] as &[IpAddr],
            port,
            Some(txt),
        )
        .map_err(e)?
        .enable_addr_auto();
        info.set_requires_probe(false);
        info.set_interfaces(mdns_if_kinds(&st.enabled));
        let full = info.get_fullname().to_string();
        if let Some(old) = st.pair_advertised.take() {
            let _ = self.daemon.unregister(&old);
        }
        self.daemon.register(info).map_err(e)?;
        st.pair_advertised = Some(full);
        Ok(())
    }

    fn withdraw_pair(&self) {
        if let Ok(mut st) = self.state.lock()
            && let Some(old) = st.pair_advertised.take()
        {
            let _ = self.daemon.unregister(&old);
        }
    }

    fn poll_pair(&self) -> Vec<Found> {
        let Ok(mut st) = self.state.lock() else {
            return Vec::new();
        };
        if st.pair_browse.is_none() {
            match self.daemon.browse(PAIR_SERVICE_TYPE) {
                Ok(rx) => st.pair_browse = Some(rx),
                Err(_) => return Vec::new(),
            }
        }
        let mut out = Vec::new();
        if let Some(rx) = &st.pair_browse {
            while let Ok(ev) = rx.try_recv() {
                if let mdns_sd::ServiceEvent::ServiceResolved(r) = ev {
                    let Some(instance) = r
                        .get_fullname()
                        .strip_suffix(&format!(".{PAIR_SERVICE_TYPE}"))
                    else {
                        continue;
                    };
                    out.push(Found {
                        instance: instance.to_string(),
                        addrs: r.get_addresses().iter().map(|a| a.to_ip_addr()).collect(),
                        port: r.get_port(),
                    });
                }
            }
        }
        out
    }

    fn poll(&self) -> Vec<Found> {
        let mut out = Vec::new();
        while let Ok(ev) = self.browse.try_recv() {
            if let mdns_sd::ServiceEvent::ServiceResolved(r) = ev {
                let Some(instance) = r.get_fullname().strip_suffix(&format!(".{SERVICE_TYPE}"))
                else {
                    continue;
                };
                out.push(Found {
                    instance: instance.to_string(),
                    addrs: r.get_addresses().iter().map(|a| a.to_ip_addr()).collect(),
                    port: r.get_port(),
                });
            }
        }
        out
    }
}

type FakeMap = BTreeMap<usize, (String, u16, Vec<IpAddr>, Arc<std::sync::atomic::AtomicBool>)>;

/// An in-memory mDNS for tests: every [`FakeMdns`] made from one [`FakeLan`] sees the others'
/// advertisements. A test can cut a node off (multicast blocked).
#[derive(Clone, Default)]
pub struct FakeLan {
    inner: Arc<Mutex<FakeMap>>,
}

impl FakeLan {
    /// A new, empty LAN.
    pub fn new() -> Self {
        Self::default()
    }

    /// A node on this LAN.
    pub fn node(&self) -> FakeMdns {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
        FakeMdns {
            lan: self.clone(),
            me: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            blocked: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

/// A node of a [`FakeLan`].
#[derive(Clone)]
pub struct FakeMdns {
    lan: FakeLan,
    me: usize,
    blocked: Arc<std::sync::atomic::AtomicBool>,
}

impl FakeMdns {
    /// Block or unblock multicast for this node (it neither sends nor receives).
    pub fn set_blocked(&self, b: bool) {
        self.blocked.store(b, std::sync::atomic::Ordering::Relaxed);
    }

    fn blocked(&self) -> bool {
        self.blocked.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl Discovery for FakeMdns {
    fn set_interfaces(&self, _: &[Interface]) -> Result<(), DiscoveryError> {
        Ok(())
    }

    fn advertise(&self, instance: &str, port: u16, addrs: &[IpAddr]) -> Result<(), DiscoveryError> {
        if let Ok(mut m) = self.lan.inner.lock() {
            m.insert(
                self.me,
                (
                    instance.to_string(),
                    port,
                    addrs.to_vec(),
                    self.blocked.clone(),
                ),
            );
        }
        Ok(())
    }

    fn withdraw(&self) {
        if let Ok(mut m) = self.lan.inner.lock() {
            m.remove(&self.me);
        }
    }

    fn poll(&self) -> Vec<Found> {
        if self.blocked() {
            return Vec::new();
        }
        let Ok(m) = self.lan.inner.lock() else {
            return Vec::new();
        };
        m.iter()
            .filter(|(k, v)| **k != self.me && !v.3.load(std::sync::atomic::Ordering::Relaxed))
            .map(|(_, (i, p, a, _))| Found {
                instance: i.clone(),
                addrs: a.clone(),
                port: *p,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ifaces::{IfAddr, Kind};

    fn iface(name: &str, index: u32, kind: Kind, up: bool, addr: Option<&str>) -> Interface {
        Interface {
            name: name.into(),
            index,
            description: String::new(),
            kind,
            up,
            addrs: addr
                .map(|a| IfAddr {
                    ip: a.parse().unwrap(),
                    prefix: 24,
                })
                .into_iter()
                .collect(),
        }
    }

    #[test]
    fn indexes_are_the_usable_interfaces() {
        let l = [
            // The Windows case of 2026-11-14: a GUID name that mdns-sd never sees.
            iface(
                "{C5C4BEC5-0000-0000-0000-000000000000}",
                17,
                Kind::Physical,
                true,
                Some("192.168.0.139"),
            ),
            iface("en0", 4, Kind::Physical, true, Some("192.168.0.47")),
            iface("en0-dup", 4, Kind::Physical, true, Some("fe80::1")),
            iface("lo", 1, Kind::Loopback, true, Some("127.0.0.1")),
            iface(
                "vEthernet (WSL)",
                30,
                Kind::Virtual,
                true,
                Some("172.20.0.1"),
            ),
            iface("wg0", 9, Kind::Vpn, true, Some("10.0.0.2")),
            iface("eth1", 5, Kind::Physical, false, Some("192.168.1.2")),
            iface("eth2", 6, Kind::Physical, true, None),
            iface("eth3", 0, Kind::Physical, true, Some("192.168.2.2")),
        ];
        assert_eq!(mdns_indexes(&l), vec![4, 17]);
    }

    #[test]
    fn selectors_are_by_index_for_both_families() {
        let k: Vec<String> = mdns_if_kinds(&[4, 17])
            .iter()
            .map(|k| format!("{k:?}"))
            .collect();
        assert_eq!(
            k,
            ["IndexV4(4)", "IndexV6(4)", "IndexV4(17)", "IndexV6(17)"]
        );
        assert!(mdns_if_kinds(&[]).is_empty());
        let off: Vec<String> = always_off().iter().map(|k| format!("{k:?}")).collect();
        assert_eq!(off, ["LoopbackV4", "LoopbackV6"]);
    }
}
