//! The agent core: one task that owns all state and wires the sync engine (`uc-core`), the
//! network (`uc-net`), pairing and membership (`uc-pair`), the key store (`uc-keys`) and the
//! clipboard backend. Plan 3.1, 3.2, protocol 5.7 to 5.9.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use data_encoding::HEXLOWER;
use rand_core::{OsRng, RngCore};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use uc_clip::Clipboard;
use uc_core::msg::{self, FrameType, Hello};
use uc_core::sync::{Config, Engine, Event as EngineEvent, SkipReason};
use uc_core::{ClipKind, DeviceId, DeviceKind, DeviceRecord, Message, Role};
use uc_keys::{DeviceKey, KeyStore};
use uc_net::ifaces::{IfAddr, Interface, Kind};
use uc_net::{LanBook, Link, Net, NetConfig, NetEvent, NoPath, PathState, Policy, Secrets};
use uc_pair::chain::{Graph, GroupSecret, Inserted, SelfAction, self_action};
use uc_pair::frame::{AbortReason, T_CONFIRM};
use uc_pair::gate::{Gate, UNKNOWN_PEER_MAX_BYTES, UNKNOWN_PEER_MAX_CHANGES};
use uc_pair::session::{
    GroupCtx, Host, InviterRun, InviterStep, Lockout, NOTICE_FAILED_TRY, lockout_notice,
};

use crate::autostart::Autostart;
use crate::clip::{Backend, Changes};
use crate::ipc::{self, Event, Incoming, IpcError, Reply};
use crate::pairing::{self, PairMsg};
use crate::profile::Profile;
use crate::settings;
use crate::state::{Activity, Notice, PairLockout, Pause, StateFile};

mod transfers;

use transfers::{FileState, Xfer};

/// The fix for a missing Windows firewall rule (one UAC prompt).
const FIREWALL_FIX: &str =
    "Run \"uc firewall add\" (or Fix in the window). Windows asks for permission once.";
const SECRET_NAME: &str = "group-secret";
const PREV_SECRET_NAME: &str = "group-secret-prev";
/// Settle time after a clipboard change (plan 3.8).
const SETTLE: Duration = Duration::from_millis(150);
/// After a failed read (clipboard held by another program, or a read that timed out), try
/// again this many times, waiting 0.5 s, 1 s, 2 s, 4 s.
const READ_RETRIES: u32 = 4;
/// Time given to queued frames before a link to a removed device closes.
const FLUSH: Duration = Duration::from_millis(1000);

/// Wall clock in ms.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn now_s() -> u64 {
    now_ms() / 1000
}

fn hex(b: &[u8]) -> String {
    HEXLOWER.encode(b)
}

/// A short device ID for log lines: the first 4 bytes in hex. Never a name or an address.
fn short_id(id: &[u8; 32]) -> String {
    hex(&id[..4])
}

/// The OS device type of protocol 4.7.
pub fn own_device_type() -> u8 {
    if cfg!(windows) {
        1
    } else if cfg!(target_os = "macos") {
        2
    } else {
        3
    }
}

/// The engine settings for this device. Test profiles with `UCLIP_TEST_NO_FILES=1` send no
/// feature bits, so they act as a 0.1.x peer that cannot receive files.
fn engine_config(profile: &Profile, rec: DeviceRecord) -> Config {
    let mut cfg = Config::new(rec, Role::Full);
    if profile.test_env("UCLIP_TEST_NO_FILES").as_deref() == Some("1") {
        cfg.features = 0;
    }
    cfg
}

fn kind_of_type(t: u8) -> DeviceKind {
    match t {
        1 => DeviceKind::Windows,
        2 => DeviceKind::Mac,
        4 => DeviceKind::IPhone,
        5 => DeviceKind::IPad,
        6 | 7 => DeviceKind::Android,
        _ => DeviceKind::Linux,
    }
}

fn type_name(t: u8) -> &'static str {
    match t {
        1 => "Windows",
        2 => "macOS",
        3 => "Linux",
        4 => "iPhone",
        5 => "iPad",
        6 => "Android phone",
        7 => "Android tablet",
        _ => "unknown",
    }
}

fn kind_text(k: ClipKind) -> &'static str {
    match k {
        ClipKind::Text => "Text",
        ClipKind::Image => "Image",
        ClipKind::TextAndImage => "Text and image",
    }
}

