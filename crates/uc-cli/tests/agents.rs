//! Multi-agent tests (plan 6.2, milestone M7): two and three real agents on loopback, paired
//! through the `uc` CLI, with the copy matrix on test clipboards. File sync (spec 7): folders,
//! the exec bit, a zip, the limit, `uc copy` of a 300 MB file, cancel, stop, keep-3, the image
//! companion and an old peer. Fake clipboards; the files are real, in temp folders.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::*;
use serde_json::{Value, json};

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

/// Decision 2026-11-14 (Max): start at login is on from the first run (in the test sandbox,
/// never the real login items), it is not a setup row, and when the user turns it off a
/// restart never turns it on again. `uc setup` only asks the rows that are left.
#[test]
fn start_at_login_is_on_by_default_in_the_sandbox() {
    let mut a = Agent::new(41);
    a.start();
    let at = uc_agent::autostart::Autostart::new(&a.profile, "x".into(), &a.dir);
    assert!(at.is_enabled(), "on after the first run");
    let v = a.uc_json(&["setup", "--yes"]);
    assert_eq!(v["sandboxed"], true);
    assert_eq!(v["done"], true);
    let rows = v["rows"].as_array().unwrap();
    for r in rows {
        assert!(
            !matches!(r["row"].as_str(), Some("autostart" | "firewall")),
            "{r}"
        );
    }
    if !cfg!(target_os = "macos") {
        assert!(rows.is_empty(), "{rows:?}");
    }
    a.uc_json(&["settings", "set", "start-at-login", "off"]);
    assert!(!at.is_enabled());
    a.stop();
    a.start();
    assert!(!at.is_enabled(), "a restart keeps the user's off");
    assert_eq!(a.uc_json(&["settings"])["start-at-login"]["value"], "off");
    // Text output, no colours when not a terminal.
    let o = a.uc(&["status"]);
    let s = String::from_utf8_lossy(&o.stdout);
    assert!(!s.contains('\u{1b}'), "no colour codes");
    assert!(s.contains("Not set up"), "{s}");
}

/// Decision 2026-11-14 (Max): the network never waits for a setup answer. A missing Windows
/// firewall rule (faked here: test profiles never look at the real rule) gives outbound-only
/// mode and a non-blocking note with the fix; `uc firewall check` reports it.
#[test]
fn missing_firewall_rule_is_a_note_not_a_setup_step() {
    let mut a = Agent::new(42);
    a.extra_env
        .push(("UCLIP_TEST_FIREWALL_RULE".into(), "absent".into()));
    a.start();
    let st = a.uc_json(&["status"]);
    assert_eq!(st["state"], "not-set-up", "{st}");
    assert_eq!(st["note"]["code"], "firewall", "{st}");
    assert!(
        st["note"]["fix"]
            .as_str()
            .unwrap()
            .contains("uc firewall add")
    );
    let v = a.uc_json(&["firewall", "check"]);
    assert_eq!(v["rule"], "absent");
    assert_eq!(v["incoming"], false);
    let d = a.uc_json(&["diagnostics"]);
    let r = d["report"].as_str().unwrap();
    assert!(r.contains("Network: on"), "{r}");
    assert!(r.contains("Firewall rule: absent (incoming: false)"), "{r}");
    let o = a.uc(&["status"]);
    let s = String::from_utf8_lossy(&o.stdout);
    assert!(s.contains("Note: Other devices cannot connect"), "{s}");

    // Default test profile: no rule of ours is needed, no note.
    let mut b = Agent::new(43);
    b.start();
    assert!(b.uc_json(&["status"])["note"].is_null());
    assert_eq!(b.uc_json(&["firewall", "check"])["rule"], "not-needed");
    // `uc firewall add` in a test profile never elevates; it only checks again.
    assert_eq!(b.uc_json(&["firewall", "add"])["rule"], "not-needed");
}

