//! Multi-agent harness: real `uc-agent` processes with test profiles on loopback, driven
//! through the real `uc` binary. Each agent has its own temp state folder, a file key store
//! and (unless a test asks for an OS clipboard that is isolated) a fake clipboard. Nothing
//! here touches the user's clipboard, screen or login items.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use uc_agent::ipc::Client;
use uc_agent::profile::Profile;

static NEXT: AtomicU32 = AtomicU32::new(0);

/// The agent binary next to the `uc` binary. `cargo test -p uc-cli` does not build other
/// packages' binaries, so this runs `cargo build -p uc-agent` once per test process (a no-op
/// when it is up to date), so a test never runs a stale agent.
pub fn agent_exe() -> PathBuf {
    static BUILT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    let uc = PathBuf::from(env!("CARGO_BIN_EXE_uc"));
    let exe = uc.with_file_name(if cfg!(windows) {
        "uc-agent.exe"
    } else {
        "uc-agent"
    });
    BUILT.get_or_init(|| {
        if let Ok(cargo) = std::env::var("CARGO") {
            let st = Command::new(cargo)
                .args(["build", "-q", "-p", "uc-agent", "--bin", "uc-agent"])
                .status()
                .expect("cargo build uc-agent");
            assert!(st.success(), "building uc-agent failed");
        }
    });
    assert!(exe.exists(), "no uc-agent at {}", exe.display());
    exe
}

/// Loopback address for agent `n`. macOS has only 127.0.0.1 on lo0.
pub fn ip(n: u8) -> String {
    if cfg!(target_os = "macos") {
        "127.0.0.1".into()
    } else {
        format!("127.0.0.{n}")
    }
}

/// One agent process.
pub struct Agent {
    pub n: u8,
    pub name: String,
    pub profile: Profile,
    pub dir: PathBuf,
    pub ip: String,
    pub child: Option<Child>,
    pub extra_env: Vec<(String, String)>,
    pub remove_env: Vec<String>,
    _tmp: Option<tempfile::TempDir>,
}

impl Agent {
    /// Make an agent (not started).
    pub fn new(n: u8) -> Agent {
        let tmp = tempfile::Builder::new().prefix("uc").tempdir().unwrap();
        let k = NEXT.fetch_add(1, Ordering::Relaxed);
        let name = format!("t{}-{}-{}", std::process::id() % 100_000, k, n);
        let dir = tmp.path().to_path_buf();
        Agent {
            n,
            profile: Profile::test(&name, dir.clone()).unwrap(),
            name,
            dir,
            ip: ip(n),
            child: None,
            extra_env: Vec::new(),
            remove_env: Vec::new(),
            _tmp: Some(tmp),
        }
    }

    /// The environment of this agent's processes.
    pub fn env(&self) -> Vec<(String, String)> {
        let mut e = vec![
            ("UCLIP_PROFILE".to_string(), self.name.clone()),
            ("UCLIP_STATE_DIR".into(), self.dir.display().to_string()),
            ("UCLIP_TEST_KEYSTORE".into(), "file".into()),
            ("UCLIP_TEST_LOOPBACK".into(), self.ip.clone()),
            ("UCLIP_TEST_FAST".into(), "1".into()),
            ("UCLIP_NO_START".into(), "1".into()),
            ("UCLIP_LOG".into(), "debug".into()),
        ];
        e.extend(self.extra_env.iter().cloned());
        e
    }

    /// Start the process and wait until it answers.
    pub fn start(&mut self) {
        let mut c = Command::new(agent_exe());
        c.envs(self.env())
            .env_remove("UCLIP_TEST_CLIPBOARD")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for (k, v) in &self.extra_env {
            c.env(k, v);
        }
        for k in &self.remove_env {
            c.env_remove(k);
        }
        self.child = Some(c.spawn().expect("spawn uc-agent"));
        self.wait_ready();
    }

    /// Wait until the agent answers `status`.
    pub fn wait_ready(&self) {
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(20) {
            if self.try_call("status", json!({})).is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("agent {} did not start; log:\n{}", self.n, self.log());
    }

    /// The agent's log (no content in it).
    pub fn log(&self) -> String {
        std::fs::read_to_string(self.dir.join("agent.log")).unwrap_or_default()
    }

    /// Run `uc` with this agent's environment.
    pub fn uc(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_uc"))
            .args(args)
            .envs(self.env())
            .stdin(Stdio::null())
            .output()
            .expect("run uc")
    }

    /// Run `uc ... --json` and parse the result. Panics on failure.
    pub fn uc_json(&self, args: &[&str]) -> Value {
        let mut a = args.to_vec();
        a.push("--json");
        let o = self.uc(&a);
        assert!(
            o.status.success(),
            "uc {args:?} failed: {}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        serde_json::from_slice(&o.stdout).unwrap_or_else(|e| {
            panic!(
                "uc {args:?}: not JSON ({e}): {}",
                String::from_utf8_lossy(&o.stdout)
            )
        })
    }

