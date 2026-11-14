//! Self-update in the agent (plan 3.13), for the Windows exe build and the macOS `.dmg` build.
//!
//! - Off for: the Microsoft Store build (the Store updates it), Linux (apt updates it), builds
//!   without the cargo feature `self-update`, and test profiles unless a test sets
//!   `UCLIP_TEST_UPDATE_URL` and `UCLIP_TEST_UPDATE_KEY` (a local `file://` feed signed with a
//!   test key). So the whole flow runs headless in tests, with no network.
//! - **One check per agent start, never again in that agent's life** (decision 2026-11-14,
//!   Max). The agent starts at login, after a reboot and after an update; 15 s after it starts
//!   (1 s with `UCLIP_TEST_FAST=1`) a thread fetches `latest.json`, checks the signature, the
//!   version, the rollout and the bad list, then downloads, checks and unpacks the file into
//!   `<state>/update/<version>/`. There is no periodic check and no "check now".
//! - **Nothing installs by itself.** A staged version is "available": the agent emits the
//!   `update` event and opens the window on its update screen once ([`open_window`]). The
//!   user picks Update now (IPC `update.apply`, `uc update --install`) or Later (IPC
//!   `update.later`: no more asking until the next agent start).
//! - `update.apply` copies the agent to `<state>/update/helper/` and starts that copy with
//!   `--apply-update`. The helper stops the agent, keeps the old version, installs the new one,
//!   starts it and checks its health (2 minutes), and rolls back on failure
//!   ([`uc_update::apply`]). A failed attempt leaves `<state>/update/apply-failed.json`; the
//!   agent that runs next shows it (state `failed`, window opened once) and removes it.
//!
//! Test switches (test profiles only): `UCLIP_TEST_UPDATE_URL`, `UCLIP_TEST_UPDATE_KEY`, and
//! two files next to the agent binary: `test-version.txt` (the version this copy claims to
//! be) and `test-broken` (this copy exits at once, like a broken release). Test profiles never
//! open a window: [`open_window`] appends the route to `<state>/update/window-opens.log`.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use crate::launch::detach;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::json;
use uc_update::apply::{AgentCtl, Baseline, DirFiles, Files, NsisFiles, Outcome, apply};
use uc_update::manifest::Kind;
use uc_update::state::UpdateState;
use uc_update::{Check, Source, Staged};

use crate::ipc::Client;
use crate::profile::Profile;

/// The version this agent reports. Test profiles may claim another version with a
/// `test-version.txt` next to the binary.
pub fn current_version(p: &Profile) -> String {
    if p.is_test()
        && let Some(v) = exe_dir()
            .map(|d| d.join("test-version.txt"))
            .and_then(|f| std::fs::read_to_string(f).ok())
    {
        return v.trim().to_string();
    }
    env!("CARGO_PKG_VERSION").to_string()
}

/// True if this test copy plays a broken release (a `test-broken` file next to it).
pub fn plays_broken(p: &Profile) -> bool {
    p.is_test() && exe_dir().is_some_and(|d| d.join("test-broken").exists())
}

fn exe_dir() -> Option<PathBuf> {
    crate::install::real_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
}

/// The folder that an update replaces: the `.app` bundle on macOS, else the binary's folder.
pub fn install_dir() -> Option<PathBuf> {
    let exe = crate::install::real_exe().ok()?;
    let dir = exe.parent()?.to_path_buf();
    if cfg!(target_os = "macos")
        && dir.file_name().is_some_and(|n| n == "MacOS")
        && let Some(contents) = dir.parent()
        && contents.file_name().is_some_and(|n| n == "Contents")
        && let Some(app) = contents.parent()
        && app.extension().is_some_and(|e| e == "app")
    {
        return Some(app.to_path_buf());
    }
    Some(dir)
}

/// The updater settings, or `None` when this build does not update itself.
#[derive(Clone, Debug)]
pub struct UpdateCfg {
    pub source: Source,
    pub dir: PathBuf,
    /// The delay between agent start and its one check (startup stays quick; the network may
    /// need a few seconds after login).
    pub first: Duration,
    pub health: Duration,
    /// Release notes page for a version (`{v}` is replaced), or `None` (test feeds).
    pub notes: Option<String>,
}

