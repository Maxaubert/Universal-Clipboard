//! Self-update end to end (plan 3.13, M9 acceptance "N, then a broken N+1, then automatic
//! rollback to N"), headless and offline:
//!
//! - version N: a copy of the real `uc-agent` in a temp "install folder", a test profile and
//!   a fake clipboard;
//! - a local `file://` feed (`latest.json`) signed with a test key, rollout 100%;
//! - N+1: the same binary with `test-version.txt` (and, for the broken case, `test-broken`,
//!   which makes it exit at once).
//!
//! The agent checks once at start and finds the update. It installs nothing by itself: it
//! asks for the window (logged, never opened, in test profiles) and waits for Update now
//! (`update.apply`, `uc update --install`). The helper then stops it, installs N+1, starts it
//! and checks its health. Broken: it puts N back, starts it and marks N+1 bad, and N shows
//! "failed". Good: N+1 runs.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use common::{Agent, agent_exe, until};
use serde_json::json;
use uc_update::manifest::{FileEntry, Kind, Manifest, SCHEMA, Signed, current_target};
use uc_update::sign::SecretKey;

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "uc-agent.exe"
    } else {
        "uc-agent"
    }
}

fn file_url(p: &Path) -> String {
    format!(
        "file:///{}",
        p.display()
            .to_string()
            .replace('\\', "/")
            .trim_start_matches('/')
    )
}

fn next_version() -> String {
    let v = env!("CARGO_PKG_VERSION");
    let mut parts: Vec<u64> = v.split('.').map(|x| x.parse().unwrap()).collect();
    parts[2] += 1;
    format!("{}.{}.{}", parts[0], parts[1], parts[2])
}

/// Write the feed for N+1 and return (latest.json URL, public key).
fn make_feed(root: &Path, version: &str, broken: bool) -> (String, String) {
    let key = SecretKey::generate();
    let payload = root.join("payload");
    std::fs::create_dir_all(&payload).unwrap();
    std::fs::copy(agent_exe(), payload.join(exe_name())).unwrap();
    std::fs::write(payload.join("test-version.txt"), version).unwrap();
    if broken {
        std::fs::write(payload.join("test-broken"), b"").unwrap();
    }
    let arc = root.join(format!("uclip-{version}.tar.gz"));
    uc_update::fetch::tar_dir(&payload, &arc).unwrap();
    let data = std::fs::read(&arc).unwrap();
    let name = arc.file_name().unwrap().to_string_lossy().to_string();
    let f = FileEntry::make(
        current_target(),
        Kind::Tar,
        &name,
        &file_url(&arc),
        version,
        &data,
        &key,
    )
    .unwrap();
    let m = Manifest {
        schema: SCHEMA,
        version: version.into(),
        released: "2026-11-14".into(),
        rollout: 100,
        files: vec![f],
    };
    let latest = root.join("latest.json");
    std::fs::write(
        &latest,
        serde_json::to_vec(&Signed::sign(&m, &key).unwrap()).unwrap(),
    )
    .unwrap();
    (file_url(&latest), key.public_b64())
}

/// Start version N from its own install folder with the feed switches.
fn start_n(a: &mut Agent, install: &Path, url: &str, key: &str) {
    std::fs::create_dir_all(install).unwrap();
    std::fs::copy(agent_exe(), install.join(exe_name())).unwrap();
    a.extra_env
        .push(("UCLIP_TEST_UPDATE_URL".into(), url.into()));
    a.extra_env
        .push(("UCLIP_TEST_UPDATE_KEY".into(), key.into()));
    let child = Command::new(install.join(exe_name()))
        .envs(a.env())
        .env_remove("UCLIP_TEST_CLIPBOARD")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    a.child = Some(child);
    a.wait_ready();
}

fn version_of(a: &Agent) -> Option<String> {
    a.try_call("status", json!({}))
        .ok()
        .and_then(|v| v["version"].as_str().map(str::to_string))
}