    /// A direct IPC call (test-only debug calls).
    pub fn try_call(&self, call: &str, params: Value) -> Result<Value, String> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let mut c = Client::connect(&self.profile)
                .await
                .map_err(|e| e.to_string())?;
            c.call(call, params)
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.message)
        })
    }

    pub fn call(&self, call: &str, params: Value) -> Value {
        self.try_call(call, params)
            .unwrap_or_else(|e| panic!("{call} on agent {}: {e}", self.n))
    }

    /// "The user copies" text on this agent's (test) clipboard.
    pub fn copy(&self, text: &str) {
        self.call("debug.copy", json!({ "text": text }));
    }

    /// The text on this agent's (test) clipboard.
    pub fn clipboard(&self) -> Option<String> {
        self.call("debug.clipboard", json!({}))["text"]
            .as_str()
            .map(str::to_string)
    }

    pub fn port(&self) -> u16 {
        self.call("status", json!({}))["port"].as_u64().unwrap() as u16
    }

    pub fn addr(&self) -> String {
        format!("{}:{}", self.ip, self.port())
    }

    /// Stop the agent and wait for the process to end.
    pub fn stop(&mut self) {
        let _ = self.try_call("debug.shutdown", json!({}));
        if let Some(mut c) = self.child.take() {
            let t = Instant::now();
            while t.elapsed() < Duration::from_secs(10) {
                if let Ok(Some(_)) = c.try_wait() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    pub fn state_dir(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        self.stop();
        uc_agent::autostart::Autostart::new(&self.profile, PathBuf::new(), &self.dir)
            .remove_sandbox();
    }
}

/// Wait until `f` holds.
pub fn until(limit: Duration, mut f: impl FnMut() -> bool) -> bool {
    let t = Instant::now();
    while t.elapsed() < limit {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Pair `joiner` with `inviter` through the CLI: `uc pair show` on the inviter (as a running
/// process, like a user waiting at the screen) and `uc pair enter <code>` on the joiner.
pub fn pair(inviter: &Agent, joiner: &mut Agent) {
    use std::io::{BufRead, BufReader};
    let mut show = Command::new(env!("CARGO_BIN_EXE_uc"))
        .args(["pair", "show", "--json"])
        .envs(inviter.env())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(show.stdout.take().unwrap());
    let mut line = String::new();
    out.read_line(&mut line).unwrap();
    let v: Value =
        serde_json::from_str(line.trim()).unwrap_or_else(|e| panic!("pair show: {e}: {line}"));
    let code = v["code"].as_str().expect("a code").replace(' ', "");
    assert!(v["qr"].as_str().unwrap().starts_with("UCLIP1:"));
    let port = v["port"].as_u64().unwrap();
    let mut j = Command::new(env!("CARGO_BIN_EXE_uc"));
    j.args(["pair", "enter", &code, "--json", "--yes"])
        .envs(joiner.env())
        .env("UCLIP_TEST_PAIR_ADDRS", format!("{}:{port}", inviter.ip))
        .stdin(Stdio::null());
    let o = j.output().unwrap();
    assert!(
        o.status.success(),
        "pair enter failed: {}{}\ninviter log:\n{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr),
        inviter.log()
    );
    let mut added = String::new();
    out.read_line(&mut added).unwrap();
    assert!(added.contains("pair.added"), "inviter saw: {added}");
    let _ = show.wait();
}

/// The copy matrix: a copy on each agent reaches every other agent.
pub fn copy_matrix(agents: &[&Agent], tag: &str) {
    for a in agents {
        let text = format!("{tag} from agent {} {}", a.n, uc_agent::agent::now_ms());
        a.copy(&text);
        for b in agents.iter().filter(|b| b.n != a.n) {
            let ok = until(Duration::from_secs(10), || {
                b.clipboard().as_deref() == Some(&text)
            });
            assert!(
                ok,
                "copy on {} did not reach {} (has {:?})\nlog {}:\n{}\nlog {}:\n{}",
                a.n,
                b.n,
                b.clipboard(),
                a.n,
                a.log(),
                b.n,
                b.log()
            );
        }
    }
}