/// The delay before the one check of a real agent.
pub const FIRST_CHECK: Duration = Duration::from_secs(15);

impl UpdateCfg {
    pub fn from_profile(p: &Profile) -> Option<UpdateCfg> {
        let dir = p.state_dir().join("update");
        if p.is_test() {
            let url = p.test_env("UCLIP_TEST_UPDATE_URL")?;
            let key = p.test_env("UCLIP_TEST_UPDATE_KEY")?;
            let fast = p.fast();
            return Some(UpdateCfg {
                source: Source {
                    url,
                    public_key: key,
                    allow_file: true,
                    target: uc_update::manifest::current_target().into(),
                },
                dir,
                first: if fast {
                    Duration::from_secs(1)
                } else {
                    FIRST_CHECK
                },
                health: Duration::from_secs(if fast { 20 } else { 120 }),
                notes: None,
            });
        }
        if !cfg!(feature = "self-update") || !(cfg!(windows) || cfg!(target_os = "macos")) {
            return None;
        }
        #[cfg(windows)]
        if crate::win::is_packaged() {
            return None;
        }
        Some(UpdateCfg {
            source: Source::real(),
            dir,
            first: FIRST_CHECK,
            health: Duration::from_secs(120),
            notes: Some(
                "https://github.com/Maxaubert/Universal-Clipboard/releases/tag/v{v}".into(),
            ),
        })
    }
}

/// The one check of an agent's life: due once, `delay` after start, then never again.
#[derive(Debug)]
pub struct OneCheck {
    at: Option<Instant>,
}

impl OneCheck {
    pub fn new(start: Instant, delay: Duration) -> OneCheck {
        OneCheck {
            at: Some(start + delay),
        }
    }

    /// True exactly once: the first call at or after the due time.
    pub fn due(&mut self, now: Instant) -> bool {
        match self.at {
            Some(t) if now >= t => {
                self.at = None;
                true
            }
            _ => false,
        }
    }

    /// True until the check has started.
    pub fn pending(&self) -> bool {
        self.at.is_some()
    }
}

/// Where the update stands, for `update.status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Nothing to install (not checked yet, up to date, or the check failed).
    None,
    /// A newer version is downloaded and checked, and waits for the user.
    Available,
    /// The helper runs: the agent stops soon.
    Applying,
    /// The last attempt failed (the old version is back), or the helper did not start.
    Failed,
}

impl Phase {
    pub fn word(self) -> &'static str {
        match self {
            Phase::None => "none",
            Phase::Available => "available",
            Phase::Applying => "applying",
            Phase::Failed => "failed",
        }
    }
}

/// What a failed attempt leaves for the next agent (`<state>/update/apply-failed.json`).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ApplyFailed {
    pub version: String,
    pub error: String,
}

fn apply_failed_file(dir: &Path) -> PathBuf {
    dir.join("apply-failed.json")
}

/// Read and remove the note of a failed attempt.
fn take_apply_failed(dir: &Path) -> Option<ApplyFailed> {
    let f = apply_failed_file(dir);
    let b = std::fs::read(&f).ok()?;
    let _ = std::fs::remove_file(&f);
    serde_json::from_slice(&b).ok()
}

/// The updater inside the agent's core. Polled from the core's 1 s tick.
pub struct Updater {
    cfg: UpdateCfg,
    profile: Profile,
    check: OneCheck,
    /// Checks started in this agent's life (0 or 1).
    checks: u32,
    rx: Option<mpsc::Receiver<Result<Option<Staged>, String>>>,
    staged: Option<Staged>,
    phase: Phase,
    /// The version the last failure was about.
    failed_version: Option<String>,
    /// "Later" was picked: do not ask again until the next agent start.
    dismissed: bool,
    /// The window was opened for this agent's update news (at most once per life).
    window_opened: bool,
    last_error: Option<String>,
    /// The last result, for the diagnostic report.
    pub last: Option<String>,
}

