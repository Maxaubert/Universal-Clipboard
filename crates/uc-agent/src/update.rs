//! Self-update in the agent (plan 3.13), for the Windows exe build and the macOS `.dmg` build.
//!
//! - Off for: the Microsoft Store build (the Store updates it), Linux (apt updates it), builds
//!   without the cargo feature `self-update`, and test profiles unless a test sets
//!   `UCLIP_TEST_UPDATE_URL` and `UCLIP_TEST_UPDATE_KEY` (a local `file://` feed signed with a
//!   test key). So the whole flow runs headless in tests, with no network.
//! - Once a day (10 minutes after start for the first time) a thread fetches `latest.json`,
//!   checks the signature, the version, the rollout and the bad list, then downloads, checks
//!   and unpacks the file into `<state>/update/<version>/`.
//! - When the clipboard has been idle for 60 s, the agent copies itself to
//!   `<state>/update/helper/` and starts that copy with `--apply-update`. The helper stops the
//!   agent, keeps the old version, installs the new one, starts it and checks its health
//!   (2 minutes), and rolls back on failure ([`uc_update::apply`]).
//!
//! Test switches (test profiles only): `UCLIP_TEST_UPDATE_URL`, `UCLIP_TEST_UPDATE_KEY`, and
//! two files next to the agent binary: `test-version.txt` (the version this copy claims to
//! be) and `test-broken` (this copy exits at once, like a broken release).

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
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
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
}

/// The folder that an update replaces: the `.app` bundle on macOS, else the binary's folder.
pub fn install_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
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
    pub first: Duration,
    pub every: Duration,
    pub idle: Duration,
    pub health: Duration,
}

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
                first: Duration::from_secs(if fast { 1 } else { 600 }),
                every: Duration::from_secs(if fast { 5 } else { 86_400 }),
                idle: Duration::from_secs(if fast { 1 } else { 60 }),
                health: Duration::from_secs(if fast { 20 } else { 120 }),
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
            first: Duration::from_secs(600),
            every: Duration::from_secs(86_400),
            idle: Duration::from_secs(60),
            health: Duration::from_secs(120),
        })
    }
}

/// The updater inside the agent's core. Polled from the core's 1 s tick.
pub struct Updater {
    cfg: UpdateCfg,
    profile: Profile,
    next: Instant,
    rx: Option<mpsc::Receiver<Result<Option<Staged>, String>>>,
    staged: Option<Staged>,
    launched: bool,
    /// The last result, for the diagnostic report.
    pub last: Option<String>,
}

impl Updater {
    pub fn new(cfg: UpdateCfg, profile: Profile) -> Updater {
        Updater {
            next: Instant::now() + cfg.first,
            cfg,
            profile,
            rx: None,
            staged: None,
            launched: false,
            last: None,
        }
    }

    /// The state file (bad versions, failures in a row).
    pub fn state(&self) -> UpdateState {
        UpdateState::load(&self.cfg.dir)
    }

    /// Call once a second. `idle_for`: time since the last clipboard activity.
    pub fn tick(&mut self, device_id: [u8; 32], idle_for: Duration) {
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(r) => {
                    self.rx = None;
                    match r {
                        Ok(Some(s)) => {
                            tracing::info!(version = %s.version, "update ready");
                            self.last = Some(format!("{} downloaded and checked", s.version));
                            self.staged = Some(s);
                        }
                        Ok(None) => {}
                        Err(e) => {
                            tracing::warn!(error = %e, "update check");
                            self.last = Some(e);
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.rx = None,
            }
        }
        if let Some(s) = &self.staged
            && !self.launched
            && idle_for >= self.cfg.idle
        {
            match launch_helper(&self.profile, s) {
                Ok(()) => {
                    tracing::info!(version = %s.version, "update helper started");
                    self.launched = true;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "update helper");
                    self.last = Some(format!("could not start the update helper: {e}"));
                    self.staged = None;
                }
            }
        }
        if self.rx.is_none() && self.staged.is_none() && Instant::now() >= self.next {
            self.next = Instant::now() + self.cfg.every;
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
    }
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
    let me = std::env::current_exe()?;
    let install = install_dir().ok_or_else(|| std::io::Error::other("no install folder"))?;
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

fn detach(c: &mut Command) {
    c.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW
        c.creation_flags(0x0000_0008 | 0x0000_0200 | 0x0800_0000);
        crate::win::no_inherit_std_handles();
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
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
        let mut c = Command::new(&self.exe);
        if self.profile.is_test() {
            c.args(["--profile", &self.profile.name]);
        }
        detach(&mut c);
        self.child = Some(c.spawn()?);
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
            every: Duration::ZERO,
            idle: Duration::ZERO,
            health: Duration::from_secs(120),
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
    let _ = st.save(&cfg.dir);
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