/// A default device name from the OS ("A new device takes a default name from the OS").
pub fn default_name() -> String {
    let raw = if cfg!(windows) {
        std::env::var("COMPUTERNAME").unwrap_or_default()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("scutil")
            .args(["--get", "ComputerName"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    } else {
        std::fs::read_to_string("/etc/hostname").unwrap_or_default()
    };
    clean_name(&raw).unwrap_or_else(|| "My computer".to_string())
}

/// A name that the chain accepts (1 to 64 bytes, no control characters), or `None`.
pub fn clean_name(s: &str) -> Option<String> {
    let mut n: String = s.trim().chars().filter(|c| !c.is_control()).collect();
    while n.len() > 64 {
        n.pop();
    }
    uc_pair::chain::check_name(n.as_bytes()).map(str::to_string)
}

/// One sync link and its gate (protocol 5.9).
struct PeerLink {
    link: Link,
    gate: Gate,
    started: Instant,
    bytes: usize,
    changes: usize,
    heads: Vec<[u8; 32]>,
    epoch: u64,
}

impl PeerLink {
    fn send(&self, m: &Message) {
        let _ = self.link.send(msg::encode(m));
    }
}

/// A pending reply to send later.
struct Pending {
    id: u64,
    reply: oneshot::Sender<Reply>,
}

impl Pending {
    fn answer(self, r: Result<Value, IpcError>) {
        let _ = self.reply.send(Reply::from_result(self.id, r));
    }
}

/// Inputs from other threads.
enum Wake {
    ClipChanged,
    /// The clipboard watcher ended. Only an error while the agent still runs (at a normal
    /// quit the watcher ends too, after the main loop has stopped reading).
    ClipWatchStopped,
    #[cfg_attr(not(windows), allow(dead_code))]
    Resume,
}

/// How often the `transfer` event may go out (at most 4 per second, file sync spec 5).
const TRANSFER_TICK: Duration = Duration::from_millis(250);

/// IPC calls that change saved settings. Their reply says so when the save failed.
const CHANGE_CALLS: &[&str] = &["devices.rename", "pause.set", "settings.set", "setup.step"];

/// Status reason and pairing error when no interface can carry sync traffic.
const NO_LAN_TEXT: &str = "This device found no network it can use.";

/// The joiner's finder for test profiles and mDNS.
struct Finder {
    fixed: Vec<SocketAddr>,
    disc: Option<Arc<dyn uc_net::Discovery>>,
    policy: Policy,
    ifaces: Vec<Interface>,
}

impl pairing::Finder for Finder {
    fn poll(&mut self) -> Vec<SocketAddr> {
        let mut out = self.fixed.clone();
        if let Some(d) = &self.disc {
            for f in d.poll_pair() {
                for ip in f.addrs {
                    let a = SocketAddr::new(ip, f.port);
                    if uc_net::addr::allowed_any(a, &self.ifaces, &self.policy) {
                        out.push(a);
                    }
                }
            }
        }
        out
    }
}

/// The agent core.
pub struct Core {
    profile: Profile,
    dir: PathBuf,
    store: Box<dyn KeyStore>,
    key: DeviceKey,
    state: StateFile,
    graph: Option<Graph>,
    secret: Option<GroupSecret>,
    prev_secret: Option<GroupSecret>,
    engine: Engine,
    clip: Backend,
    clip_error: Option<String>,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    mac_poll: bool,
    #[cfg(target_os = "macos")]
    poller: uc_clip_mac::poll::Poller,
    net: Option<Net>,
    net_error: Option<String>,
    /// The Windows inbound rule, checked at network start: `Some(false)` = missing, so the
    /// network runs outbound-only. `None` = this build needs no rule of ours.
    fw_rule: Option<bool>,
    disc: Option<Arc<dyn uc_net::Discovery>>,
    policy: Policy,
    ifaces: Vec<Interface>,
    /// The network sent its first interface list (until then `ifaces` is not known).
    ifaces_seen: bool,
    vpn_up: bool,
    port: u16,
    host: Host,
    pair_open: bool,
    runs: HashMap<u64, InviterRun>,
    run_added: HashMap<u64, String>,
    next_run: u64,
    joining: Option<Pending>,
    peers: BTreeMap<DeviceId, PeerLink>,
    paths: BTreeMap<DeviceId, PathState>,
    closing: Vec<(Instant, DeviceId)>,
    members_due: Option<Instant>,
    rotate_at: Option<Instant>,
    leaving: Option<(Instant, Pending, bool)>,
    settle_at: Option<Instant>,
    subscribers: Vec<mpsc::UnboundedSender<Event>>,
    clock_ahead: bool,
    pair_tx: mpsc::UnboundedSender<PairMsg>,
    autostart: Autostart,
    last_tick: (Instant, u64),
    shutdown: bool,
    started: Instant,
    dirty: bool,
    /// Save the address book at the next tick (a link came up).
    book_due: bool,
    /// Re-reads done since the last clipboard change, after failed reads.
    read_retries: u32,
    /// The last state save failed (cleared by the next good save).
    save_error: Option<String>,
    /// How many state saves failed in this run (to tell an IPC change that its save failed).
    save_failures: u64,
    /// `state.json` could not be read at start and was set aside as `state.json.bad`.
    state_damaged: bool,
    /// Self-update (Windows exe and macOS dmg builds; test feeds in test profiles). It checks
    /// once, shortly after start, and never installs without the user.
    updater: Option<crate::update::Updater>,
    /// The version this agent reports.
    version: String,
    /// File copies: walk, hash, sends, the fetch, the files folder (file sync spec 5).
    files: FileState,
    /// The other end of `files`' channel, until the main loop takes it.
    xfer_rx: Option<mpsc::UnboundedReceiver<Xfer>>,
}

/// Why the agent could not start.
#[derive(Debug)]
pub struct StartError(pub String);

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Exit code when another agent of this user and profile runs.
pub const EXIT_ALREADY_RUNNING: i32 = 3;

/// Run the agent until it is told to stop. Blocks.
pub fn run(profile: Profile) -> Result<(), StartError> {
    let dir = profile.state_dir();
    std::fs::create_dir_all(&dir).map_err(|e| StartError(format!("state folder: {e}")))?;
    // Single agent per user and profile: a lock file, held for the whole run.
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("agent.lock"))
        .map_err(|e| StartError(format!("lock file: {e}")))?;
    if lock.try_lock().is_err() {
        return Err(StartError("already running".into()));
    }
    crate::log::init(&dir, profile.is_test());
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("uc-agent")
        .build()
        .map_err(|e| StartError(format!("runtime: {e}")))?;
    let r = rt.block_on(async move {
        let listener = match ipc::bind(&profile) {
            Ok(l) => l,
            Err(ipc::BindError::AlreadyRunning) => {
                return Err(StartError("already running".into()));
            }
            Err(e) => return Err(StartError(e.to_string())),
        };
        tracing::info!(
            version = env!("CARGO_PKG_VERSION"),
            os = std::env::consts::OS,
            profile = %profile.name,
            "agent starting"
        );
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(listener.run(tx));
        let (pair_tx, pair_rx) = mpsc::unbounded_channel();
        let (wake_tx, wake_rx) = mpsc::unbounded_channel();
        let mut core = Core::new(profile, dir.clone(), pair_tx, wake_tx)?;
        core.main(rx, pair_rx, wake_rx).await;
        Ok(())
    });
    match &r {
        Err(e) if e.0 == "already running" => tracing::info!("another agent already runs"),
        Err(e) => tracing::error!(error = %e.0, "agent could not start"),
        Ok(()) => {}
    }
    rt.shutdown_timeout(Duration::from_secs(2));
    drop(lock);
    r
}

impl Core {
    fn new(
        profile: Profile,
        dir: PathBuf,
        pair_tx: mpsc::UnboundedSender<PairMsg>,
        wake_tx: mpsc::UnboundedSender<Wake>,
    ) -> Result<Core, StartError> {
        let profile_for_update = profile.clone();
        let store = uc_keys::open_store(profile.keystore_name(), &dir);
        let key = DeviceKey::load_or_create(&*store)
            .map_err(|e| StartError(format!("key store: {e}")))?;
        let (state, mut state_damaged) = StateFile::load_checked(&dir);
        let graph = load_graph(&state);
        if graph.is_none() && !state.group.is_empty() {
            tracing::warn!("saved group could not be read, starting without it");
            // Keep a copy before the next save writes an empty group over it.
            let _ = std::fs::copy(dir.join("state.json"), dir.join("state.json.bad"));
            state_damaged = true;
        }
        if let Some(g) = &graph
            && !g.is_member(&key.public())
            && !g.is_removed(&key.public())
        {
            tracing::warn!("this device's key is not in its saved group (key lost or replaced)");
        }
        let secret = load_secret(&*store, SECRET_NAME);
        let prev_secret = load_secret(&*store, PREV_SECRET_NAME);
        let (clip, changes, clip_error) = match Backend::open(&profile) {
            Ok((b, c)) => (b, c, None),
            Err(e) => {
                tracing::warn!(error = %e.0, "no clipboard");
                (
                    Backend::Fake(uc_clip::FakeClipboard::new()),
                    Changes::Manual,
                    Some(e.0),
                )
            }
        };
        let mut mac_poll = false;
        match changes {
            Changes::Channel(rx) => {
                let tx = wake_tx.clone();
                std::thread::Builder::new()
                    .name("uc-clip-events".into())
                    .spawn(move || {
                        while rx.recv().is_ok() {
                            if tx.send(Wake::ClipChanged).is_err() {
                                // The agent is stopping.
                                return;
                            }
                        }
                        let _ = tx.send(Wake::ClipWatchStopped);
                    })
                    .map_err(|e| StartError(format!("thread: {e}")))?;
            }
            Changes::Poll => mac_poll = true,
            Changes::Manual => {}
        }
        #[cfg(windows)]
        {
            let (rtx, mut rrx) = mpsc::unbounded_channel();
            if crate::win::watch_resume(rtx).is_ok() {
                let tx = wake_tx.clone();
                tokio::spawn(async move {
                    while rrx.recv().await.is_some() {
                        let _ = tx.send(Wake::Resume);
                    }
                });
            }
        }
        let _ = &wake_tx;
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("uc-agent"));
        let autostart = Autostart::new(&profile, exe, &dir);
        let rec = DeviceRecord {
            id: DeviceId(key.public()),
            name: String::new(),
            kind: kind_of_type(own_device_type()),
        };
        let engine = Engine::new(engine_config(&profile, rec));
        let port = if state.port != 0 {
            state.port
        } else {
            profile.default_port()
        };
        let host = Host::with_lockout(Lockout::from(state.pair_lockout).loaded(now_ms()));
        let (mut files, xfer_rx) = FileState::new(profile.files_dir());
        files.send_rate = profile
            .test_env("UCLIP_TEST_SEND_RATE")
            .and_then(|v| v.parse().ok());
        files.serve_refused = profile.test_env("UCLIP_TEST_SERVE_REFUSED").as_deref() == Some("1");
        let mut core = Core {
            profile,
            dir,
            store,
            key,
            state,
            graph,
            secret,
            prev_secret,
            engine,
            clip,
            clip_error,
            mac_poll,
            #[cfg(target_os = "macos")]
            poller: uc_clip_mac::poll::Poller::new(),
            net: None,
            net_error: None,
            fw_rule: None,
            disc: None,
            policy: Policy::default(),
            ifaces: Vec::new(),
            ifaces_seen: false,
            vpn_up: false,
            port,
            host,
            pair_open: false,
            runs: HashMap::new(),
            run_added: HashMap::new(),
            next_run: 1,
            joining: None,
            peers: BTreeMap::new(),
            paths: BTreeMap::new(),
            closing: Vec::new(),
            members_due: None,
            rotate_at: None,
            leaving: None,
            settle_at: None,
            subscribers: Vec::new(),
            clock_ahead: false,
            pair_tx,
            autostart,
            last_tick: (Instant::now(), now_ms()),
            shutdown: false,
            started: Instant::now(),
            dirty: false,
            book_due: false,
            read_retries: 0,
            save_error: None,
            save_failures: 0,
            state_damaged,
            updater: crate::update::UpdateCfg::from_profile(&profile_for_update)
                .map(|c| crate::update::Updater::new(c, profile_for_update.clone())),
            version: crate::update::current_version(&profile_for_update),
            files,
            xfer_rx: Some(xfer_rx),
        };
        core.files_start();
        core.reset_engine();
        core.apply_settings();
        core.default_autostart();
        if core.state.pause == Pause::Forever
            || matches!(core.state.pause, Pause::Until(t) if t > now_ms())
        {
            let evs = core.engine.set_paused(true);
            core.on_engine(evs);
        }
        core.start_net();
        if core.state.settings.tray_icon {
            core.start_tray();
        }
        Ok(core)
    }

    /// Start the app in tray mode (tray spec 3.1). A build without the window has no icon.
    fn start_tray(&self) {
        if let Err(e) = crate::update::open_tray(&self.profile) {
            tracing::info!(error = %e, "no tray icon");
        }
    }

    // ------------------------------------------------------------ setup helpers

    fn me(&self) -> DeviceId {
        DeviceId(self.key.public())
    }

    fn own_name(&self) -> String {
        self.graph
            .as_ref()
            .and_then(|g| g.device_name(&self.key.public()))
            .map(|(_, n)| n)
            .or_else(|| self.state.device_name.clone())
            .unwrap_or_else(default_name)
    }

    fn name_of(&self, d: &DeviceId) -> String {
        self.graph
            .as_ref()
            .and_then(|g| g.device_name(&d.0))
            .map(|(_, n)| n)
            .unwrap_or_else(|| format!("device {}", &hex(&d.0)[..8]))
    }

    fn reset_engine(&mut self) {
        let rec = DeviceRecord {
            id: self.me(),
            name: self.own_name(),
            kind: kind_of_type(own_device_type()),
        };
        let paused = self.engine.is_paused();
        self.files_reset();
        self.engine = Engine::new(engine_config(&self.profile, rec));
        // A fresh engine has no transfers to cancel.
        let _ = self.engine.set_paused(paused);
        self.apply_settings();
        self.sync_engine_group();
    }

    fn sync_engine_group(&mut self) {
        let rec = DeviceRecord {
            id: self.me(),
            name: self.own_name(),
            kind: kind_of_type(own_device_type()),
        };
        self.engine.set_device(rec);
        if let Some(g) = &self.graph {
            let (epoch, _) = g.current_secret();
            self.engine
                .set_group(*g.group_id(), g.heads(), u64::from(epoch));
        } else {
            self.engine.set_group([0; 32], Vec::new(), 0);
        }
    }

    fn apply_settings(&mut self) {
        let s = self.state.settings.clone();
        self.engine.set_sync_images(s.sync_images);
        self.engine
            .set_image_max(s.image_limit_mb as usize * 1024 * 1024);
        let fwd = cfg!(target_os = "macos") && s.forward_apple_clips;
        self.engine.set_forward_remote(fwd);
        self.clip.set_forward_remote(fwd);
        self.engine
            .set_file_max(uc_core::caps::file_limit_bytes(u64::from(s.file_limit_mb)));
        // Off only ends an automatic walk; picked copies and transfers go on.
        self.engine.set_auto_file_copy(s.auto_file_copy);
    }

    /// Decision 2026-11-14 (Max): start at login is on by default and is no longer a setup
    /// question. The first run turns it on once (test profiles in their sandbox). When the
    /// user turned it off, or a setup from before this decision answered it, it is left alone.
    /// A failed try (for example a Mac app still on the disk image) is tried again next start.
    fn default_autostart(&mut self) {
        if self.state.autostart_default_done {
            return;
        }
        if self.state.setup_done
            || !self.state.settings.start_at_login
            || self.autostart.managed_by_windows()
        {
            self.state.autostart_default_done = true;
            self.save();
            return;
        }
        match self.autostart.enable() {
            Ok(()) => {
                self.state.autostart_default_done = true;
                self.save();
            }
            Err(e) => tracing::warn!(error = %e, "start at login could not be turned on"),
        }
    }

    /// Is the Windows inbound rule there? `None` when this build needs none (macOS, Linux,
    /// the Store package). Test profiles never look at the real rule and never elevate:
    /// `UCLIP_TEST_FIREWALL_RULE=present|absent` fakes the answer, else they need none.
    fn check_firewall_rule(&self) -> Option<bool> {
        if let Some(v) = self.profile.test_env("UCLIP_TEST_FIREWALL_RULE") {
            return Some(v != "absent");
        }
        if self.profile.is_test() {
            return None;
        }
        crate::firewall::present(&self.profile)
    }

    fn start_net(&mut self) {
        self.net = None;
        self.peers.clear();
        self.paths.clear();
        self.ifaces_seen = false;
        // Decision 2026-11-14 (Max): the network never waits for a setup answer. The rule
        // (added by the installer) decides: there, accept incoming; missing, outbound-only.
        self.fw_rule = self.check_firewall_rule();
        let fast = self.profile.fast();
        let mut cfg = NetConfig::new(self.key.signing_key().clone(), self.port);
        cfg.accept_incoming = self.fw_rule != Some(false);
        if fast {
            cfg.backoff_max = Duration::from_secs(1);
            cfg.path_delay = Duration::from_secs(2);
        }
        let book = HEXLOWER
            .decode(self.state.book.as_bytes())
            .map(|b| LanBook::decode(&b))
            .unwrap_or_default();
        let disc: Arc<dyn uc_net::Discovery>;
        let binder: Arc<dyn uc_net::SocketBinder>;
        if let Some(ip) = self.profile.test_env("UCLIP_TEST_LOOPBACK") {
            let Ok(ip) = ip.parse::<std::net::IpAddr>() else {
                self.net_error = Some("UCLIP_TEST_LOOPBACK is not an address".into());
                return;
            };
            // The network lab (Linux namespaces) passes a LAN address: a physical /24 there.
            let (kind, prefix) = if ip.is_loopback() {
                (Kind::Loopback, 8)
            } else {
                (Kind::Physical, 24)
            };
            self.policy = Policy {
                allow_loopback: true,
            };
            cfg.policy = self.policy;
            let list = vec![Interface {
                name: "lo-test".into(),
                index: 1,
                description: "test loopback".into(),
                kind,
                up: true,
                addrs: vec![IfAddr { ip, prefix }],
            }];
            self.ifaces = list.clone();
            cfg.ifaces = Some(Arc::new(move || list.clone()));
            disc = Arc::new(uc_net::NoDiscovery);
            binder = Arc::new(uc_net::AddressBinder);
            self.disc = None;
        } else if self.profile.test_env("UCLIP_TEST_NO_LAN").as_deref() == Some("1") {
            // Test profiles only: a machine whose only adapter is a Hyper-V internal switch,
            // so no socket can be bound.
            let list = vec![Interface {
                name: "{test-default-switch}".into(),
                index: 25,
                description: "vEthernet (Default Switch) | Hyper-V Virtual Ethernet Adapter".into(),
                kind: Kind::Virtual,
                up: true,
                addrs: vec![IfAddr {
                    ip: "172.28.64.1".parse().expect("address"),
                    prefix: 20,
                }],
            }];
            cfg.ifaces = Some(Arc::new(move || list.clone()));
            disc = Arc::new(uc_net::NoDiscovery);
            binder = Arc::new(uc_net::AddressBinder);
            self.disc = None;
        } else {
            let d: Arc<dyn uc_net::Discovery> = match uc_net::MdnsSd::new() {
                Ok(m) => Arc::new(m),
                Err(e) => {
                    tracing::warn!(error = %e, "mDNS off");
                    Arc::new(uc_net::NoDiscovery)
                }
            };
            disc = d.clone();
            self.disc = Some(d);
            binder = Arc::new(uc_net::OsBinder);
        }
        match Net::start(cfg, binder, disc, book) {
            Ok(n) => {
                self.net = Some(n);
                self.net_error = None;
                self.push_membership_to_net();
            }
            Err(e) => {
                tracing::error!(error = %e, "network did not start");
                self.net_error = Some(e.to_string());
            }
        }
    }

    fn push_membership_to_net(&mut self) {
        let Some(net) = &self.net else { return };
        let me = self.key.public();
        let members: Vec<DeviceId> = self
            .graph
            .as_ref()
            .map(|g| {
                g.members()
                    .into_iter()
                    .filter(|k| *k != me)
                    .map(DeviceId)
                    .collect()
            })
            .unwrap_or_default();
        net.set_members(members);
        net.set_secrets(Secrets {
            current: self.secret.as_ref().map(|s| s.secret),
            previous: self.prev_secret.as_ref().map(|s| s.secret),
        });
    }

    fn save(&mut self) {
        self.state.group = self
            .graph
            .as_ref()
            .map(|g| g.canonical().map(|c| hex(&c.to_bytes())).collect())
            .unwrap_or_default();
        self.state.port = self.port;
        if let Err(e) = self.state.save(&self.dir) {
            // Log the first failure loudly; the retries at each tick quietly.
            if self.save_error.is_none() {
                tracing::error!(error = %e, "state save failed");
            } else {
                tracing::debug!(error = %e, "state save failed again");
            }
            self.save_error = Some(e.to_string());
            self.save_failures += 1;
            // Keep it dirty, so the next tick tries again.
            self.dirty = true;
        } else {
            if self.save_error.take().is_some() {
                tracing::info!("state saved again");
            }
            self.dirty = false;
        }
    }

    async fn save_book(&mut self) {
        if let Some(net) = &self.net
            && let Some(st) = net.status().await
        {
            self.state.book = hex(&st.book.encode());
        }
        self.save();
    }

    fn store_secret(&mut self, s: GroupSecret) {
        if let Some(old) = self.secret.take()
            && old.epoch != s.epoch
        {
            let _ = self.store.set(PREV_SECRET_NAME, &encode_secret(&old));
            self.prev_secret = Some(old);
        }
        let _ = self.store.set(SECRET_NAME, &encode_secret(&s));
        self.secret = Some(s);
        self.push_membership_to_net();
    }

    fn emit(&mut self, name: &str, data: Value) {
        let ev = Event {
            event: name.into(),
            data,
        };
        self.subscribers.retain(|s| s.send(ev.clone()).is_ok());
    }

    fn activity(
        &mut self,
        kind: &str,
        direction: &str,
        device: Option<String>,
        note: Option<String>,
    ) {
        let a = Activity {
            kind: kind.into(),
            direction: direction.into(),
            device,
            time_ms: now_ms(),
            note,
        };
        self.emit("activity", serde_json::to_value(&a).unwrap_or(Value::Null));
        self.state.push_activity(a);
        self.dirty = true;
    }

    // ------------------------------------------------------------ main loop

    async fn main(
        &mut self,
        mut ipc_rx: mpsc::Receiver<Incoming>,
        mut pair_rx: mpsc::UnboundedReceiver<PairMsg>,
        mut wake_rx: mpsc::UnboundedReceiver<Wake>,
    ) {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut progress = tokio::time::interval(TRANSFER_TICK);
        progress.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut xfer_rx = self.xfer_rx.take().expect("the main loop runs once");
        #[cfg(target_os = "macos")]
        let mut poll_at = tokio::time::Instant::now();
        #[cfg(not(target_os = "macos"))]
        let poll_at = tokio::time::Instant::now();
        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);
        while !self.shutdown {
            let settle = self.settle_at.map(tokio::time::Instant::from_std);
            tokio::select! {
                Some(inc) = ipc_rx.recv() => self.on_ipc(inc).await,
                ev = next_net(&mut self.net) => match ev {
                    Some(ev) => self.on_net(ev),
                    None => { self.net = None; }
                },
                Some(m) = pair_rx.recv() => self.on_pair(m),
                Some(w) = wake_rx.recv() => match w {
                    Wake::ClipChanged => {
                        self.read_retries = 0;
                        self.settle_at = Some(Instant::now() + SETTLE);
                    }
                    Wake::Resume => self.on_resume(),
                    Wake::ClipWatchStopped => {
                        if !self.shutdown {
                            tracing::error!(
                                "clipboard watching stopped; local copies will not sync until a restart"
                            );
                        }
                    }
                },
                _ = sleep_opt(settle) => {
                    self.settle_at = None;
                    self.on_local_change();
                }
                _ = tokio::time::sleep_until(poll_at), if self.mac_poll => {
                    #[cfg(target_os = "macos")]
                    {
                        let count = self.clip.change_count().unwrap_or(0);
                        let p = self.poller.on_poll(now_ms(), count);
                        if p.changed.is_some() {
                            self.read_retries = 0;
                            self.settle_at = Some(Instant::now() + SETTLE);
                        }
                        poll_at = tokio::time::Instant::now() + p.next;
                    }
                }
                Some(m) = xfer_rx.recv() => self.on_xfer(m),
                _ = progress.tick(), if self.files.busy() => self.emit_transfer(),
                _ = tick.tick() => self.on_tick().await,
                _ = &mut ctrl_c => self.shutdown = true,
            }
        }
        self.save_book().await;
        if let Some(n) = &self.net {
            n.shutdown();
        }
        tracing::info!("agent stopped");
    }

    fn on_resume(&mut self) {
        tracing::info!("resume from sleep: reconnect");
        if let Some(n) = &self.net {
            n.wake();
        }
    }

    async fn on_tick(&mut self) {
        let now = Instant::now();
        let wall = now_ms();
        // Sleep and wake on every OS: a tick that comes late, or a wall clock jump.
        let (last, last_wall) = self.last_tick;
        let gap = now.saturating_duration_since(last);
        if gap > Duration::from_secs(5)
            || wall.saturating_sub(last_wall) > gap.as_millis() as u64 + 5000
        {
            self.on_resume();
        }
        self.last_tick = (now, wall);
        let me = self.key.public();
        if let Some(u) = &mut self.updater
            && u.tick(me)
        {
            let v = u.status_json();
            self.emit("update", v);
        }
        // Pause for a time ends.
        if let Pause::Until(t) = self.state.pause
            && wall >= t
        {
            self.state.pause = Pause::Off;
            let evs = self.engine.set_paused(false);
            self.on_engine(evs);
            self.emit("status", json!({"paused": false}));
            self.dirty = true;
        }
        // Runs that ended outside a pairing message can still have moved the lockout ladder.
        self.save_lockout();
        // The open pairing ended by time.
        if self.pair_open && !self.host.is_open(wall) {
            self.pair_open = false;
            if let Some(d) = &self.disc {
                d.withdraw_pair();
            }
            if !self.host.status(wall).succeeded {
                self.emit("pair.closed", json!({"reason": "expired"}));
            }
        }
        // Probation links that ran out of time (5.9: 10 s), and links to removed devices.
        let stale: Vec<DeviceId> = self
            .peers
            .iter()
            .filter(|(_, p)| {
                !p.gate.data_allowed() && now.duration_since(p.started) > Duration::from_secs(10)
            })
            .map(|(d, _)| *d)
            .collect();
        for d in stale {
            self.close_peer(d);
        }
        let due: Vec<DeviceId> = self
            .closing
            .iter()
            .filter(|(t, _)| *t <= now)
            .map(|(_, d)| *d)
            .collect();
        self.closing.retain(|(t, _)| *t > now);
        for d in due {
            self.close_peer(d);
        }
        if self.members_due.is_some_and(|t| t <= now) {
            self.members_due = None;
            self.push_membership_to_net();
        }
        // 5.8: sign the rotate if nobody else did.
        if self.rotate_at.is_some_and(|t| t <= now) {
            self.rotate_at = None;
            self.maybe_rotate();
        }
        if let Some((t, _, _)) = &self.leaving
            && *t <= now
            && let Some((_, p, forget)) = self.leaving.take()
        {
            self.wipe(None, true);
            if forget {
                let _ = self.autostart.disable();
                self.state.settings.start_at_login = false;
            }
            self.save();
            p.answer(Ok(json!({"left": true})));
        }
        if self.dirty {
            self.save();
        }
        if self.book_due
            || (self.started.elapsed() > Duration::from_secs(2) && wall % 60_000 < 1000)
        {
            self.book_due = false;
            // About once a minute: keep remembered addresses on disk.
            self.save_book().await;
        }
    }

    // ------------------------------------------------------------ clipboard

    fn on_local_change(&mut self) {
        if self.clip_error.is_some() {
            return;
        }
        let evs = self.engine.on_local_change(&mut self.clip, now_ms());
        if !evs
            .iter()
            .any(|e| matches!(e, EngineEvent::Skipped(SkipReason::ReadFailed)))
        {
            // The read worked: the next failure starts a fresh series of retries.
            self.read_retries = 0;
        }
        self.on_engine(evs);
    }

    fn on_engine(&mut self, evs: Vec<EngineEvent>) {
        for e in evs {
            if self.on_file_event(&e) {
                continue;
            }
            match e {
                EngineEvent::Sent { kind, peers, .. } => {
                    tracing::info!(kind = kind_text(kind), peers, "sent");
                    self.activity(kind_text(kind), "sent", None, None);
                }
                EngineEvent::Applied { kind, origin, .. } => {
                    let name = self.name_of(&origin);
                    tracing::info!(kind = kind_text(kind), "applied");
                    self.activity(kind_text(kind), "received", Some(name), None);
                }
                EngineEvent::PartDropped(d) => {
                    tracing::info!(format = ?d.format, size = d.size, "part not sent");
                    if d.format == uc_clip::Format::TextPlain {
                        self.activity(
                            "Text",
                            "not sent",
                            None,
                            Some("Text was too large to sync (limit 1 MB).".into()),
                        );
                    } else if d.format == uc_clip::Format::ImagePng {
                        let note = format!(
                            "Image was too large to sync (limit {} MB).",
                            self.state.settings.image_limit_mb
                        );
                        self.activity("Image", "not sent", None, Some(note));
                    }
                }
                EngineEvent::Skipped(SkipReason::TooLarge) => {
                    tracing::info!(reason = "TooLarge", "skipped");
                    self.activity(
                        "Copy",
                        "not sent",
                        None,
                        Some("It was too large to sync.".into()),
                    );
                }
                // Our own echoes are routine: debug. Every other skip explains a copy that did
                // not sync, so it is in the default log (reason only, never content).
                EngineEvent::Skipped(
                    r @ (SkipReason::OwnWrite
                    | SkipReason::Marker
                    | SkipReason::SeenId
                    | SkipReason::SeenContent
                    | SkipReason::Paused),
                ) => tracing::debug!(reason = ?r, "skipped"),
                EngineEvent::Skipped(SkipReason::ReadFailed) => {
                    if self.read_retries < READ_RETRIES {
                        let wait = Duration::from_millis(500) * 2u32.pow(self.read_retries);
                        self.read_retries += 1;
                        tracing::info!(
                            attempt = self.read_retries,
                            wait_ms = wait.as_millis() as u64,
                            "clipboard read failed, trying again"
                        );
                        // A newer change replaces this timer, and resets the count.
                        if self.settle_at.is_none() {
                            self.settle_at = Some(Instant::now() + wait);
                        }
                    } else {
                        tracing::warn!("clipboard read failed, copy not sent");
                        self.activity(
                            "Copy",
                            "not sent",
                            None,
                            Some("Another app kept the clipboard busy.".into()),
                        );
                    }
                }
                EngineEvent::Skipped(r @ SkipReason::Invalid(_)) => {
                    tracing::warn!(reason = ?r, "skipped")
                }
                EngineEvent::PartInvalid { format } => {
                    tracing::warn!(format = ?format, "part could not be read, left out");
                    if format == uc_clip::Format::ImagePng {
                        self.activity(
                            "Image",
                            "not sent",
                            None,
                            Some("The image could not be read. The text was sent.".into()),
                        );
                    }
                }
                EngineEvent::Skipped(r) => tracing::info!(reason = ?r, "skipped"),
                EngineEvent::OwnClockAhead { .. } => self.clock_ahead = true,
                EngineEvent::BadFrame { peer, .. } => {
                    tracing::warn!("bad frame, closing link");
                    self.close_peer(peer);
                }
                EngineEvent::WriteFailed => tracing::warn!("clipboard write failed"),
                EngineEvent::RefusedFutureStamp { .. } => tracing::info!("refused a future stamp"),
                EngineEvent::BadClip { peer, error } => {
                    tracing::warn!(peer = %short_id(&peer.0), error = ?error, "received clip could not be read");
                }
                _ => {}
            }
        }
        self.sync_walk();
    }

    // ------------------------------------------------------------ network

    fn on_net(&mut self, ev: NetEvent) {
        match ev {
            NetEvent::Up(link) => self.on_up(link),
            NetEvent::Down { peer } => {
                tracing::info!(peer = %short_id(&peer.0), "link down");
                self.peers.remove(&peer);
                let evs = self.engine.disconnect(peer);
                self.on_engine(evs);
                self.files_peer_gone(peer);
                self.emit("devices", json!({}));
            }
            NetEvent::Frame { peer, frame } => self.on_frame(peer, frame),
            NetEvent::Path { peer, state } => {
                if self.paths.get(&peer) != Some(&state) {
                    tracing::info!(peer = %short_id(&peer.0), state = ?state, "path");
                }
                self.paths.insert(peer, state);
                self.emit("devices", json!({}));
            }
            NetEvent::Pairing(conn) => self.on_pairing_conn(conn),
            NetEvent::Stream { peer, link, stream } => self.on_stream(peer, link, stream),
            NetEvent::PortChanged(p) => {
                tracing::info!(port = p, "port in use, moved to a new one");
                self.port = p;
                self.dirty = true;
            }
            NetEvent::Interfaces { physical, vpn_up } => {
                let was = self.no_lan();
                if !self.policy.allow_loopback {
                    self.ifaces = physical;
                }
                self.ifaces_seen = true;
                self.vpn_up = vpn_up;
                if self.no_lan() != was {
                    self.emit("status", json!({}));
                }
            }
        }
    }

    fn gate_hello(&self) -> Option<Message> {
        let g = self.graph.as_ref()?;
        let (epoch, _) = g.current_secret();
        Some(Message::Hello(Hello {
            protocol: msg::PROTOCOL_VERSION,
            app_version: self.version.clone(),
            device: DeviceRecord {
                id: self.me(),
                name: self.own_name(),
                kind: kind_of_type(own_device_type()),
            },
            role: Role::Full,
            chain_heads: g.heads(),
            epoch: u64::from(epoch),
            wall_ms: now_ms(),
            group: *g.group_id(),
            // The same extension as the engine's Hello (file sync spec 3).
            ext: self.engine.hello_ext(),
        }))
    }

    fn on_up(&mut self, link: Link) {
        let peer = link.peer;
        tracing::info!(peer = %short_id(&peer.0), "link up");
        // Keep the address that worked on disk soon, not only once a minute, so a crash
        // right after pairing or an address change can still dial on the next start.
        self.book_due = true;
        let Some(g) = &self.graph else {
            link.close(2, b"no group");
            return;
        };
        if g.is_removed(&peer.0) {
            // Tell a removed device, then close (it reads nothing from it). Without this it
            // would never learn that it was removed.
            let all: Vec<Vec<u8>> = g.canonical().map(|c| c.to_bytes()).collect();
            let _ = link.send(msg::encode(&Message::Membership { changes: all }));
            let _ = link.send(msg::encode(&Message::Removed));
            self.peers.insert(
                peer,
                PeerLink {
                    link,
                    gate: Gate::new(peer.0),
                    started: Instant::now(),
                    bytes: 0,
                    changes: 0,
                    heads: Vec::new(),
                    epoch: 0,
                },
            );
            self.closing.push((Instant::now() + FLUSH, peer));
            return;
        }
        let Some(hello) = self.gate_hello() else {
            return;
        };
        let _ = link.send(msg::encode(&hello));
        let evs = self.engine.disconnect(peer);
        self.on_engine(evs);
        self.files_peer_gone(peer);
        self.peers.insert(
            peer,
            PeerLink {
                link,
                gate: Gate::new(peer.0),
                started: Instant::now(),
                bytes: 0,
                changes: 0,
                heads: Vec::new(),
                epoch: 0,
            },
        );
    }

    fn close_peer(&mut self, peer: DeviceId) {
        if let Some(p) = self.peers.remove(&peer) {
            p.link.close(2, b"closed");
        }
        let evs = self.engine.disconnect(peer);
        self.on_engine(evs);
        self.files_peer_gone(peer);
        if let Some(n) = &self.net {
            n.end_probation(peer);
        }
    }

    fn on_frame(&mut self, peer: DeviceId, frame: Vec<u8>) {
        let Some(p) = self.peers.get_mut(&peer) else {
            return;
        };
        if self.closing.iter().any(|(_, d)| *d == peer) {
            return;
        }
        if !p.gate.data_allowed() {
            p.bytes += frame.len();
            self.on_gate_frame(peer, frame);
            return;
        }
        match msg::frame_type(&frame) {
            Some(FrameType::Membership) => {
                if let Ok(Some(Message::Membership { changes })) = msg::decode(&frame) {
                    self.on_membership(peer, changes);
                }
            }
            Some(FrameType::AddrUpdate) => {
                if let (Some(n), Ok(Some(Message::AddrUpdate { signed }))) =
                    (&self.net, msg::decode(&frame))
                {
                    n.addr_update(peer, signed);
                }
            }
            Some(FrameType::Secret) => self.on_secret_msg(peer, &frame),
            _ => {
                let evs = self.engine.on_frame(peer, &frame, &mut self.clip, now_ms());
                self.on_engine(evs);
            }
        }
    }

    /// Protocol 5.9 steps 1 to 5.
    fn on_gate_frame(&mut self, peer: DeviceId, frame: Vec<u8>) {
        let Some(g) = self.graph.as_ref() else {
            self.close_peer(peer);
            return;
        };
        let Some(p) = self.peers.get_mut(&peer) else {
            return;
        };
        let unknown = !g.is_member(&peer.0);
        if unknown && p.bytes > UNKNOWN_PEER_MAX_BYTES {
            self.close_peer(peer);
            return;
        }
        match msg::decode(&frame) {
            Ok(Some(Message::Hello(h))) => {
                if p.gate.on_hello(g, &h.group).is_err() {
                    tracing::info!("gate closed at hello");
                    self.close_peer(peer);
                    return;
                }
                p.heads = h.chain_heads.clone();
                p.epoch = h.epoch;
                let missing: Vec<Vec<u8>> = g
                    .missing_for(&h.chain_heads)
                    .iter()
                    .map(|c| c.to_bytes())
                    .collect();
                p.send(&Message::Membership { changes: missing });
            }
            Ok(Some(Message::Membership { changes })) => {
                if !p.gate.membership_allowed() {
                    self.close_peer(peer);
                    return;
                }
                p.changes += changes.len();
                if unknown && p.changes > UNKNOWN_PEER_MAX_CHANGES {
                    self.close_peer(peer);
                    return;
                }
                self.on_membership(peer, changes);
                let (Some(g), Some(p)) = (self.graph.as_ref(), self.peers.get_mut(&peer)) else {
                    return;
                };
                if p.gate.after_membership_sync(g).is_err() {
                    tracing::info!("gate closed after membership sync");
                    self.close_peer(peer);
                    return;
                }
                self.open_peer(peer);
            }
            _ => {
                // Nothing else before the gate opens.
                self.close_peer(peer);
            }
        }
    }

    fn open_peer(&mut self, peer: DeviceId) {
        let Some(p) = self.peers.get(&peer) else {
            return;
        };
        let link = p.link.clone();
        let peer_epoch = p.epoch;
        // A probation peer that turned out to be a member becomes a normal link.
        self.push_membership_to_net();
        if let Some(n) = &self.net {
            n.ready(peer);
        }
        let evs = self.engine.connect(Box::new(link.peer_handle()), now_ms());
        self.on_engine(evs);
        // 5.8: ask for a secret we lack.
        if let Some(g) = &self.graph {
            let (epoch, _) = g.current_secret();
            let have = self.secret.as_ref().is_some_and(|s| g.secret_matches(s));
            if !have && u64::from(epoch) <= peer_epoch.max(u64::from(epoch)) {
                link.send(msg::encode(&Message::WantSecret { epoch })).ok();
            }
        }
        self.emit("devices", json!({}));
    }

    fn on_secret_msg(&mut self, peer: DeviceId, frame: &[u8]) {
        let Some(g) = &self.graph else { return };
        match msg::decode(frame) {
            Ok(Some(Message::WantSecret { epoch })) => {
                if !g.is_member(&peer.0) {
                    return;
                }
                if let Some(s) = self.secret.as_ref().filter(|s| s.epoch == epoch)
                    && let Some(p) = self.peers.get(&peer)
                {
                    p.send(&Message::Secret {
                        epoch,
                        secret: s.secret,
                    });
                }
            }
            Ok(Some(Message::Secret { epoch, secret })) => {
                let s = GroupSecret { epoch, secret };
                if g.secret_matches(&s) {
                    self.store_secret(s);
                    self.sync_engine_group();
                }
            }
            _ => {}
        }
    }

    /// Changes arrived (5.9 step 4, or later).
    fn on_membership(&mut self, from: DeviceId, changes: Vec<Vec<u8>>) {
        let Some(g) = self.graph.as_mut() else { return };
        let before: BTreeSet<[u8; 32]> = g.canonical().map(|c| *c.id()).collect();
        let mut any = false;
        for c in &changes {
            if matches!(g.insert_bytes(c), Ok(Inserted::Stored)) {
                any = true;
            }
        }
        if !any {
            return;
        }
        let new: Vec<Vec<u8>> = g
            .canonical()
            .filter(|c| !before.contains(c.id()))
            .map(|c| c.to_bytes())
            .collect();
        self.spread(&new, Some(from));
        self.after_graph_change();
    }

    /// Send new changes to every open peer (except `except`).
    fn spread(&self, changes: &[Vec<u8>], except: Option<DeviceId>) {
        if changes.is_empty() {
            return;
        }
        let m = Message::Membership {
            changes: changes.to_vec(),
        };
        for (d, p) in &self.peers {
            if Some(*d) != except && p.gate.data_allowed() {
                p.send(&m);
            }
        }
    }

    /// 5.7: evaluate again and act.
    fn after_graph_change(&mut self) {
        let Some(g) = &self.graph else { return };
        if self_action(g, &self.key.public()) == SelfAction::WipeAndRekey {
            tracing::warn!("this device was removed by another device");
            self.wipe(Some(Notice::Removed), true);
            self.emit("removed", json!({}));
            return;
        }
        let members = g.members();
        // Close links to keys that are no longer members (after a short flush).
        let gone: Vec<DeviceId> = self
            .peers
            .iter()
            .filter(|(d, p)| p.gate.data_allowed() && !members.contains(&d.0))
            .map(|(d, _)| *d)
            .collect();
        let mut engine_evs = Vec::new();
        for d in &gone {
            if let Some(p) = self.peers.get(d) {
                p.send(&Message::Removed);
            }
            // The removed peer's link closes, which ends its transfers. The engine may
            // start a queued send for another peer here, so its events are handled below.
            engine_evs.extend(self.engine.disconnect(*d));
            self.closing.push((Instant::now() + FLUSH, *d));
        }
        for p in self.peers.values_mut() {
            let _ = p.gate.recheck(g);
        }
        // 5.8: a secret we lack.
        let (epoch, _) = g.current_secret();
        let have = self.secret.as_ref().is_some_and(|s| g.secret_matches(s));
        let needs_rotate = g.needs_rotate();
        if !have {
            for p in self.peers.values().filter(|p| p.gate.data_allowed()) {
                p.send(&Message::WantSecret { epoch });
            }
        }
        if gone.is_empty() {
            self.push_membership_to_net();
        } else {
            self.members_due = Some(Instant::now() + FLUSH);
        }
        if needs_rotate && self.rotate_at.is_none() {
            let wait = if self.profile.fast() {
                1 + OsRng.next_u32() % 2
            } else {
                30 + OsRng.next_u32() % 31
            };
            self.rotate_at = Some(Instant::now() + Duration::from_secs(u64::from(wait)));
        }
        self.sync_engine_group();
        for d in gone {
            self.files_peer_gone(d);
        }
        self.on_engine(engine_evs);
        self.emit("devices", json!({}));
        self.save();
    }

    fn maybe_rotate(&mut self) {
        let Some(g) = self.graph.as_mut() else { return };
        if !g.needs_rotate() {
            return;
        }
        if let Ok((c, s)) = g.sign_rotate(&self.key, now_s(), &mut OsRng) {
            let b = c.to_bytes();
            self.store_secret(s);
            self.spread(&[b], None);
            self.after_graph_change();
        }
    }

    /// Forget the group (removed, or leaving). `rekey` makes a new device key (5.7).
    fn wipe(&mut self, notice: Option<Notice>, rekey: bool) {
        for d in self.peers.keys().copied().collect::<Vec<_>>() {
            self.close_peer(d);
        }
        self.graph = None;
        self.secret = None;
        self.prev_secret = None;
        let _ = self.store.delete(SECRET_NAME);
        let _ = self.store.delete(PREV_SECRET_NAME);
        self.state.group.clear();
        self.state.book.clear();
        self.state.notice = notice;
        self.rotate_at = None;
        if rekey && let Ok(k) = uc_pair::chain::rekey(&*self.store) {
            self.key = k;
        }
        self.host.cancel();
        self.pair_open = false;
        self.save();
        self.reset_engine();
        self.start_net();
    }

    // ------------------------------------------------------------ pairing

    fn pair_ifaces(
        &self,
        remote: std::net::IpAddr,
    ) -> (std::net::IpAddr, Vec<uc_pair::lan::Iface>) {
        if self.policy.allow_loopback {
            // Test profiles on loopback: uc-net already applied the LAN rule with its
            // loopback test policy. uc-pair's rule has no loopback switch, so the
            // connection is shown to it as one private on-link address.
            let fake = std::net::IpAddr::from([10, 255, 255, 1]);
            let ifc = uc_pair::lan::Iface {
                index: 1,
                physical: true,
                addr: std::net::IpAddr::from([10, 255, 255, 2]),
                prefix_len: 24,
            };
            return (fake, vec![ifc]);
        }
        let list = self
            .ifaces
            .iter()
            .flat_map(|i| {
                i.addrs.iter().map(move |a| uc_pair::lan::Iface {
                    index: i.index,
                    physical: i.kind == Kind::Physical,
                    addr: a.ip,
                    prefix_len: a.prefix,
                })
            })
            .collect();
        (remote, list)
    }

    fn on_pairing_conn(&mut self, conn: uc_net::Conn) {
        let Some(mut t) = pairing::transport(&conn) else {
            conn.conn.close(0u32.into(), b"");
            return;
        };
        let (remote, ifaces) = self.pair_ifaces(t.remote);
        t.remote = remote;
        match self.host.accept(&t, &ifaces, self.key.public(), now_ms()) {
            Ok(run) => {
                let id = self.next_run;
                self.next_run += 1;
                self.runs.insert(id, run);
                tokio::spawn(pairing::inviter(conn, id, self.pair_tx.clone()));
            }
            Err(uc_pair::session::Refuse::Busy) => {
                let c = conn.conn.clone();
                tokio::spawn(async move {
                    if let Ok((mut s, _)) = c.accept_bi().await {
                        let _ = s.write_all(&uc_pair::frame::abort(AbortReason::Busy)).await;
                        let _ = s.finish();
                    }
                    c.close(0u32.into(), b"");
                });
            }
            Err(_) => conn.conn.close(0u32.into(), b""),
        }
    }

    fn on_pair(&mut self, m: PairMsg) {
        match m {
            PairMsg::InviterFrame { run, frame, reply } => {
                let step = self.inviter_step(run, frame);
                let _ = reply.send(step);
            }
            PairMsg::InviterClosed { run } => {
                if let Some(mut r) = self.runs.remove(&run) {
                    r.on_close();
                }
                let st = self.host.status(now_ms());
                self.save_lockout();
                if let Some(name) = self.run_added.remove(&run) {
                    self.pair_open = false;
                    if let Some(d) = &self.disc {
                        d.withdraw_pair();
                    }
                    self.emit("pair.added", json!({ "name": name }));
                } else if st.failed_code_tries > 0 || st.code_dead {
                    self.emit(
                        "pair.tried",
                        json!({
                            "notice": NOTICE_FAILED_TRY,
                            "code_dead": st.code_dead,
                            "lockout": (st.lockout_left_ms > 0)
                                .then(|| lockout_notice(st.lockout_left_ms)),
                            "lockout_ms": st.lockout_left_ms,
                        }),
                    );
                }
            }
            PairMsg::Joined {
                joined,
                inviter,
                remote,
                reply,
            } => {
                let ok = self.store_joined(*joined, inviter, remote);
                let _ = reply.send(ok);
            }
            PairMsg::JoinFinished(r) => {
                if let Some(p) = self.joining.take() {
                    p.answer(r.map(|n| json!({ "name": n })));
                }
            }
        }
    }

    /// Saves the lockout ladder when it changed, so an agent restart keeps it (protocol 4.6).
    fn save_lockout(&mut self) {
        let l = PairLockout::from(self.host.lockout());
        if l != self.state.pair_lockout {
            self.state.pair_lockout = l;
            self.save();
        }
    }

    fn inviter_step(&mut self, run: u64, frame: uc_pair::frame::Frame) -> InviterStep {
        let addr_book = Vec::new();
        let is_confirm = frame.ty == T_CONFIRM;
        let Some(r) = self.runs.get_mut(&run) else {
            return InviterStep::Abort(None);
        };
        let step = match (self.graph.as_mut(), self.secret.as_ref()) {
            (Some(graph), Some(secret)) => r.on_frame(
                frame,
                now_ms(),
                &mut OsRng,
                Some(GroupCtx {
                    graph,
                    secret,
                    key: &self.key,
                    time: now_s(),
                    addr_book: &addr_book,
                }),
            ),
            _ => r.on_frame(frame, now_ms(), &mut OsRng, None),
        };
        if is_confirm && let InviterStep::SendState { added, .. } = &step {
            let peer = *r.peer_key();
            if let Some(c) = added {
                let b = c.to_bytes();
                self.spread(&[b], None);
            }
            let name = self.name_of(&DeviceId(peer));
            self.run_added.insert(run, name);
            self.state.notice = None;
            self.after_graph_change();
        }
        self.save_lockout();
        step
    }

    fn store_joined(
        &mut self,
        j: uc_pair::session::Joined,
        inviter: [u8; 32],
        remote: SocketAddr,
    ) -> bool {
        if self.joining.is_none() {
            return false;
        }
        // 4.7: leave the old group first (the user said yes to "Leave your other devices").
        if let Some(g) = self.graph.as_mut()
            && g.group_id() != j.graph.group_id()
            && g.members().len() > 1
            && let Ok((rm, _)) = g.sign_remove(&self.key, self.key.public(), now_s(), &mut OsRng)
        {
            let b = rm.to_bytes();
            self.spread(&[b], None);
        }
        for d in self.peers.keys().copied().collect::<Vec<_>>() {
            self.closing.push((Instant::now() + FLUSH, d));
        }
        let members: Vec<DeviceId> = j.graph.members().into_iter().map(DeviceId).collect();
        let book_bytes = j.addr_book.clone();
        self.graph = Some(j.graph);
        self.prev_secret = None;
        let _ = self.store.delete(PREV_SECRET_NAME);
        self.secret = None;
        self.store_secret(j.secret.clone());
        self.state.notice = None;
        self.state.device_name = None;
        self.reset_engine();
        self.members_due = Some(Instant::now() + FLUSH);
        if let Some(n) = &self.net {
            n.add_addr(DeviceId(inviter), remote);
            let mut tmp = LanBook::new();
            tmp.import_announcements(&book_bytes, &members, now_ms(), &self.policy);
            for d in &members {
                for a in tmp.dial_list(d) {
                    n.add_addr(*d, a);
                }
            }
        }
        self.after_graph_change();
        let name = self.name_of(&DeviceId(inviter));
        if let Some(p) = self.joining.take() {
            p.answer(Ok(json!({ "name": name })));
        }
        self.emit("pair.joined", json!({ "name": name }));
        true
    }

    // ------------------------------------------------------------ IPC

    async fn on_ipc(&mut self, inc: Incoming) {
        let Incoming { req, reply, events } = inc;
        let id = req.id;
        let p = req.params.clone();
        let call = req.call.as_str();
        if !ipc::CALLS.contains(&call)
            && !ipc::SERVICE_CALLS.contains(&call)
            && !(self.profile.is_test() && ipc::DEBUG_CALLS.contains(&call))
        {
            let _ = reply.send(Reply::from_result(
                id,
                Err(IpcError::new(
                    "unknown-call",
                    format!("Unknown call \"{call}\"."),
                )),
            ));
            return;
        }
        let pending = Pending { id, reply };
        let fails_before = self.save_failures;
        let r = match call {
            "subscribe" => {
                self.subscribers.push(events);
                Ok(json!({"subscribed": true}))
            }
            "version" => Ok(json!({"version": self.version})),
            "quit" => {
                tracing::info!("asked to quit");
                // The tray icon ends with the agent (tray spec 3.4).
                self.emit("quitting", json!({}));
                self.shutdown = true;
                Ok(json!({"quitting": true}))
            }
            "status" => Ok(self.status_json().await),
            "devices.list" => Ok(self.devices_json()),
            "devices.rename" => self.rename(&p),
            "devices.remove" => self.remove(&p),
            "pause.set" => self.pause(&p),
            "settings.get" => Ok(self.settings_json()),
            "settings.set" => self.settings_set(&p),
            "files.copy" => self.files_copy(&p),
            "transfer.stop" => Ok(self.transfer_stop()),
            "diagnostics" => Ok(self.diagnostics().await),
            "pair.start" => self.pair_start().await,
            "pair.cancel" => {
                self.host.cancel();
                self.pair_open = false;
                if let Some(d) = &self.disc {
                    d.withdraw_pair();
                }
                self.emit("pair.closed", json!({"reason": "cancelled"}));
                Ok(json!({"cancelled": true}))
            }
            "pair.join" => match self.pair_join(&p) {
                Ok(()) => {
                    self.joining = Some(pending);
                    return;
                }
                Err(e) => Err(e),
            },
            "leave" => match self.leave(&p) {
                Ok(()) => {
                    let forget = p.get("forget").and_then(Value::as_bool).unwrap_or(true);
                    self.leaving = Some((Instant::now() + FLUSH, pending, forget));
                    return;
                }
                Err(e) => Err(e),
            },
            "setup.status" => Ok(self.setup_status()),
            "setup.step" => self.setup_step(&p),
            "firewall.check" => Ok(self.firewall_check()),
            "update.status" => Ok(self.update_status()),
            "update.apply" => self.update_apply(),
            "update.later" => {
                if let Some(u) = &mut self.updater {
                    u.later();
                }
                let v = self.update_status();
                self.emit("update", v.clone());
                Ok(v)
            }
            "debug.copy" => {
                let t = p.get("text").and_then(Value::as_str).unwrap_or_default();
                match self.clip.debug_copy(t) {
                    Ok(seq) => {
                        if matches!(self.clip, Backend::Fake(_)) {
                            self.read_retries = 0;
                            self.settle_at = Some(Instant::now() + SETTLE);
                        }
                        Ok(json!({"seq": seq}))
                    }
                    Err(e) => Err(IpcError::new("clipboard", e.to_string())),
                }
            }
            "debug.copy_files" => {
                let paths: Vec<PathBuf> = p
                    .get("paths")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(PathBuf::from)
                    .collect();
                match self.clip.debug_copy_files(&paths) {
                    Ok(seq) => {
                        self.read_retries = 0;
                        self.settle_at = Some(Instant::now() + SETTLE);
                        Ok(json!({"seq": seq}))
                    }
                    Err(e) => Err(IpcError::new("clipboard", e.to_string())),
                }
            }
            "debug.clipboard" => Ok(json!({
                "text": self.clip.debug_text(),
                "files": self.clip.debug_files(),
                "png": self.clip.debug_png().map(|b| HEXLOWER.encode(blake3::hash(&b).as_bytes())),
            })),
            "debug.wake" => {
                self.on_resume();
                Ok(json!({}))
            }
            "debug.shutdown" => {
                self.shutdown = true;
                Ok(json!({}))
            }
            _ => Err(IpcError::new("unknown-call", "Unknown call.")),
        };
        // A change the user made must be on disk before we say it worked.
        let r = if CHANGE_CALLS.contains(&call) && r.is_ok() {
            if self.dirty {
                self.save();
            }
            if self.save_failures > fails_before {
                Err(IpcError::new(
                    "save-failed",
                    format!(
                        "The change could not be saved, so it is lost after a restart ({}).",
                        self.save_error.clone().unwrap_or_default()
                    ),
                ))
            } else {
                r
            }
        } else {
            r
        };
        pending.answer(r);
    }

    fn find_device(&self, p: &Value) -> Result<DeviceId, IpcError> {
        let q = p
            .get("device")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        let g = self
            .graph
            .as_ref()
            .ok_or_else(|| IpcError::new("no-group", "This device has no other devices yet."))?;
        let members: Vec<[u8; 32]> = g.members().into_iter().collect();
        let by_name: Vec<&[u8; 32]> = members
            .iter()
            .filter(|k| {
                g.device_name(k)
                    .is_some_and(|(_, n)| n.eq_ignore_ascii_case(&q))
            })
            .collect();
        if by_name.len() == 1 {
            return Ok(DeviceId(*by_name[0]));
        }
        if by_name.len() > 1 {
            return Err(IpcError::new(
                "ambiguous",
                "Two devices have that name. Use the ID from \"uc devices\".",
            ));
        }
        let ql = q.to_ascii_lowercase();
        if ql.len() >= 4 {
            let by_id: Vec<&[u8; 32]> = members
                .iter()
                .filter(|k| hex(&k[..]).starts_with(&ql))
                .collect();
            if by_id.len() == 1 {
                return Ok(DeviceId(*by_id[0]));
            }
        }
        Err(IpcError::new(
            "not-found",
            format!("There is no device \"{q}\". See \"uc devices\"."),
        ))
    }

    fn rename(&mut self, p: &Value) -> Result<Value, IpcError> {
        let d = self.find_device(p)?;
        let name = p.get("name").and_then(Value::as_str).unwrap_or_default();
        self.rename_device(d, name)
    }

    fn rename_device(&mut self, d: DeviceId, name: &str) -> Result<Value, IpcError> {
        let name = clean_name(name)
            .ok_or_else(|| IpcError::new("bad-name", "A name has 1 to 64 characters."))?;
        let g = self
            .graph
            .as_mut()
            .ok_or_else(|| IpcError::new("no-group", "This device has no other devices yet."))?;
        let c = g
            .sign_rename(&self.key, d.0, &name, now_s())
            .map_err(|_| IpcError::new("rename-failed", "The name could not be changed."))?;
        let b = c.to_bytes();
        self.spread(&[b], None);
        self.after_graph_change();
        Ok(json!({"renamed": hex(&d.0), "name": name}))
    }

    fn remove(&mut self, p: &Value) -> Result<Value, IpcError> {
        let d = self.find_device(p)?;
        if d == self.me() {
            return Err(IpcError::new(
                "self",
                "To remove this device, use \"uc leave\".",
            ));
        }
        let name = self.name_of(&d);
        let g = self.graph.as_mut().expect("checked in find_device");
        let (rm, rot) = g
            .sign_remove(&self.key, d.0, now_s(), &mut OsRng)
            .map_err(|_| IpcError::new("remove-failed", "The device could not be removed."))?;
        let mut out = vec![rm.to_bytes()];
        if let Some((c, s)) = rot {
            out.push(c.to_bytes());
            self.store_secret(s);
        }
        // The removed device gets the changes too, so it knows (then its link closes).
        let m = Message::Membership { changes: out };
        for p in self.peers.values().filter(|p| p.gate.data_allowed()) {
            p.send(&m);
        }
        self.after_graph_change();
        Ok(json!({"removed": hex(&d.0), "name": name}))
    }

    fn pause(&mut self, p: &Value) -> Result<Value, IpcError> {
        let mode = p.get("mode").and_then(Value::as_str).unwrap_or("forever");
        self.state.pause = match mode {
            "off" => Pause::Off,
            "forever" => Pause::Forever,
            "hour" => Pause::Until(now_ms() + 3_600_000),
            "minutes" => {
                let m = p
                    .get("minutes")
                    .and_then(Value::as_u64)
                    .unwrap_or(60)
                    .clamp(1, 7 * 24 * 60);
                Pause::Until(now_ms() + m * 60_000)
            }
            _ => {
                return Err(IpcError::new(
                    "bad-mode",
                    "Use off, forever, hour or minutes.",
                ));
            }
        };
        let evs = self.engine.set_paused(self.state.pause != Pause::Off);
        self.on_engine(evs);
        self.save();
        self.emit("status", json!({}));
        Ok(serde_json::to_value(self.state.pause).unwrap_or(Value::Null))
    }

    fn settings_json(&self) -> Value {
        let mut m = serde_json::Map::new();
        for k in settings::KEYS {
            if k.mac_only && !cfg!(target_os = "macos") {
                continue;
            }
            let v = if k.name == "device-name" {
                self.own_name()
            } else {
                self.state.settings.get(k.name).unwrap_or_default()
            };
            m.insert(k.name.into(), json!({"value": v, "label": k.label}));
        }
        Value::Object(m)
    }

    fn settings_set(&mut self, p: &Value) -> Result<Value, IpcError> {
        let key = p.get("key").and_then(Value::as_str).unwrap_or_default();
        let value = p.get("value").and_then(Value::as_str).unwrap_or_default();
        if key == "device-name" {
            if self.graph.is_some() {
                let me = self.me();
                return self.rename_device(me, value);
            }
            let n = clean_name(value)
                .ok_or_else(|| IpcError::new("bad-name", "A name has 1 to 64 characters."))?;
            self.state.device_name = Some(n);
            self.sync_engine_group();
            self.save();
            return Ok(json!({"key": key, "value": value}));
        }
        self.state
            .settings
            .set(key, value)
            .map_err(|m| IpcError::new("bad-setting", m))?;
        if key == "start-at-login" {
            let r = if self.state.settings.start_at_login {
                self.autostart.enable()
            } else {
                self.autostart.disable()
            };
            r.map_err(|e| IpcError::new("autostart", format!("Start at login: {e}")))?;
        }
        self.apply_settings();
        self.save();
        let value = self.state.settings.get(key);
        self.emit("settings", json!({"key": key, "value": value}));
        if key == "tray-icon" && self.state.settings.tray_icon {
            self.start_tray();
        }
        Ok(json!({"key": key, "value": value}))
    }

    async fn pair_start(&mut self) -> Result<Value, IpcError> {
        if self.net.is_none() {
            return Err(IpcError::new(
                "no-network",
                "The network is not on yet. Run \"uc setup\" first.",
            ));
        }
        if self.graph.is_none() {
            let (g, s) = Graph::create(
                &self.key,
                own_device_type(),
                &self.own_name(),
                now_s(),
                &mut OsRng,
            )
            .map_err(|_| IpcError::new("group", "Could not make a group."))?;
            self.graph = Some(g);
            self.store_secret(s);
            self.state.notice = None;
            self.after_graph_change();
        }
        let g = self.graph.as_ref().expect("made above");
        let opened = self.host.open(g, now_ms(), &mut OsRng).map_err(|_| {
            IpcError::new(
                "group-full",
                "Your group already has 8 devices, the most it can have.",
            )
        })?;
        self.pair_open = true;
        let st = match &self.net {
            Some(n) => n.status().await,
            None => None,
        };
        let port = st.as_ref().map_or(self.port, |s| s.port);
        if let Some(d) = &self.disc
            && let Err(e) = d.advertise_pair(&opened.instance, port)
        {
            tracing::warn!(error = %e, "pairing mDNS");
        }
        let mut addrs: Vec<std::net::IpAddr> = st
            .map(|s| s.sockets.iter().map(|(_, a, _)| a.ip()).collect())
            .unwrap_or_default();
        addrs.sort_by_key(|a| a.is_ipv6());
        addrs.dedup();
        addrs.truncate(uc_pair::qr::MAX_ADDRS);
        let qr = if addrs.is_empty() {
            None
        } else {
            Some(
                uc_pair::qr::QrPayload {
                    inviter_key: self.key.public(),
                    qr_secret: opened.qr_secret,
                    port,
                    addrs,
                }
                .to_text(),
            )
        };
        let left = self.host.status(now_ms()).lockout_left_ms;
        Ok(json!({
            "code": opened.code.as_ref().and_then(pairing::show_code),
            "qr": qr,
            "expires_ms": opened.expires_ms,
            "port": port,
            "notice": (left > 0).then(|| lockout_notice(left)),
            "lockout_ms": left,
        }))
    }

    fn pair_join(&mut self, p: &Value) -> Result<(), IpcError> {
        if self.joining.is_some() {
            return Err(IpcError::new("busy", "A pairing is already running."));
        }
        let code = p
            .get("code")
            .and_then(Value::as_str)
            .and_then(pairing::parse_code)
            .ok_or_else(|| IpcError::new("bad-code", "Type the 6 digits of the code."))?;
        let leave = p
            .get("leave_other")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if let Some(g) = &self.graph
            && g.members().len() > 1
            && !leave
        {
            return Err(IpcError::new(
                "in-group",
                "Leave your other devices and join these?",
            ));
        }
        let Some(net) = &self.net else {
            return Err(IpcError::new(
                "no-network",
                "The network is not on yet. Run \"uc setup\" first.",
            ));
        };
        if self.no_lan() {
            return Err(IpcError::new("no-lan", NO_LAN_TEXT));
        }
        let record = uc_pair::frame::DeviceRecord::new(own_device_type(), &self.own_name())
            .ok_or_else(|| IpcError::new("bad-name", "This device's name is not valid."))?;
        // Test profiles only: where to look (no mDNS on loopback). The CLI passes
        // `UCLIP_TEST_PAIR_ADDRS` from its own environment.
        let fixed: Vec<SocketAddr> = if self.profile.is_test() {
            p.get("test_addrs")
                .and_then(Value::as_str)
                .map(|s| s.split(',').filter_map(|a| a.trim().parse().ok()).collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let finder = Finder {
            fixed,
            disc: self.disc.clone(),
            policy: self.policy,
            ifaces: self.ifaces.clone(),
        };
        tokio::spawn(pairing::join(
            net.handle(),
            Box::new(finder),
            code,
            self.key.public(),
            record,
            self.pair_tx.clone(),
            Duration::from_secs(20),
        ));
        Ok(())
    }

    fn leave(&mut self, _p: &Value) -> Result<(), IpcError> {
        if let Some(g) = self.graph.as_mut()
            && g.members().len() > 1
        {
            let (rm, _) = g
                .sign_remove(&self.key, self.key.public(), now_s(), &mut OsRng)
                .map_err(|_| IpcError::new("leave-failed", "Could not leave the group."))?;
            let b = rm.to_bytes();
            self.spread(&[b], None);
        }
        Ok(())
    }

    // ------------------------------------------------------------ setup

    /// The setup rows that are left. Decision 2026-11-14 (Max): nothing that shows a system
    /// prompt later is asked here. The Windows firewall rule comes from the installer, and
    /// start at login is on by default (a switch in the settings, see `default_autostart`),
    /// so no OS has a row of its own. Rows appear only when something is blocked: macOS
    /// Local Network or paste access, or "desktop" when the clipboard cannot be used.
    fn setup_rows(&self) -> Vec<Value> {
        let mut rows = Vec::new();
        // macOS: the agent is a launchd job, and macOS lets it use the local network and read
        // the clipboard without a prompt (checked 2026-11-14 on macOS 27.2 with a never-seen
        // signing identity: Local Network entry created as allowed, paste access "allow").
        // So these rows appear only when macOS actually blocks something; no Allow button
        // that does nothing.
        if cfg!(target_os = "macos") {
            if self.net_error.is_some() {
                rows.push(json!({
                    "row": "local-network",
                    "title": "Find devices on your network",
                    "why": "Needed to reach devices on your network.",
                    "if_refused": "Sync does not work on the network until this is allowed in System Settings > Privacy & Security > Local Network.",
                    "state": "refused",
                }));
            }
            let access = self.clip.paste_access().unwrap_or("allow");
            if access != "allow" {
                rows.push(json!({
                    "row": "paste",
                    "title": "Read what you copy",
                    "why": "The app must read the clipboard without asking each time.",
                    "if_refused": "Sync from this Mac does not work. Sync to this Mac still works.",
                    "state": match access { "deny" | "ask" => "refused", _ => "todo" },
                    "access": access,
                }));
            }
        }
        if let Some(e) = &self.clip_error {
            rows.push(json!({
                "row": "desktop",
                "title": "This desktop blocks background clipboard access.",
                "why": e,
                "state": "refused",
            }));
        }
        rows
    }

    /// The firewall rule as text for replies and the report.
    fn fw_rule_text(&self) -> &'static str {
        match self.fw_rule {
            Some(true) => "present",
            Some(false) => "absent",
            None => "not-needed",
        }
    }

    /// IPC `update.status`.
    fn update_status(&self) -> Value {
        match &self.updater {
            Some(u) => u.status_json(),
            None => crate::update::disabled_status_json(&self.profile),
        }
    }

    /// IPC `update.apply`: run the signed apply and rollback helper for the staged version.
    fn update_apply(&mut self) -> Result<Value, IpcError> {
        let Some(u) = &mut self.updater else {
            return Err(IpcError::new(
                "no-updater",
                "This app is updated by the store or the package manager.",
            ));
        };
        match u.apply() {
            Ok(version) => {
                let v = self.update_status();
                self.emit("update", v);
                Ok(json!({"applying": true, "version": version}))
            }
            Err(e) => {
                let v = self.update_status();
                self.emit("update", v);
                Err(IpcError::new("update", e))
            }
        }
    }

    /// IPC `firewall.check` (`uc firewall check`, and after `uc firewall add` or the window's
    /// Fix): look at the rule again and restart the network when the answer changed.
    fn firewall_check(&mut self) -> Value {
        let now = self.check_firewall_rule();
        if now != self.fw_rule {
            self.start_net();
            self.emit("status", json!({}));
        }
        json!({ "rule": self.fw_rule_text(), "incoming": self.fw_rule != Some(false) })
    }

    /// The non-blocking note for a missing firewall rule (status `note`).
    fn firewall_note(&self) -> Option<Value> {
        (self.fw_rule == Some(false)).then(|| {
            json!({
                "code": "firewall",
                "text": "Other devices cannot connect to this PC. It only connects out, so sync works when the other device accepts connections.",
                "fix": FIREWALL_FIX,
            })
        })
    }

    fn setup_status(&self) -> Value {
        json!({
            "rows": self.setup_rows(),
            "done": self.state.setup_done,
            "sandboxed": self.autostart.sandboxed(),
        })
    }

    fn setup_step(&mut self, p: &Value) -> Result<Value, IpcError> {
        let row = p.get("row").and_then(Value::as_str).unwrap_or_default();
        let yes = p.get("answer").and_then(Value::as_bool).unwrap_or(false);
        match row {
            "paste" if cfg!(target_os = "macos") => {
                // Plan 2.2: one read on purpose, so the app appears in "Paste from Other
                // Apps". The content is dropped at once.
                if yes {
                    let _ = self.clip.read();
                }
            }
            "local-network" => {
                // Shown only after the network failed: try again (the user may have just
                // allowed it in System Settings).
                if yes {
                    self.start_net();
                }
            }
            "done" => {
                // Start at login is already on from the first run (`default_autostart`, every
                // OS), so finishing setup is only a flag.
                self.state.setup_done = true;
                self.save();
            }
            _ => {
                return Err(IpcError::new(
                    "bad-row",
                    format!("No setup step \"{row}\"."),
                ));
            }
        }
        Ok(self.setup_status())
    }

    // ------------------------------------------------------------ status, devices, report

    fn connected(&self) -> usize {
        self.peers
            .values()
            .filter(|p| p.gate.data_allowed())
            .count()
    }

    /// The network is on but this device has no LAN address it can use (every interface is
    /// virtual, a VPN, down or without a LAN address), so no socket is bound.
    fn no_lan(&self) -> bool {
        self.net.is_some()
            && self.ifaces_seen
            && !self.policy.allow_loopback
            && !self
                .ifaces
                .iter()
                .any(|i| !uc_net::endpoint::bind_addrs(i, 0, &self.policy).is_empty())
    }

    /// The "Needs attention" reason and its fix, if any (plan 2.7).
    fn attention(&self) -> Option<(&'static str, String, String)> {
        if self.state.notice == Some(Notice::Removed) {
            return Some((
                "removed",
                "Another device removed this one.".into(),
                "Set up again: \"uc pair enter\" with a code from another device.".into(),
            ));
        }
        if !self.profile.is_test() && crate::launch::app_location_problem().is_some() {
            return Some((
                "app-location",
                "The app is running from the disk image or Downloads.".into(),
                "Quit it, drag Universal Clipboard into the Applications folder, and open it from there."
                    .into(),
            ));
        }
        if self.state_damaged && self.graph.is_none() {
            return Some((
                "state-damaged",
                "This device's saved settings were damaged, so it was reset.".into(),
                "Set it up again: \"uc pair enter\" with a code from another device. The old file is state.json.bad."
                    .into(),
            ));
        }
        if let Some(g) = &self.graph
            && !g.is_member(&self.me().0)
            && !g.is_removed(&self.me().0)
        {
            return Some((
                "key-lost",
                "This device lost its key, so your other devices no longer know it.".into(),
                "On another device, remove this one. Then set it up again with \"uc pair enter\"."
                    .into(),
            ));
        }
        if let Some(e) = &self.save_error {
            return Some((
                "save-failed",
                format!("Settings could not be saved ({e})."),
                "Check that the disk has free space, then run \"uc diagnostics\".".into(),
            ));
        }
        if let Some(e) = &self.net_error {
            return Some((
                "network",
                format!("The network could not start ({e})."),
                "Run \"uc diagnostics\" and send the report.".into(),
            ));
        }
        if self.no_lan() {
            return Some((
                "no-lan",
                NO_LAN_TEXT.into(),
                "Connect it to Wi-Fi or a network cable.".into(),
            ));
        }
        if self.clip_error.is_some() {
            return Some((
                "desktop",
                "This desktop blocks background clipboard access.".into(),
                "See the help page for your desktop.".into(),
            ));
        }
        if matches!(self.clip.paste_access(), Some("ask") | Some("deny")) {
            return Some((
                "paste-access",
                "macOS asks before this app can read what you copy.".into(),
                "System Settings > Privacy & Security > Paste from Other Apps: set it to Allow."
                    .into(),
            ));
        }
        if self.updater.as_ref().is_some_and(|u| u.state().gave_up()) {
            return Some((
                "update-failed",
                "The app could not update itself.".into(),
                "Download the newest version: https://github.com/Maxaubert/Universal-Clipboard/releases"
                    .into(),
            ));
        }
        if self.clock_ahead {
            return Some((
                "clock",
                "The date and time on this device are wrong, so its copies are ignored.".into(),
                "Fix the date and time in the system settings.".into(),
            ));
        }
        if self
            .paths
            .values()
            .any(|s| *s == PathState::NoPath(NoPath::VpnBlocksLan))
        {
            return Some((
                "vpn",
                "Can't reach your other device? Your VPN blocks the local network.".into(),
                "Turn on Local network sharing in your VPN.".into(),
            ));
        }
        // The missing rule is only a note (status `note`), unless it may be why no device is
        // connected: in a group, nobody connected, and this PC only connects out.
        if self.fw_rule == Some(false)
            && self.graph.as_ref().is_some_and(|g| g.members().len() > 1)
            && self.connected() == 0
        {
            return Some((
                "firewall",
                "No other device is connected. The firewall rule is missing, so other devices cannot connect to this PC."
                    .into(),
                FIREWALL_FIX.into(),
            ));
        }
        None
    }

    async fn status_json(&mut self) -> Value {
        let members = self.graph.as_ref().map_or(0, |g| g.members().len());
        let connected = self.connected();
        let paused = self.engine.is_paused();
        let (state, text, reason, fix) = if let Some((code, r, f)) = self.attention() {
            (
                "needs-attention",
                "Needs attention",
                Some(r),
                Some((code, f)),
            )
        } else if self.graph.is_none() {
            (
                "not-set-up",
                "Not set up",
                Some("This device has no other devices yet.".to_string()),
                Some((
                    "pair",
                    "Run \"uc pair show\" here, or \"uc pair enter\" with a code from another device."
                        .to_string(),
                )),
            )
        } else if paused {
            ("paused", "Paused", None, None)
        } else if connected == 0 {
            ("no-peers", "No other device is on right now", None, None)
        } else {
            ("working", "Working", None, None)
        };
        let port = match &self.net {
            Some(n) => n.status().await.map_or(self.port, |s| s.port),
            None => self.port,
        };
        json!({
            "state": state,
            "text": text,
            "reason": reason,
            "fix": fix.as_ref().map(|f| &f.1),
            "fix_code": fix.as_ref().map(|f| f.0),
            "note": self.firewall_note(),
            "devices": members,
            "connected": connected,
            "paused": paused,
            "pause": self.state.pause,
            "this_device": self.own_name(),
            "port": port,
            "profile": self.profile.name,
            "pid": std::process::id(),
            "version": self.version,
            "activity": self.state.activity.iter().rev().take(5).collect::<Vec<_>>(),
            "transfer": self.transfer_view(),
            "last_transfer_error": self.last_transfer_error(),
        })
    }

    fn devices_json(&self) -> Value {
        let Some(g) = &self.graph else {
            return json!([{
                "id": hex(&self.key.public()),
                "name": self.own_name(),
                "os": type_name(own_device_type()),
                "this": true,
                "state": "This device",
            }]);
        };
        let me = self.key.public();
        let mut out = Vec::new();
        for k in g.members() {
            let (t, name) = g.device_name(&k).unwrap_or((0, "unknown".into()));
            let d = DeviceId(k);
            let state = if k == me {
                "This device"
            } else if self.peers.get(&d).is_some_and(|p| p.gate.data_allowed()) {
                "Connected"
            } else if self.paths.get(&d) == Some(&PathState::NoPath(NoPath::VpnBlocksLan)) {
                "Blocked by a VPN or firewall"
            } else {
                "Asleep or away"
            };
            out.push(json!({
                "id": hex(&k),
                "name": name,
                "os": type_name(t),
                "this": k == me,
                "state": state,
            }));
        }
        Value::Array(out)
    }

    async fn diagnostics(&mut self) -> Value {
        let st = match &self.net {
            Some(n) => n.status().await,
            None => None,
        };
        let mut lines = vec![
            format!("Universal Clipboard {}", self.version),
            format!("OS: {} {}", std::env::consts::OS, std::env::consts::ARCH),
            format!(
                "Profile: {}",
                if self.profile.is_test() {
                    self.profile.name.as_str()
                } else {
                    "(normal)"
                }
            ),
            format!("Device ID: {}", &hex(&self.key.public())[..16]),
            format!("Clipboard: {}", self.clip.name()),
            format!("Key store: {:?}", self.store.kind()),
            format!(
                "Start at login: {} ({})",
                self.autostart.is_enabled(),
                self.autostart.location()
            ),
            format!("Paused: {:?}", self.state.pause),
            format!("Uptime: {} s", self.started.elapsed().as_secs()),
        ];
        if let Some(e) = &self.clip_error {
            lines.push(format!("Clipboard error: {e}"));
        }
        if let Some(a) = self.clip.paste_access() {
            lines.push(format!("macOS paste access: {a}"));
        }
        lines.push(format!(
            "Firewall rule: {} (incoming: {})",
            self.fw_rule_text(),
            self.fw_rule != Some(false)
        ));
        match &self.updater {
            Some(u) => {
                let st = u.state();
                lines.push(format!(
                    "Updates: on; last: {}; failures in a row: {}; bad versions: {}",
                    u.last
                        .as_deref()
                        .or(st.last_result.as_deref())
                        .unwrap_or("none yet"),
                    st.failures_in_row,
                    if st.bad.is_empty() {
                        "none".to_string()
                    } else {
                        st.bad.join(", ")
                    }
                ));
            }
            None => lines.push("Updates: by the store or the package manager".into()),
        }
        match &self.net_error {
            Some(e) => lines.push(format!("Network: off ({e})")),
            None if self.no_lan() => lines.push(format!("Network: on. {NO_LAN_TEXT}")),
            None => lines.push("Network: on".into()),
        }
        let lan = uc_net::LAN_ONLY;
        lines.push(format!(
            "LAN only: relay={} lookup={} hole-punching={} port-mapping={} wildcard-bind={} migration={}",
            lan.relay, lan.address_lookup, lan.hole_punching, lan.port_mapping, lan.wildcard_bind, lan.migration
        ));
        if let Some(s) = &st {
            lines.push(format!("UDP port: {}", s.port));
            lines.push(format!("VPN up: {}", s.vpn_up));
            lines.push(format!("Refused by the LAN rule: {}", s.refused));
            for (i, a, t) in &s.sockets {
                lines.push(format!("Socket: {i} {a} ({t:?})"));
            }
            for (d, st) in &s.paths {
                lines.push(format!("Path: {} {:?}", self.name_of(d), st));
            }
        }
        if let Some(g) = &self.graph {
            let (epoch, _) = g.current_secret();
            lines.push(format!(
                "Group: {} members, {} changes, epoch {}, secret held: {}",
                g.members().len(),
                g.len(),
                epoch,
                self.secret.as_ref().is_some_and(|s| g.secret_matches(s)),
            ));
        } else {
            lines.push("Group: none".into());
        }
        lines.push(format!("Links open: {}", self.connected()));
        let s = &self.state.settings;
        lines.push(format!(
            "Automatic file copies: {}, up to {} MB, transfer: {}",
            if s.auto_file_copy { "on" } else { "off" },
            s.file_limit_mb,
            self.transfer_view()
                .and_then(|v| v["phase"].as_str().map(str::to_string))
                .unwrap_or_else(|| "none".into())
        ));
        let ps = self.host.status(now_ms());
        lines.push(format!(
            "Pairing: open={} code tries={} lockout={} lockout ms={} lockouts={}",
            ps.open, ps.tries_code, ps.code_off, ps.lockout_left_ms, ps.lockouts
        ));
        for a in self.state.activity.iter().rev().take(10) {
            lines.push(format!(
                "Activity: {} {} {} {}",
                a.time_ms,
                a.kind,
                a.direction,
                a.note.clone().unwrap_or_default()
            ));
        }
        lines.push(String::new());
        lines.push("Log (last lines):".into());
        lines.extend(crate::log::tail(&self.dir, 100));
        let text = lines.join("\n");
        json!({ "report": text })
    }
}