impl Updater {
    pub fn new(cfg: UpdateCfg, profile: Profile) -> Updater {
        Updater {
            check: OneCheck::new(Instant::now(), cfg.first),
            cfg,
            profile,
            checks: 0,
            rx: None,
            staged: None,
            phase: Phase::None,
            failed_version: None,
            dismissed: false,
            window_opened: false,
            last_error: None,
            last: None,
        }
    }

    /// The state file (bad versions, failures in a row).
    pub fn state(&self) -> UpdateState {
        UpdateState::load(&self.cfg.dir)
    }

    /// Call once a second. Returns true when the status changed (the core emits `update`).
    pub fn tick(&mut self, device_id: [u8; 32]) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(r) => {
                    self.rx = None;
                    changed = true;
                    match r {
                        Ok(Some(s)) => {
                            tracing::info!(version = %s.version, "update ready, asking the user");
                            self.last = Some(format!("{} downloaded and checked", s.version));
                            self.last_error = None;
                            self.staged = Some(s);
                            self.phase = Phase::Available;
                            self.failed_version = None;
                            self.ask();
                        }
                        Ok(None) => {}
                        Err(e) => {
                            tracing::warn!(error = %e, "update check");
                            self.last = Some(e.clone());
                            self.last_error = Some(e);
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.rx = None,
            }
        }
        if self.check.due(Instant::now()) {
            changed |= self.notice_failed_attempt();
            // Run from the disk image or a translocated copy: the bundle cannot be replaced.
            if !self.profile.is_test() && crate::launch::app_location_problem().is_some() {
                self.last = Some("not checked: the app is not in the Applications folder".into());
                return changed;
            }
            self.checks += 1;
            tracing::info!("update check (the only one in this agent's life)");
            let (tx, rx) = mpsc::channel();
            self.rx = Some(rx);
            let cfg = self.cfg.clone();
            let current = current_version(&self.profile);
            let _ = std::thread::Builder::new()
                .name("uc-update".into())
                .spawn(move || {
                    let _ = tx.send(check_and_stage(&cfg, &current, &device_id));
                });
        }
        changed
    }

    /// The helper left a note that the last attempt failed: show it (once).
    fn notice_failed_attempt(&mut self) -> bool {
        let Some(f) = take_apply_failed(&self.cfg.dir) else {
            return false;
        };
        tracing::warn!(version = %f.version, error = %f.error, "the last update failed");
        self.phase = Phase::Failed;
        self.failed_version = Some(f.version);
        self.last_error = Some(f.error);
        self.ask();
        true
    }

    /// Open the window on its update screen, at most once per agent life, never after Later.
    fn ask(&mut self) {
        if self.window_opened || self.dismissed {
            return;
        }
        self.window_opened = true;
        if let Err(e) = open_window(&self.profile, "update") {
            tracing::warn!(error = %e, "could not open the window for the update");
        }
    }

    /// IPC `update.status`.
    pub fn status_json(&self) -> serde_json::Value {
        let available = self.staged.as_ref().map(|s| s.version.clone());
        let notes = available
            .as_deref()
            .and_then(|v| self.cfg.notes.as_ref().map(|n| n.replace("{v}", v)));
        let st = self.state();
        json!({
            "enabled": true,
            "current": current_version(&self.profile),
            "available": available,
            "failed_version": self.failed_version,
            "notes_url": notes,
            "state": self.phase.word(),
            "dismissed": self.dismissed,
            "checked": !self.check.pending() && self.rx.is_none(),
            "checks": self.checks,
            "last_check_ms": st.last_check_ms,
            "last_error": self.last_error,
        })
    }

    /// IPC `update.later`: no more asking in this agent's life.
    pub fn later(&mut self) {
        self.dismissed = true;
    }

    /// IPC `update.apply`: start the helper for the staged version.
    pub fn apply(&mut self) -> Result<String, String> {
        if self.phase == Phase::Applying {
            return Err("The update is being installed.".into());
        }
        let Some(s) = self.staged.clone() else {
            return Err("There is no update to install.".into());
        };
        match launch_helper(&self.profile, &s) {
            Ok(()) => {
                tracing::info!(version = %s.version, "update helper started");
                self.phase = Phase::Applying;
                self.last = Some(format!("installing {}", s.version));
                Ok(s.version)
            }
            Err(e) => {
                tracing::warn!(error = %e, "update helper");
                let msg = format!("Could not start the update: {e}");
                self.phase = Phase::Failed;
                self.failed_version = Some(s.version.clone());
                self.last = Some(msg.clone());
                self.last_error = Some(msg.clone());
                let mut st = self.state();
                st.failed(&s.version, &msg, false);
                let _ = st.save(&self.cfg.dir);
                self.staged = None;
                Err(msg)
            }
        }
    }
}

