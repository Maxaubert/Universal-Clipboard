//! Multi-agent tests (plan 6.2, milestone M7): two and three real agents on loopback, paired
//! through the `uc` CLI, with the copy matrix on test clipboards.

mod common;

use std::time::Duration;

use common::*;
use serde_json::json;

/// Three agents: pair through the CLI (the third from the second device), the copy matrix,
/// device list, rename, pause, settings, remove, leave.
#[test]
fn three_agents_pair_sync_and_manage_through_the_cli() {
    let mut a = Agent::new(11);
    let mut b = Agent::new(12);
    let mut c = Agent::new(13);
    a.start();
    b.start();
    c.start();
    assert_eq!(a.uc_json(&["status"])["state"], "not-set-up");
    a.uc_json(&["settings", "set", "device-name", "Alpha"]);
    b.uc_json(&["settings", "set", "device-name", "Bravo"]);
    c.uc_json(&["settings", "set", "device-name", "Charlie"]);

    pair(&a, &mut b);
    pair(&b, &mut c); // A third device can be added from any device.
    for x in [&a, &b, &c] {
        assert!(
            until(
                Duration::from_secs(10),
                || x.uc_json(&["status"])["connected"] == 2
            ),
            "agent {} not connected to both: {}",
            x.n,
            x.uc_json(&["status"])
        );
        let d = x.uc_json(&["devices"]);
        assert_eq!(d.as_array().unwrap().len(), 3);
        assert_eq!(x.uc_json(&["status"])["state"], "working");
    }
    copy_matrix(&[&a, &b, &c], "three");

    // Rename on C, seen on A.
    c.uc_json(&["rename", "Alpha", "Alpha PC"]);
    assert!(until(Duration::from_secs(5), || {
        a.uc_json(&["devices"])
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["name"] == "Alpha PC" && d["this"] == true)
    }));

    // Pause on B: A's copy does not reach B; resume and the next copy does.
    b.uc_json(&["pause"]);
    assert_eq!(b.uc_json(&["status"])["state"], "paused");
    a.copy("while paused");
    assert!(until(Duration::from_secs(5), || c.clipboard().as_deref()
        == Some("while paused")));
    std::thread::sleep(Duration::from_millis(500));
    assert_ne!(b.clipboard().as_deref(), Some("while paused"));
    b.uc_json(&["resume"]);
    copy_matrix(&[&a, &b, &c], "after resume");

    // Settings.
    let s = a.uc_json(&["settings"]);
    assert_eq!(s["sync-images"]["value"], "on");
    a.uc_json(&["settings", "set", "sync-images", "off"]);
    assert_eq!(a.uc_json(&["settings"])["sync-images"]["value"], "off");
    assert!(
        !a.uc(&["settings", "set", "sync-images", "maybe"])
            .status
            .success()
    );

    // Remove C from A: C learns it, B drops it, C's copies go nowhere.
    a.uc_json(&["remove", "Charlie", "--yes"]);
    assert!(
        until(
            Duration::from_secs(10),
            || c.uc_json(&["status"])["fix_code"] == "removed"
        ),
        "C must see that it was removed: {}\n{}",
        c.uc_json(&["status"]),
        c.log()
    );
    assert!(until(Duration::from_secs(10), || {
        b.uc_json(&["devices"]).as_array().unwrap().len() == 2
    }));
    c.copy("from the removed device");
    std::thread::sleep(Duration::from_millis(800));
    assert_ne!(a.clipboard().as_deref(), Some("from the removed device"));
    assert_ne!(b.clipboard().as_deref(), Some("from the removed device"));
    copy_matrix(&[&a, &b], "two left");

    // B leaves: A is alone again.
    b.uc_json(&["leave", "--yes"]);
    assert!(until(Duration::from_secs(10), || {
        a.uc_json(&["devices"]).as_array().unwrap().len() == 1
    }));
    assert_eq!(b.uc_json(&["status"])["state"], "not-set-up");

    // The diagnostic report has no clipboard content.
    let r = a.uc_json(&["diagnostics"])["report"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(r.contains("LAN only: relay=false"));
    assert!(!r.contains("three from agent"));
    assert!(!r.contains("after resume"));
    // Logs neither.
    for x in [&a, &b, &c] {
        let log = x.log();
        assert!(!log.contains("from agent"), "content in the log of {}", x.n);
        assert!(!log.contains("while paused"));
    }
}