async fn next_net(net: &mut Option<Net>) -> Option<NetEvent> {
    match net {
        Some(n) => n.next_event().await,
        None => std::future::pending().await,
    }
}

async fn sleep_opt(at: Option<tokio::time::Instant>) {
    match at {
        Some(t) => tokio::time::sleep_until(t).await,
        None => std::future::pending().await,
    }
}

fn encode_secret(s: &GroupSecret) -> Vec<u8> {
    let mut v = s.epoch.to_be_bytes().to_vec();
    v.extend_from_slice(&s.secret);
    v
}

fn load_secret(store: &dyn KeyStore, name: &str) -> Option<GroupSecret> {
    let v = store.get(name).ok()??;
    if v.len() != 36 {
        return None;
    }
    let epoch = u32::from_be_bytes(v[..4].try_into().ok()?);
    let mut secret = [0u8; 32];
    secret.copy_from_slice(&v[4..]);
    Some(GroupSecret { epoch, secret })
}

fn load_graph(s: &StateFile) -> Option<Graph> {
    let mut it = s
        .group
        .iter()
        .filter_map(|h| HEXLOWER.decode(h.as_bytes()).ok());
    let first = uc_pair::chain::Change::parse(&it.next()?).ok()?;
    let mut g = Graph::from_genesis(first).ok()?;
    for b in it {
        let _ = g.insert_bytes(&b);
    }
    Some(g)
}