/// The "no updater" reply of `update.status` (the Store, apt, builds without the feature).
pub fn disabled_status_json(p: &Profile) -> serde_json::Value {
    json!({
        "enabled": false,
        "current": current_version(p),
        "available": null,
        "failed_version": null,
        "notes_url": null,
        "state": "none",
        "dismissed": false,
        "checked": false,
        "checks": 0,
        "last_check_ms": 0,
        "last_error": null,
    })
}

/// Open the window with `--route <route>`, detached, as its own app. The agent shows no
/// dialog of its own. A window that is already open gets the arguments through its
/// single-instance handler (Windows, Linux) or is brought to the front (macOS) and follows the
/// `update` event.
///
/// - Windows: `Universal Clipboard.exe` next to `uc-agent.exe`.
/// - macOS: `/usr/bin/open -a <the .app> --args --route <route>`. LaunchServices starts the
///   app as its own responsible process, so no permission moves between the agent and it.
/// - Linux: `universal-clipboard` next to the agent (`/usr/lib/universal-clipboard/`).
///
/// Test profiles never start anything: the route is appended to
/// `<state>/update/window-opens.log`.
pub fn open_window(p: &Profile, route: &str) -> std::io::Result<()> {
    if p.is_test() {
        use std::io::Write;
        let dir = p.state_dir().join("update");
        std::fs::create_dir_all(&dir)?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("window-opens.log"))?;
        return writeln!(f, "{route}");
    }
    let mut c = window_command(route)?;
    detach(&mut c);
    c.spawn().map(|_| ())
}

#[cfg(target_os = "macos")]
fn window_command(route: &str) -> std::io::Result<Command> {
    let app = install_dir()
        .filter(|d| d.extension().is_some_and(|e| e == "app"))
        .ok_or_else(|| std::io::Error::other("the agent is not in an app bundle"))?;
    if !app
        .join("Contents")
        .join("MacOS")
        .join("universal-clipboard")
        .exists()
    {
        return Err(std::io::Error::other("this build has no window"));
    }
    let mut c = Command::new("/usr/bin/open");
    c.arg("-a").arg(&app).args(["--args", "--route", route]);
    Ok(c)
}

#[cfg(not(target_os = "macos"))]
fn window_command(route: &str) -> std::io::Result<Command> {
    let name = if cfg!(windows) {
        "Universal Clipboard.exe"
    } else {
        "universal-clipboard"
    };
    let exe = exe_dir()
        .map(|d| d.join(name))
        .filter(|e| e.exists())
        .ok_or_else(|| std::io::Error::other("this build has no window"))?;
    let mut c = Command::new(exe);
    c.args(["--route", route]);
    Ok(c)
}

fn check_and_stage(
    cfg: &UpdateCfg,
    current: &str,
    device_id: &[u8; 32],
) -> Result<Option<Staged>, String> {
    let mut st = UpdateState::load(&cfg.dir);
    st.last_check_ms = crate::agent::now_ms();
    let r = match uc_update::check(&cfg.source, current, device_id, &st) {
        Ok(Check::UpToDate) => {
            st.last_result = Some(format!("{current} is the newest"));
            Ok(None)
        }
        Ok(Check::Skip(why)) => {
            st.last_result = Some(why);
            Ok(None)
        }
        Ok(Check::Take(m, f)) => match uc_update::stage(&cfg.source, &m, &f, &cfg.dir) {
            Ok(s) => Ok(Some(s)),
            Err(e) => {
                st.failed(&m.version, &e.to_string(), false);
                Err(e.to_string())
            }
        },
        Err(e) => {
            st.last_result = Some(e.to_string());
            Err(e.to_string())
        }
    };
    let _ = st.save(&cfg.dir);
    r
}

