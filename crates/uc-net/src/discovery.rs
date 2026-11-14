//! mDNS / DNS-SD behind a trait (plan 3.4, method 3), so iOS (`NWBrowser`) and Android
//! (`NsdManager`) can plug in their own.
//!
//! Service type `_uclip._udp.local.`. The instance name is a rotating tag ([`crate::tags`]);
//! the host name is the tag too, so no device name ever goes out. The desktop implementation
//! ([`MdnsSd`], crate `mdns-sd`) sends and listens only on the physical interfaces: every other
//! interface (VPN, virtual, loopback) is disabled in the daemon.

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
    enabled: Vec<String>,
    pair_advertised: Option<String>,
    pair_browse: Option<mdns_sd::Receiver<mdns_sd::ServiceEvent>>,
}

impl MdnsSd {
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
        let want: Vec<String> = ifaces
            .iter()
            .filter(|i| i.usable())
            .map(|i| i.name.clone())
            .collect();
        if want == st.enabled {
            return Ok(());
        }
        let off: Vec<mdns_sd::IfKind> = st
            .enabled
            .iter()
            .filter(|n| !want.contains(n))
            .map(|n| mdns_sd::IfKind::Name(n.clone()))
            .collect();
        if !off.is_empty() {
            self.daemon.disable_interface(off).map_err(e)?;
        }
        let on: Vec<mdns_sd::IfKind> = want
            .iter()
            .filter(|n| !st.enabled.contains(n))
            .map(|n| mdns_sd::IfKind::Name(n.clone()))
            .collect();
        if !on.is_empty() {
            self.daemon.enable_interface(on).map_err(e)?;
        }
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
        info.set_interfaces(
            st.enabled
                .iter()
                .map(|n| mdns_sd::IfKind::Name(n.clone()))
                .collect(),
        );
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
        info.set_interfaces(
            st.enabled
                .iter()
                .map(|n| mdns_sd::IfKind::Name(n.clone()))
                .collect(),
        );
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