/// Restart: an agent that stops and starts again with its state reconnects and syncs.
/// Also: a second agent of the same user and profile refuses to start (single agent).
#[test]
fn restart_reconnects_and_one_agent_per_profile() {
    let mut a = Agent::new(21);
    let mut b = Agent::new(22);
    a.start();
    b.start();
    pair(&a, &mut b);
    copy_matrix(&[&a, &b], "before restart");

    // Single agent per user and profile.
    let o = std::process::Command::new(agent_exe())
        .envs(a.env())
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(3), "second agent must exit with 3");

    b.stop();
    a.copy("while B is off");
    b.start();
    assert!(
        until(
            Duration::from_secs(15),
            || a.uc_json(&["status"])["connected"] == 1
        ),
        "{}",
        b.log()
    );
    // A copy made while B was off is under 1 hour old: B gets it on reconnect (plan 2.4).
    assert!(
        until(Duration::from_secs(10), || b.clipboard().as_deref()
            == Some("while B is off")),
        "B has {:?}",
        b.clipboard()
    );
    copy_matrix(&[&a, &b], "after restart");
    // Wake hook: reconnects at once and keeps syncing.
    a.call("debug.wake", json!({}));
    copy_matrix(&[&a, &b], "after wake");
}

/// Wrong code: plain error, nothing joined. The inviter shows "Someone else tried this code".
#[test]
fn wrong_code_is_refused_with_the_plan_text() {
    use std::io::{BufRead, BufReader};
    let mut a = Agent::new(31);
    let mut b = Agent::new(32);
    a.start();
    b.start();
    let mut show = std::process::Command::new(env!("CARGO_BIN_EXE_uc"))
        .args(["pair", "show", "--json"])
        .envs(a.env())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(show.stdout.take().unwrap());
    let mut line = String::new();
    out.read_line(&mut line).unwrap();
    let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    let code = v["code"].as_str().unwrap().replace(' ', "");
    let wrong = if code == "000000" { "000001" } else { "000000" };
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_uc"))
        .args(["pair", "enter", wrong])
        .envs(b.env())
        .env(
            "UCLIP_TEST_PAIR_ADDRS",
            format!("{}:{}", a.ip, v["port"].as_u64().unwrap()),
        )
        .output()
        .unwrap();
    assert!(!o.status.success());
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("That code is not right"), "{err}");
    let mut tried = String::new();
    out.read_line(&mut tried).unwrap();
    assert!(tried.contains("Someone else tried this code"), "{tried}");
    assert_eq!(b.uc_json(&["status"])["state"], "not-set-up");
    let _ = a.uc(&["pair", "cancel"]);
    let _ = show.wait();
}

/// `uc setup` registers start at login (in the test sandbox, never the real login items)
/// and `uc settings set start-at-login off` removes it again.
#[test]
fn setup_and_autostart_in_the_sandbox() {
    let mut a = Agent::new(41);
    a.start();
    let v = a.uc_json(&["setup", "--yes"]);
    assert_eq!(v["sandboxed"], true);
    let rows = v["rows"].as_array().unwrap();
    let auto = rows.iter().find(|r| r["row"] == "autostart").unwrap();
    assert_eq!(auto["state"], "done");
    let at = uc_agent::autostart::Autostart::new(&a.profile, "x".into(), &a.dir);
    assert!(at.is_enabled());
    a.uc_json(&["settings", "set", "start-at-login", "off"]);
    assert!(!at.is_enabled());
    // Text output, no colours when not a terminal.
    let o = a.uc(&["status"]);
    let s = String::from_utf8_lossy(&o.stdout);
    assert!(!s.contains('\u{1b}'), "no colour codes");
    assert!(s.contains("Not set up"), "{s}");
}