fn kind_word(k: Kind) -> &'static str {
    match k {
        Kind::Nsis => "nsis",
        Kind::Tar => "tar",
    }
}

/// Copy this binary to `<state>/update/helper/` and start the copy with `--apply-update`,
/// detached, in its own process group (so stopping the agent, or launchd ending its job,
/// does not end the helper).
fn launch_helper(p: &Profile, s: &Staged) -> std::io::Result<()> {
    let me = crate::install::real_exe()?;
    let install = install_dir().ok_or_else(|| std::io::Error::other("no install folder"))?;
    writable(&install)?;
    let rel = me
        .strip_prefix(&install)
        .map_err(|_| std::io::Error::other("binary outside the install folder"))?
        .to_path_buf();
    let hdir = p.state_dir().join("update").join("helper");
    let _ = std::fs::remove_dir_all(&hdir);
    std::fs::create_dir_all(&hdir)?;
    let helper = hdir.join(me.file_name().unwrap_or_default());
    std::fs::copy(&me, &helper)?;
    // A test copy that claims a version keeps claiming it (the helper does not report).
    let mut c = Command::new(&helper);
    c.arg("--apply-update")
        .arg("--to-version")
        .arg(&s.version)
        .arg("--kind")
        .arg(kind_word(s.kind))
        .arg("--staged")
        .arg(&s.path)
        .arg("--install-dir")
        .arg(&install)
        .arg("--agent-rel")
        .arg(&rel);
    if p.is_test() {
        c.args(["--profile", &p.name]);
    }
    detach(&mut c);
    c.spawn().map(|_| ())
}

/// The helper's arguments.
#[derive(Debug, Clone)]
pub struct HelperArgs {
    pub version: String,
    pub kind: Kind,
    pub staged: PathBuf,
    pub install_dir: PathBuf,
    pub agent_rel: PathBuf,
}

impl HelperArgs {
    /// Parse `--to-version V --kind K --staged P --install-dir D --agent-rel R`.
    pub fn parse(args: &[String]) -> Result<HelperArgs, String> {
        let get = |f: &str| -> Result<String, String> {
            let i = args
                .iter()
                .position(|a| a == f)
                .ok_or(format!("{f} missing"))?;
            args.get(i + 1).cloned().ok_or(format!("{f} needs a value"))
        };
        let kind = match get("--kind")?.as_str() {
            "nsis" => Kind::Nsis,
            "tar" => Kind::Tar,
            k => return Err(format!("unknown kind {k}")),
        };
        Ok(HelperArgs {
            version: get("--to-version")?,
            kind,
            staged: get("--staged")?.into(),
            install_dir: get("--install-dir")?.into(),
            agent_rel: get("--agent-rel")?.into(),
        })
    }
}