/// Lockout ladder (protocol 4.6): the 5th wrong code turns the code off for 1 minute. The
/// inviter shows the time left, and an agent restart keeps the lockout.
#[test]
fn five_wrong_codes_lock_out_and_a_restart_keeps_it() {
    use std::io::{BufRead, BufReader};
    let mut a = Agent::new(33);
    let mut b = Agent::new(34);
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
    assert!(v["notice"].is_null(), "{v}");
    let code = v["code"].as_str().unwrap().replace(' ', "");
    let wrong = if code == "000000" { "000001" } else { "000000" };
    let mut last = serde_json::Value::Null;
    for i in 1..=5 {
        let o = std::process::Command::new(env!("CARGO_BIN_EXE_uc"))
            .args(["pair", "enter", wrong])
            .envs(b.env())
            .env(
                "UCLIP_TEST_PAIR_ADDRS",
                format!("{}:{}", a.ip, v["port"].as_u64().unwrap()),
            )
            .output()
            .unwrap();
        assert!(!o.status.success(), "try {i}");
        let mut tried = String::new();
        out.read_line(&mut tried).unwrap();
        last = serde_json::from_str(tried.trim()).unwrap();
        if i < 5 {
            assert!(last["data"]["lockout"].is_null(), "try {i}: {last}");
        }
    }
    assert_eq!(
        last["data"]["lockout"], "Too many wrong codes. Try again in 1 minute.",
        "{last}"
    );
    let _ = a.uc(&["pair", "cancel"]);
    let _ = show.wait();

    // A restart keeps the lockout: no code, the time left, the QR code still there.
    a.stop();
    a.start();
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
    assert!(v["code"].is_null(), "{v}");
    assert_eq!(
        v["notice"], "Too many wrong codes. Try again in 1 minute.",
        "{v}"
    );
    let left = v["lockout_ms"].as_u64().unwrap();
    assert!(left > 0 && left <= 60_000, "{v}");
    assert!(v["qr"].is_string(), "{v}");
    let _ = a.uc(&["pair", "cancel"]);
    let _ = show.wait();
}

/// No usable network on this device (2026-11-14, Max's PC: every adapter was skipped, so no
/// socket was bound). Status and `uc pair enter` say so plainly, and the log says which
/// interfaces were skipped and why.
#[test]
fn no_usable_network_is_named_in_status_and_pairing() {
    let mut a = Agent::new(61);
    a.remove_env.push("UCLIP_TEST_LOOPBACK".into());
    a.extra_env.push(("UCLIP_TEST_NO_LAN".into(), "1".into()));
    a.start();
    let mut st = json!(null);
    assert!(
        until(Duration::from_secs(10), || {
            st = a.uc_json(&["status"]);
            st["fix_code"] == "no-lan"
        }),
        "{st}\n{}",
        a.log()
    );
    assert_eq!(st["state"], "needs-attention");
    assert_eq!(st["reason"], "This device found no network it can use.");
    let o = a.uc(&["pair", "enter", "123456"]);
    assert!(!o.status.success());
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains("This device found no network it can use."),
        "{err}"
    );
    let log = a.log();
    assert!(log.contains("no usable network"), "{log}");
    assert!(
        log.contains("vEthernet (Default Switch): virtual, no LAN default gateway"),
        "{log}"
    );
}

// ------------------------------------------------------------------ file sync (spec 7)

/// Bytes that differ along the file, so a misplaced chunk is caught.
fn pattern(n: usize, seed: u8) -> Vec<u8> {
    (0..n)
        .map(|i| (i as u32).wrapping_mul(2654435761).to_le_bytes()[1] ^ seed)
        .collect()
}