fn update_state(a: &Agent) -> serde_json::Value {
    std::fs::read(a.dir.join("update").join("update.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(json!({}))
}

fn helper_log(a: &Agent) -> String {
    std::fs::read_to_string(a.dir.join("update").join("helper.log")).unwrap_or_default()
}

/// Stop whichever agent of this profile runs now (the helper may have started it), and wait
/// until it is gone, so no process outlives the test.
fn stop_all(a: &mut Agent) {
    a.stop();
    let _ = a.try_call("debug.shutdown", json!({}));
    until(Duration::from_secs(10), || {
        !uc_agent::install::agent_running(&a.profile)
    });
}

/// How often this agent asked the window to open (test profiles only log it).
fn window_opens(a: &Agent) -> usize {
    std::fs::read_to_string(a.dir.join("update").join("window-opens.log"))
        .unwrap_or_default()
        .lines()
        .filter(|l| *l == "update")
        .count()
}

fn update_status(a: &Agent) -> serde_json::Value {
    a.try_call("update.status", json!({})).unwrap_or(json!({}))
}

/// Wait until the one check found `next` and the agent asks the user.
fn wait_available(a: &Agent, next: &str) {
    let ok = until(Duration::from_secs(60), || {
        let s = update_status(a);
        s["state"] == "available" && s["available"] == next
    });
    assert!(
        ok,
        "no update offered; status: {}\nupdate.json: {}\nagent log:\n{}",
        update_status(a),
        update_state(a),
        a.log()
    );
}

/// Decision 2026-11-14: the agent checks once at start, opens the window once, installs
/// nothing by itself, and "Later" holds until the next agent start.
#[test]
fn found_at_start_asks_once_and_never_installs_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let next = next_version();
    let (url, key) = make_feed(&tmp.path().join("feed"), &next, false);
    let install = tmp.path().join("install");
    let mut a = Agent::new(3);
    start_n(&mut a, &install, &url, &key);
    wait_available(&a, &next);
    assert_eq!(window_opens(&a), 1, "the window is asked for once");
    let s = a.uc_json(&["update"]);
    assert_eq!(s["state"], "available");
    assert_eq!(s["current"], env!("CARGO_PKG_VERSION"));
    // A long quiet run (the fast profile used to check every 5 s and install after 1 s idle):
    // no second check, no second window, nothing installed.
    std::thread::sleep(Duration::from_secs(12));
    let s = update_status(&a);
    assert_eq!(s["checks"], 1, "{s}");
    assert_eq!(window_opens(&a), 1);
    assert_eq!(version_of(&a).as_deref(), Some(env!("CARGO_PKG_VERSION")));
    assert!(helper_log(&a).is_empty(), "nothing was installed");
    // Later: dismissed until the agent starts again.
    let l = a.uc_json(&["update", "later"]);
    assert_eq!(l["dismissed"], true);
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(window_opens(&a), 1);
    // The next agent start checks once more and asks again.
    stop_all(&mut a);
    start_n(&mut a, &install, &url, &key);
    wait_available(&a, &next);
    assert_eq!(update_status(&a)["dismissed"], false);
    assert_eq!(window_opens(&a), 2);
    stop_all(&mut a);
}

#[test]
fn broken_update_rolls_back_to_n() {
    let tmp = tempfile::tempdir().unwrap();
    let next = next_version();
    let (url, key) = make_feed(&tmp.path().join("feed"), &next, true);
    let install: PathBuf = tmp.path().join("install");
    let mut a = Agent::new(1);
    start_n(&mut a, &install, &url, &key);
    assert_eq!(version_of(&a).as_deref(), Some(env!("CARGO_PKG_VERSION")));
    wait_available(&a, &next);
    // The user picks Update now (the window's button and `uc update --install` are the same call).
    let r = a.uc_json(&["update", "--install"]);
    assert_eq!(r["version"], next.as_str());
    // The helper tries N+1, it fails, N comes back and N+1 is marked bad.
    let ok = until(Duration::from_secs(90), || {
        update_state(&a)["bad"]
            .as_array()
            .is_some_and(|b| b.iter().any(|v| v == next.as_str()))
            && helper_log(&a).contains("RolledBack")
            && version_of(&a).as_deref() == Some(env!("CARGO_PKG_VERSION"))
    });
    assert!(
        ok,
        "no rollback; helper log:\n{}\nupdate.json: {}\nagent log:\n{}",
        helper_log(&a),
        update_state(&a),
        a.log()
    );
    assert!(!install.join("test-broken").exists(), "N is back on disk");
    // The agent that runs now (N) says the update failed, and asks the window once more.
    let ok = until(Duration::from_secs(20), || {
        update_status(&a)["state"] == "failed"
    });
    assert!(ok, "status: {}\nagent log:\n{}", update_status(&a), a.log());
    assert_eq!(update_status(&a)["failed_version"], next.as_str());
    // N stays: the bad version is not offered again.
    std::thread::sleep(Duration::from_secs(8));
    assert_eq!(version_of(&a).as_deref(), Some(env!("CARGO_PKG_VERSION")));
    assert_eq!(update_state(&a)["failures_in_row"], 1);
    assert_eq!(update_status(&a)["available"], serde_json::Value::Null);
    let report = a.call("diagnostics", json!({})).to_string();
    assert!(report.contains("bad versions"), "{report}");
    stop_all(&mut a);
}

#[test]
fn good_update_runs_n_plus_1() {
    let tmp = tempfile::tempdir().unwrap();
    let next = next_version();
    let (url, key) = make_feed(&tmp.path().join("feed"), &next, false);
    let install = tmp.path().join("install");
    let mut a = Agent::new(2);
    start_n(&mut a, &install, &url, &key);
    wait_available(&a, &next);
    let r = a.call("update.apply", json!({}));
    assert_eq!(r["applying"], true);
    let ok = until(Duration::from_secs(90), || {
        version_of(&a).as_deref() == Some(next.as_str()) && helper_log(&a).contains("Updated")
    });
    assert!(
        ok,
        "no update; helper log:\n{}\nupdate.json: {}\nagent log:\n{}",
        helper_log(&a),
        update_state(&a),
        a.log()
    );
    assert!(helper_log(&a).contains("Updated"));
    assert_eq!(update_state(&a)["last_good"], next.as_str());
    assert!(
        a.dir
            .join("update")
            .join("previous")
            .join(exe_name())
            .exists()
    );
    // N+1 is the newest: its own start check offers nothing.
    let ok = until(Duration::from_secs(20), || {
        update_status(&a)["checked"] == true
    });
    assert!(ok, "{}", update_status(&a));
    assert_eq!(update_status(&a)["state"], "none");
    stop_all(&mut a);
}