/// Fail early when the install folder cannot be replaced (root-owned, read-only volume), so
/// the helper never stops the agent for an update it cannot install.
fn writable(dir: &Path) -> std::io::Result<()> {
    let probe = dir.join(format!(".uc-update-probe-{}", std::process::id()));
    std::fs::write(&probe, b"")
        .map_err(|e| std::io::Error::other(format!("the app folder cannot be written: {e}")))?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

/// The agent steps through IPC and processes.
struct IpcAgent {
    profile: Profile,
    exe: PathBuf,
    pid: Option<u32>,
    child: Option<Child>,
    rt: tokio::runtime::Runtime,
}

impl IpcAgent {
    fn status(&self) -> Option<serde_json::Value> {
        self.rt.block_on(async {
            let mut c =
                tokio::time::timeout(Duration::from_secs(2), Client::connect(&self.profile))
                    .await
                    .ok()?
                    .ok()?;
            tokio::time::timeout(Duration::from_secs(3), c.call("status", json!({})))
                .await
                .ok()?
                .ok()?
                .ok()
        })
    }
}

fn kill(pid: u32) {
    #[cfg(windows)]
    let mut c = {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let mut c = Command::new(Path::new(&root).join("System32").join("taskkill.exe"));
        c.args(["/F", "/PID", &pid.to_string()]);
        c
    };
    #[cfg(unix)]
    let mut c = {
        let mut c = Command::new("kill");
        c.args(["-9", &pid.to_string()]);
        c
    };
    detach(&mut c);
    let _ = c.status();
}

impl AgentCtl for IpcAgent {
    fn baseline(&mut self) -> Baseline {
        let s = self.status();
        self.pid = s.as_ref().and_then(|v| v["pid"].as_u64()).map(|p| p as u32);
        Baseline {
            connected: s.and_then(|v| v["connected"].as_u64()).unwrap_or(0) as usize,
        }
    }

    fn stop(&mut self) -> std::io::Result<()> {
        // From here until start(): `uc`, the window and launchd do not start the old agent.
        crate::launch::set_update_marker(&self.profile, true);
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        if crate::install::stop_agent(&self.profile, Duration::from_secs(10)) {
            return Ok(());
        }
        if let Some(pid) = self
            .status()
            .and_then(|v| v["pid"].as_u64())
            .or(self.pid.map(u64::from))
        {
            kill(pid as u32);
        }
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(5) {
            if !crate::install::agent_running(&self.profile) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err(std::io::Error::other("the agent did not stop"))
    }

    fn start(&mut self) -> std::io::Result<()> {
        // The files are in place: whoever starts the agent now starts this version.
        crate::launch::set_update_marker(&self.profile, false);
        // macOS bundle: through launchd (the login item if it is on), so the new agent is not
        // a child of the helper and keeps its own permissions.
        self.child = crate::launch::start(&self.profile, &self.exe)?;
        Ok(())
    }

    fn healthy(&mut self, version: &str, base: &Baseline, within: Duration) -> Result<(), String> {
        let t = Instant::now();
        let mut last = "the agent did not answer".to_string();
        while t.elapsed() < within {
            if let Some(c) = &mut self.child
                && let Ok(Some(st)) = c.try_wait()
            {
                return Err(format!("the agent exited ({st})"));
            }
            if let Some(s) = self.status() {
                let v = s["version"].as_str().unwrap_or("");
                let port = s["port"].as_u64().unwrap_or(0);
                let connected = s["connected"].as_u64().unwrap_or(0) as usize;
                let net_broken = s["fix_code"].as_str() == Some("network");
                if v != version {
                    last = format!("version {v} answers, expected {version}");
                } else if net_broken || port == 0 {
                    last = "the network did not start".into();
                } else if base.connected > 0 && connected == 0 {
                    last = "no device that was connected before came back".into();
                } else {
                    return Ok(());
                }
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        Err(last)
    }
}

/// `uc-agent --apply-update ...`: the update helper. Returns the process exit code.
pub fn run_helper(p: &Profile, args: &[String]) -> i32 {
    let a = match HelperArgs::parse(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let Some(cfg) = UpdateCfg::from_profile(p).or_else(|| {
        // The real app with the feature on always has a config; this is a fallback for the
        // state folder only.
        Some(UpdateCfg {
            source: Source::real(),
            dir: p.state_dir().join("update"),
            first: Duration::ZERO,
            health: Duration::from_secs(120),
            notes: None,
        })
    }) else {
        return 2;
    };
    let previous = cfg.dir.join("previous");
    let mut files: Box<dyn Files> = match a.kind {
        Kind::Nsis => Box::new(NsisFiles {
            install_dir: a.install_dir.clone(),
            previous_dir: previous,
            setup_exe: a.staged.clone(),
            timeout: Duration::from_secs(300),
        }),
        Kind::Tar => Box::new(DirFiles {
            install_dir: a.install_dir.clone(),
            previous_dir: previous,
            staged_dir: a.staged.clone(),
        }),
    };
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return 1;
    };
    let mut agent = IpcAgent {
        profile: p.clone(),
        exe: a.install_dir.join(&a.agent_rel),
        pid: None,
        child: None,
        rt,
    };
    let mut st = UpdateState::load(&cfg.dir);
    let o = apply(&mut *files, &mut agent, &mut st, &a.version, cfg.health);
    crate::launch::set_update_marker(p, false);
    let _ = st.save(&cfg.dir);
    // A failed attempt: the agent that runs now (the old version) shows it at its one check.
    let failure = match &o {
        Outcome::Updated => None,
        Outcome::NotStarted(e) | Outcome::RolledBack(e) | Outcome::RollbackFailed(e) => {
            Some(e.clone())
        }
    };
    if let Some(error) = failure {
        let note = ApplyFailed {
            version: a.version.clone(),
            error,
        };
        if let Ok(b) = serde_json::to_vec(&note) {
            let _ = std::fs::write(apply_failed_file(&cfg.dir), b);
        }
    }
    let line = format!(
        "{} update to {}: {o:?}\n",
        crate::agent::now_ms(),
        a.version
    );
    let log = cfg.dir.join("helper.log");
    let old = std::fs::read_to_string(&log).unwrap_or_default();
    let keep: String = old
        .lines()
        .rev()
        .take(50)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|l| format!("{l}\n"))
        .collect();
    let _ = std::fs::write(&log, keep + &line);
    // The staged files are not needed any more.
    if let Some(vdir) = a
        .staged
        .ancestors()
        .find(|d| d.parent() == Some(cfg.dir.as_path()))
    {
        let _ = std::fs::remove_dir_all(vdir);
    }
    match o {
        Outcome::Updated => 0,
        Outcome::NotStarted(_) | Outcome::RolledBack(_) => 1,
        Outcome::RollbackFailed(_) => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_args_parse() {
        let a: Vec<String> = [
            "--to-version",
            "1.2.3",
            "--kind",
            "tar",
            "--staged",
            "/s",
            "--install-dir",
            "/i",
            "--agent-rel",
            "uc-agent",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let h = HelperArgs::parse(&a).unwrap();
        assert_eq!(h.version, "1.2.3");
        assert_eq!(h.kind, Kind::Tar);
        assert!(HelperArgs::parse(&a[..4]).is_err());
    }

    /// Decision 2026-11-14: one check per agent start, never periodic. A simulated 30-day
    /// run of 1 s ticks starts exactly one check, `delay` after start.
    #[test]
    fn one_check_per_agent_life() {
        let start = Instant::now();
        let mut c = OneCheck::new(start, FIRST_CHECK);
        let mut due = Vec::new();
        for s in 0..(30 * 24 * 3600u64) {
            if c.due(start + Duration::from_secs(s)) {
                due.push(s);
            }
        }
        assert_eq!(due, vec![FIRST_CHECK.as_secs()]);
        assert!(!c.pending());
    }

    #[test]
    fn real_check_is_soon_after_start() {
        assert!(FIRST_CHECK >= Duration::from_secs(5) && FIRST_CHECK <= Duration::from_secs(60));
        let p = Profile::new("").unwrap();
        if let Some(c) = UpdateCfg::from_profile(&p) {
            assert_eq!(c.first, FIRST_CHECK);
        }
    }

    #[test]
    fn a_failed_attempt_is_shown_once() {
        let d = tempfile::tempdir().unwrap();
        let note = ApplyFailed {
            version: "9.9.9".into(),
            error: "health check failed".into(),
        };
        std::fs::write(
            apply_failed_file(d.path()),
            serde_json::to_vec(&note).unwrap(),
        )
        .unwrap();
        assert_eq!(take_apply_failed(d.path()).unwrap().version, "9.9.9");
        assert!(take_apply_failed(d.path()).is_none());
    }

    #[test]
    fn test_profiles_never_open_a_window() {
        let d = tempfile::tempdir().unwrap();
        let p = Profile::test("upd-win", d.path().to_path_buf()).unwrap();
        open_window(&p, "update").unwrap();
        open_window(&p, "update").unwrap();
        let log =
            std::fs::read_to_string(p.state_dir().join("update").join("window-opens.log")).unwrap();
        assert_eq!(log, "update\nupdate\n");
    }

    #[test]
    fn the_real_app_never_uses_test_feeds() {
        let p = Profile::new("").unwrap();
        if let Some(c) = UpdateCfg::from_profile(&p) {
            assert!(!c.source.allow_file);
            assert!(c.source.url.starts_with("https://github.com/"));
            assert_eq!(c.source.public_key, uc_update::UPDATER_PUBLIC_KEY);
        }
        assert_eq!(current_version(&p), env!("CARGO_PKG_VERSION"));
    }
}