/// The file paths on an agent's (test) clipboard.
fn files_on(a: &Agent) -> Vec<PathBuf> {
    a.call("debug.clipboard", json!({}))["files"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .map(PathBuf::from)
        .collect()
}

/// The newest activity entry.
fn last_activity(a: &Agent) -> Value {
    a.uc_json(&["status"])["activity"][0].clone()
}

/// True if one of the last activity entries has exactly this note.
fn has_note(a: &Agent, note: &str) -> bool {
    a.uc_json(&["status"])["activity"]
        .as_array()
        .is_some_and(|l| l.iter().any(|x| x["note"] == note))
}

/// What moves now (`transfer` of `uc status --json`).
fn transfer(a: &Agent) -> Value {
    a.uc_json(&["status"])["transfer"].clone()
}

/// Received copies in an agent's files folder (`<state>/files` for test profiles).
fn received_folders(a: &Agent) -> Vec<String> {
    std::fs::read_dir(a.dir.join("files"))
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// Two started agents named Alpha and Bravo, paired and connected. `a_env` and `b_env` are
/// extra test switches for each.
fn alpha_bravo(n: u8, a_env: &[(&str, &str)], b_env: &[(&str, &str)]) -> (Agent, Agent) {
    let mut a = Agent::new(n);
    let mut b = Agent::new(n + 1);
    for (x, env) in [(&mut a, a_env), (&mut b, b_env)] {
        x.extra_env
            .extend(env.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        x.start();
    }
    a.uc_json(&["settings", "set", "device-name", "Alpha"]);
    b.uc_json(&["settings", "set", "device-name", "Bravo"]);
    pair(&a, &mut b);
    assert!(
        until(Duration::from_secs(10), || {
            a.uc_json(&["status"])["connected"] == 1 && b.uc_json(&["status"])["connected"] == 1
        }),
        "not connected:\n{}",
        a.log()
    );
    (a, b)
}

/// Every entry under `root`: `/`-separated relative path, and the bytes (`None` for a
/// folder).
fn tree(root: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Option<Vec<u8>>>) {
        for e in std::fs::read_dir(dir).unwrap().map(Result::unwrap) {
            let p = e.path();
            let rel = p
                .strip_prefix(base)
                .unwrap()
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            if p.is_dir() {
                out.insert(rel, None);
                walk(base, &p, out);
            } else {
                out.insert(rel, Some(std::fs::read(&p).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// CRC-32 (IEEE), for the test zip.
fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for b in data {
        c ^= u32::from(*b);
        for _ in 0..8 {
            c = if c & 1 != 0 {
                (c >> 1) ^ 0xEDB8_8320
            } else {
                c >> 1
            };
        }
    }
    !c
}

/// A real zip archive with one stored (not compressed) file.
fn zip_with(name: &str, data: &[u8]) -> Vec<u8> {
    let crc = crc32(data);
    let len = data.len() as u32;
    let n = name.len() as u16;
    let mut z = Vec::new();
    // Local file header.
    z.extend(0x0403_4b50u32.to_le_bytes());
    z.extend(20u16.to_le_bytes()); // version needed
    z.extend([0u8; 4]); // flags, method (stored)
    z.extend(0u16.to_le_bytes()); // time
    z.extend(0x21u16.to_le_bytes()); // date 1980-01-01
    z.extend(crc.to_le_bytes());
    z.extend(len.to_le_bytes());
    z.extend(len.to_le_bytes());
    z.extend(n.to_le_bytes());
    z.extend(0u16.to_le_bytes());
    z.extend(name.as_bytes());
    z.extend(data);
    let cd = z.len() as u32;
    // Central directory.
    z.extend(0x0201_4b50u32.to_le_bytes());
    z.extend(20u16.to_le_bytes());
    z.extend(20u16.to_le_bytes());
    z.extend([0u8; 4]);
    z.extend(0u16.to_le_bytes());
    z.extend(0x21u16.to_le_bytes());
    z.extend(crc.to_le_bytes());
    z.extend(len.to_le_bytes());
    z.extend(len.to_le_bytes());
    z.extend(n.to_le_bytes());
    z.extend([0u8; 12]); // extra, comment, disk, internal and external attributes
    z.extend(0u32.to_le_bytes()); // offset of the local header
    z.extend(name.as_bytes());
    let cd_len = z.len() as u32 - cd;
    // End of central directory.
    z.extend(0x0605_4b50u32.to_le_bytes());
    z.extend([0u8; 4]);
    z.extend(1u16.to_le_bytes());
    z.extend(1u16.to_le_bytes());
    z.extend(cd_len.to_le_bytes());
    z.extend(cd.to_le_bytes());
    z.extend(0u16.to_le_bytes());
    z
}

/// A 1x1 PNG.
const PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

/// BLAKE3 of a file, read in pieces.
fn file_hash(p: &Path) -> blake3::Hash {
    let mut h = blake3::Hasher::new();
    h.update_reader(std::fs::File::open(p).unwrap()).unwrap();
    h.finalize()
}

/// Write `mib` MiB of the pattern to `p`, 1 MiB at a time.
fn write_big(p: &Path, mib: usize) {
    use std::io::Write;
    let mut f = std::io::BufWriter::new(std::fs::File::create(p).unwrap());
    for i in 0..mib {
        f.write_all(&pattern(1 << 20, i as u8)).unwrap();
    }
    f.flush().unwrap();
}

/// Wait until `b` receives from Alpha and some bytes are in; `a` shows the send.
fn wait_receiving(a: &Agent, b: &Agent) {
    let mut t = Value::Null;
    assert!(
        until(Duration::from_secs(20), || {
            t = transfer(b);
            t["phase"] == "receiving" && t["done"].as_u64().unwrap_or(0) > 0
        }),
        "no transfer on B: {t}\nA:\n{}\nB:\n{}",
        a.log(),
        b.log()
    );
    assert_eq!(t["device"], "Alpha");
    let s = transfer(a);
    assert_eq!(s["phase"], "sending", "{s}");
    assert_eq!(s["device"], "Bravo", "{s}");
}

/// An automatic copy of a folder tree (nested files, an empty folder, an executable script)
/// and a zip next to it: everything arrives byte for byte, the exec bit too (macOS and Linux),
/// and the zip stays a zip.
#[test]
fn folder_tree_exec_bit_and_zip_arrive_intact() {
    let (a, b) = alpha_bravo(71, &[], &[]);
    let src = tempfile::tempdir().unwrap();
    let proj = src.path().join("Project");
    std::fs::create_dir_all(proj.join("src").join("deep").join("er")).unwrap();
    std::fs::create_dir_all(proj.join("empty")).unwrap();
    std::fs::create_dir_all(proj.join("bin")).unwrap();
    std::fs::write(proj.join("src").join("main.rs"), b"fn main() {}\n").unwrap();
    std::fs::write(
        proj.join("src").join("deep").join("er").join("data.bin"),
        pattern(700_000, 9),
    )
    .unwrap();
    std::fs::write(proj.join("bin").join("run.sh"), b"#!/bin/sh\necho hi\n").unwrap();
    std::fs::write(proj.join("README"), b"").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let run = proj.join("bin").join("run.sh");
        std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let zip = src.path().join("archive.zip");
    std::fs::write(&zip, zip_with("hello.txt", b"hello from the zip")).unwrap();

    a.call("debug.copy_files", json!({ "paths": [proj, zip] }));
    assert!(
        until(Duration::from_secs(20), || files_on(&b).len() == 2),
        "files did not arrive:\nA:\n{}\nB:\n{}",
        a.log(),
        b.log()
    );
    let got = files_on(&b);
    let dir = got.iter().find(|p| p.is_dir()).expect("the folder");
    let z = got.iter().find(|p| p.is_file()).expect("the zip");
    assert_eq!(dir.file_name().unwrap(), "Project");
    assert_eq!(z.file_name().unwrap(), "archive.zip");
    assert_eq!(tree(dir), tree(&proj), "the tree arrives as it was");
    assert!(dir.join("empty").is_dir());
    assert_eq!(std::fs::read(z).unwrap(), std::fs::read(&zip).unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir.join("bin").join("run.sh")), 0o755);
        assert_eq!(mode(&dir.join("src").join("main.rs")), 0o644);
    }
    let act = last_activity(&b);
    assert_eq!(act["direction"], "received");
    assert_eq!(act["device"], "Alpha");
    // 5 files: main.rs, data.bin, run.sh, README and the zip.
    assert!(
        act["kind"].as_str().unwrap().starts_with("5 files, "),
        "{act}"
    );
    // A received copy is never sent back: A's clipboard still has its own paths.
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(files_on(&a), vec![proj.clone(), zip.clone()]);
}

/// `uc copy` of a 300 MB file: over the automatic limit (100 MB), sent anyway because it was
/// picked, intact on the other side.
#[test]
fn uc_copy_sends_a_300_mb_file() {
    let (a, b) = alpha_bravo(73, &[], &[]);
    assert_eq!(a.uc_json(&["settings"])["file-limit-mb"]["value"], "100");
    let src = tempfile::tempdir().unwrap();
    let big = src.path().join("big.bin");
    write_big(&big, 300);
    let t = std::time::Instant::now();
    let r = a.uc_json(&["copy", big.to_str().unwrap()]);
    assert_eq!(r["preparing"], true);
    assert!(
        until(Duration::from_secs(300), || files_on(&b).len() == 1),
        "300 MB did not arrive:\nA:\n{}\nB:\n{}",
        a.log(),
        b.log()
    );
    let took = t.elapsed();
    eprintln!(
        "300 MiB by uc copy: {took:?} ({:.0} MiB/s, hash and send)",
        300.0 / took.as_secs_f64()
    );
    let got = files_on(&b).remove(0);
    assert_eq!(got.file_name().unwrap(), "big.bin");
    assert_eq!(std::fs::metadata(&got).unwrap().len(), 300 << 20);
    assert_eq!(file_hash(&got), file_hash(&big));
    let act = last_activity(&b);
    assert!(
        act["kind"].as_str().unwrap().starts_with("1 file, "),
        "{act}"
    );
    assert!(act["note"].is_null(), "{act}");
    assert!(
        !has_note(
            &a,
            "Files over 100 MB are not sent automatically. Use Copy files... in the app."
        ),
        "a picked copy gives no over-limit note"
    );
    assert!(b.uc_json(&["status"])["last_transfer_error"].is_null());
}

/// A newer copy cancels a running transfer: text copied on the sender, then text copied on
/// the receiver. The partial folder is deleted and the receiver says why.
#[test]
fn a_newer_copy_cancels_the_running_transfer() {
    // 4 MB/s, so 40 MiB takes about 10 s.
    let (a, b) = alpha_bravo(75, &[("UCLIP_TEST_SEND_RATE", "4000000")], &[]);
    let src = tempfile::tempdir().unwrap();
    let one = src.path().join("one.bin");
    let two = src.path().join("two.bin");
    write_big(&one, 40);
    write_big(&two, 41);

    // A newer copy on the sending device.
    a.uc_json(&["copy", one.to_str().unwrap()]);
    wait_receiving(&a, &b);
    a.copy("newer text on Alpha");
    assert!(
        until(Duration::from_secs(10), || b.clipboard().as_deref()
            == Some("newer text on Alpha")),
        "{}",
        b.log()
    );
    assert!(until(Duration::from_secs(10), || {
        transfer(&a).is_null() && transfer(&b).is_null()
    }));
    assert!(
        has_note(&b, "A newer copy replaced these files."),
        "{}",
        b.uc_json(&["status"])
    );
    assert!(until(Duration::from_secs(5), || received_folders(&b).is_empty()));
    assert!(files_on(&b).is_empty());

    // A newer copy on the receiving device.
    a.uc_json(&["copy", two.to_str().unwrap()]);
    wait_receiving(&a, &b);
    b.copy("newer text on Bravo");
    assert!(
        until(Duration::from_secs(10), || a.clipboard().as_deref()
            == Some("newer text on Bravo")),
        "{}",
        a.log()
    );
    assert!(until(Duration::from_secs(10), || {
        transfer(&a).is_null() && transfer(&b).is_null()
    }));
    assert!(until(Duration::from_secs(5), || received_folders(&b).is_empty()));
    assert!(files_on(&b).is_empty());
    // Text still syncs both ways.
    copy_matrix(&[&a, &b], "after cancel");
}

/// `uc transfer stop` on the receiver, then on the sender: the transfer ends on both sides,
/// nothing lands on the receiver's clipboard, and the notes say who stopped it.
#[test]
fn uc_transfer_stop_ends_it_on_both_sides() {
    let (a, b) = alpha_bravo(77, &[("UCLIP_TEST_SEND_RATE", "4000000")], &[]);
    let src = tempfile::tempdir().unwrap();
    let one = src.path().join("one.bin");
    let two = src.path().join("two.bin");
    write_big(&one, 40);
    write_big(&two, 41);

    a.uc_json(&["copy", one.to_str().unwrap()]);
    wait_receiving(&a, &b);
    assert_eq!(b.uc_json(&["transfer", "stop"])["stopped"], true);
    assert!(until(Duration::from_secs(10), || {
        transfer(&a).is_null() && transfer(&b).is_null()
    }));
    assert!(has_note(&b, "Stopped."), "{}", b.uc_json(&["status"]));
    assert_eq!(b.uc_json(&["transfer", "stop"])["stopped"], false);
    std::thread::sleep(Duration::from_millis(1000));
    assert!(
        files_on(&b).is_empty(),
        "a stopped copy is not fetched again"
    );
    assert!(received_folders(&b).is_empty());

    a.uc_json(&["copy", two.to_str().unwrap()]);
    wait_receiving(&a, &b);
    let o = a.uc(&["transfer", "stop"]);
    assert!(o.status.success());
    assert!(until(Duration::from_secs(10), || {
        transfer(&a).is_null() && transfer(&b).is_null()
    }));
    assert!(
        until(Duration::from_secs(5), || has_note(
            &b,
            "Alpha stopped sending the files."
        )),
        "{}",
        b.uc_json(&["status"])
    );
    assert!(until(Duration::from_secs(5), || received_folders(&b).is_empty()));
    assert!(files_on(&b).is_empty());
}

/// A single small image file also puts the image itself on the receiver's clipboard (spec
/// 2.2.5). Not for a text file, and not when Sync images is off there.
#[test]
fn a_single_image_file_brings_the_image_along() {
    let (a, b) = alpha_bravo(79, &[], &[]);
    let src = tempfile::tempdir().unwrap();
    let png = src.path().join("shot.png");
    std::fs::write(&png, PNG_1X1).unwrap();
    let png_hash = blake3::hash(PNG_1X1).to_hex().to_string();
    let arrived = |name: &str| {
        until(Duration::from_secs(15), || {
            files_on(&b)
                .first()
                .is_some_and(|p| p.file_name().is_some_and(|n| n == name))
        })
    };

    a.call("debug.copy_files", json!({ "paths": [png] }));
    assert!(arrived("shot.png"), "{}", b.log());
    let c = b.call("debug.clipboard", json!({}));
    assert_eq!(c["png"], png_hash.as_str(), "{c}");

    let txt = src.path().join("notes.txt");
    std::fs::write(&txt, b"no image").unwrap();
    a.call("debug.copy_files", json!({ "paths": [txt] }));
    assert!(arrived("notes.txt"));
    assert!(b.call("debug.clipboard", json!({}))["png"].is_null());

    b.uc_json(&["settings", "set", "sync-images", "off"]);
    let png2 = src.path().join("shot2.png");
    std::fs::write(&png2, PNG_1X1).unwrap();
    a.call("debug.copy_files", json!({ "paths": [png2] }));
    assert!(arrived("shot2.png"));
    assert!(b.call("debug.clipboard", json!({}))["png"].is_null());
}

/// A peer without the files bit (a 0.1.x agent) stays connected, gets no files, and the
/// copying device says it needs an update. Text keeps syncing.
#[test]
fn an_old_peer_stays_connected_and_gets_the_note() {
    let (a, b) = alpha_bravo(81, &[], &[("UCLIP_TEST_NO_FILES", "1")]);
    let src = tempfile::tempdir().unwrap();
    let f = src.path().join("small.txt");
    std::fs::write(&f, b"for the old peer").unwrap();
    a.call("debug.copy_files", json!({ "paths": [f] }));
    assert!(
        until(Duration::from_secs(10), || has_note(
            &a,
            "Bravo needs an update to receive files."
        )),
        "{}",
        a.uc_json(&["status"])
    );
    std::thread::sleep(Duration::from_millis(800));
    assert!(files_on(&b).is_empty());
    assert!(received_folders(&b).is_empty());
    assert_eq!(a.uc_json(&["status"])["connected"], 1);
    assert_eq!(b.uc_json(&["status"])["connected"], 1);
    copy_matrix(&[&a, &b], "old peer");
}

/// `uc copy` of a folder and a file, an automatic copy over the limit (note on this device),
/// keep-3 in the files folder, Sync files off on the receiver, and no file names in logs.
#[test]
fn files_reach_the_other_agent() {
    let mut a = Agent::new(91);
    let mut b = Agent::new(92);
    a.start();
    b.start();
    a.uc_json(&["settings", "set", "device-name", "Alpha"]);
    b.uc_json(&["settings", "set", "device-name", "Bravo"]);
    pair(&a, &mut b);
    assert!(
        until(Duration::from_secs(10), || {
            a.uc_json(&["status"])["connected"] == 1 && b.uc_json(&["status"])["connected"] == 1
        }),
        "not connected:\n{}",
        a.log()
    );
    assert_eq!(a.uc_json(&["settings"])["sync-files"]["value"], "on");
    assert_eq!(a.uc_json(&["settings"])["file-limit-mb"]["value"], "100");

    // A folder with a nested file, an empty folder, and a loose file, picked with `uc copy`.
    let src = tempfile::tempdir().unwrap();
    let photos = src.path().join("Secret Photos");
    std::fs::create_dir_all(photos.join("trip")).unwrap();
    std::fs::create_dir_all(photos.join("empty")).unwrap();
    let big = pattern(3 << 20, 7);
    std::fs::write(photos.join("trip").join("beach.bin"), &big).unwrap();
    std::fs::write(photos.join("note-zebra.txt"), b"hello").unwrap();
    let loose = src.path().join("report-walrus.pdf");
    std::fs::write(&loose, pattern(1000, 3)).unwrap();
    let missing = src.path().join("nope");
    let o = a.uc(&["copy", missing.to_str().unwrap()]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("There is no file or folder"));

    let r = a.uc_json(&["copy", photos.to_str().unwrap(), loose.to_str().unwrap()]);
    assert_eq!(r["preparing"], true);
    assert!(
        until(Duration::from_secs(20), || files_on(&b).len() == 2),
        "files did not arrive:\nA:\n{}\nB:\n{}",
        a.log(),
        b.log()
    );
    let got = files_on(&b);
    let root = b.dir.join("files");
    for p in &got {
        assert!(p.is_absolute() && p.starts_with(&root), "{}", p.display());
    }
    let dir = got.iter().find(|p| p.is_dir()).expect("the folder");
    let file = got.iter().find(|p| p.is_file()).expect("the file");
    assert_eq!(dir.file_name().unwrap(), "Secret Photos");
    assert_eq!(
        std::fs::read(dir.join("trip").join("beach.bin")).unwrap(),
        big
    );
    assert_eq!(std::fs::read(dir.join("note-zebra.txt")).unwrap(), b"hello");
    assert!(dir.join("empty").is_dir());
    assert_eq!(std::fs::read(file).unwrap(), pattern(1000, 3));
    // No partial folder is left.
    assert!(received_folders(&b).iter().all(|n| !n.ends_with(".part")));
    let first = dir
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap();

    // The picked items are on A's own clipboard too (written with our marker).
    let mine = files_on(&a);
    assert_eq!(mine, vec![photos.clone(), loose.clone()]);

    // Last activity: counts and sizes only.
    let ra = last_activity(&b);
    assert_eq!(ra["direction"], "received");
    assert_eq!(ra["device"], "Alpha");
    assert_eq!(ra["kind"], "3 files, 3 MB");
    assert!(until(
        Duration::from_secs(5),
        || last_activity(&a)["direction"] == "sent"
    ));
    assert_eq!(last_activity(&a)["kind"], "3 files, 3 MB");
    assert!(until(Duration::from_secs(5), || {
        a.uc_json(&["status"])["transfer"].is_null() && b.uc_json(&["status"])["transfer"].is_null()
    }));
    assert!(b.uc_json(&["status"])["last_transfer_error"].is_null());
    assert_eq!(a.uc_json(&["transfer", "stop"])["stopped"], false);

    // An automatic copy over the limit is not sent, with a note on this device.
    a.uc_json(&["settings", "set", "file-limit-mb", "1"]);
    let over = src.path().join("over.bin");
    std::fs::write(&over, pattern(2 << 20, 1)).unwrap();
    a.call("debug.copy_files", json!({ "paths": [over] }));
    assert!(
        until(Duration::from_secs(10), || {
            last_activity(&a)["note"]
                .as_str()
                .is_some_and(|n| n.contains("Files over 1 MB are not sent automatically"))
        }),
        "{}",
        a.uc_json(&["status"])
    );

    // Small automatic copies sync; the files folder keeps the last 3.
    for i in 0..3 {
        let f = src.path().join(format!("small-{i}.txt"));
        std::fs::write(&f, format!("small {i}")).unwrap();
        a.call("debug.copy_files", json!({ "paths": [f] }));
        assert!(
            until(Duration::from_secs(15), || {
                files_on(&b)
                    .first()
                    .and_then(|p| std::fs::read_to_string(p).ok())
                    .as_deref()
                    == Some(format!("small {i}").as_str())
            }),
            "small copy {i} did not arrive:\nA:\n{}\nB:\n{}",
            a.log(),
            b.log()
        );
    }
    assert!(
        until(Duration::from_secs(5), || received_folders(&b).len() == 3),
        "{:?}",
        received_folders(&b)
    );
    assert!(
        !received_folders(&b).contains(&first),
        "the oldest of 4 copies is deleted: {:?}",
        received_folders(&b)
    );

    // Sync files off on the receiver: a new copy is not fetched.
    b.uc_json(&["settings", "set", "sync-files", "off"]);
    let f = src.path().join("while-off.txt");
    std::fs::write(&f, b"off").unwrap();
    a.call("debug.copy_files", json!({ "paths": [f] }));
    assert!(until(Duration::from_secs(5), || last_activity(&a)["kind"]
        == "1 file, 3 bytes"));
    std::thread::sleep(Duration::from_millis(800));
    assert!(files_on(&b).iter().all(|p| !p.ends_with("while-off.txt")));
    // And `uc copy` there says why it cannot.
    let o = b.uc(&["copy", loose.to_str().unwrap()]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("sync-files on"));

    // Logs and the report hold no file or folder names.
    for x in [&a, &b] {
        let log = x.log();
        for name in [
            "Secret Photos",
            "note-zebra",
            "report-walrus",
            "beach.bin",
            "small-",
        ] {
            assert!(!log.contains(name), "agent {} logged {name}", x.n);
        }
        let r = x.uc_json(&["diagnostics"])["report"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(!r.contains("walrus") && !r.contains("zebra"));
    }
}
