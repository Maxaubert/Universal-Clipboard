//! Network lab rows for file sync (file sync spec 7, plan 6.3): two real agents, each in its
//! own Linux network namespace on a bridge, paired through `uc`, with `tc netem` on the switch
//! ports. Fake clipboards; the files are real, in temp folders.
//!
//! Needs root, `ip`, `pgrep` and `tc`, so every test is `#[ignore]`. Run (as root, Linux only):
//!
//! ```text
//! wsl -d Ubuntu -u root -- bash tools/netsim/run-wsl.sh
//! ```
//!
//! `UC_NETNS_SLEEP_SECS` sets how long the receiver sleeps in row 12 (default 40, longer than
//! the 24 s QUIC idle timeout, so the link really breaks).

#![cfg(target_os = "linux")]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use common::*;
use serde_json::{Value, json};

const SUBNET: &str = "10.77.0";

fn sh(cmd: &str) {
    let out = Command::new("bash")
        .arg("-c")
        .arg(cmd)
        .output()
        .expect("bash");
    assert!(
        out.status.success(),
        "command failed: {cmd}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn sh_out(cmd: &str) -> String {
    let out = Command::new("bash")
        .arg("-c")
        .arg(cmd)
        .output()
        .expect("bash");
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// A LAN of namespaces: a bridge in a "switch" namespace and one namespace per device, linked
/// by a veth pair (`lan0` inside the device). Deleted on drop.
struct Lab {
    tag: String,
    devices: Vec<u8>,
}

impl Lab {
    fn new(tag: &str, devices: &[u8]) -> Lab {
        let lab = Lab {
            tag: tag.to_string(),
            devices: devices.to_vec(),
        };
        lab.cleanup();
        let sw = lab.sw();
        sh(&format!("ip netns add {sw}"));
        sh(&format!(
            "ip -n {sw} link add br0 type bridge && ip -n {sw} link set br0 up"
        ));
        for &d in devices {
            let ns = lab.ns(d);
            sh(&format!("ip netns add {ns}"));
            sh(&format!(
                "ip link add lan0 netns {ns} type veth peer name v{d} netns {sw}"
            ));
            sh(&format!("ip -n {sw} link set v{d} master br0 up"));
            sh(&format!("ip -n {ns} link set lo up"));
            sh(&format!("ip -n {ns} addr add {SUBNET}.{d}/24 dev lan0"));
            sh(&format!("ip -n {ns} link set lan0 up"));
        }
        std::thread::sleep(Duration::from_millis(1500));
        lab
    }

    fn sw(&self) -> String {
        format!("uc{}-sw", self.tag)
    }

    fn ns(&self, d: u8) -> String {
        format!("uc{}-{d}", self.tag)
    }

    /// `tc netem` with `spec` on every switch port (traffic towards each device).
    fn netem(&self, spec: &str) {
        let sw = self.sw();
        for d in &self.devices {
            sh(&format!(
                "ip netns exec {sw} tc qdisc add dev v{d} root netem {spec}"
            ));
        }
    }

    fn cleanup(&self) {
        for d in &self.devices {
            let _ = Command::new("ip")
                .args(["netns", "del", &self.ns(*d)])
                .output();
        }
        let _ = Command::new("ip")
            .args(["netns", "del", &self.sw()])
            .output();
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        self.cleanup();
    }
}

/// An agent for device `d` of the lab: bound to `10.77.0.<d>` inside its namespace.
fn lab_agent(lab: &Lab, d: u8) -> Agent {
    let mut a = Agent::new(d);
    a.ip = format!("{SUBNET}.{d}");
    a.netns = Some(lab.ns(d));
    a.start();
    a
}

/// Alpha (device 1) and Bravo (device 2), paired and connected.
fn alpha_bravo(lab: &Lab) -> (Agent, Agent) {
    let a = lab_agent(lab, 1);
    let mut b = lab_agent(lab, 2);
    a.uc_json(&["settings", "set", "device-name", "Alpha"]);
    b.uc_json(&["settings", "set", "device-name", "Bravo"]);
    pair(&a, &mut b);
    assert!(
        until(Duration::from_secs(30), || {
            a.uc_json(&["status"])["connected"] == 1 && b.uc_json(&["status"])["connected"] == 1
        }),
        "not connected:\n{}",
        a.log()
    );
    (a, b)
}

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

/// Write `mib` MiB that differ along the file, 1 MiB at a time.
fn write_big(p: &Path, mib: usize) {
    use std::io::Write;
    let mut f = std::io::BufWriter::new(std::fs::File::create(p).unwrap());
    for i in 0..mib {
        let chunk: Vec<u8> = (0..1u32 << 20)
            .map(|j| j.wrapping_mul(2654435761).to_le_bytes()[1] ^ i as u8)
            .collect();
        f.write_all(&chunk).unwrap();
    }
    f.flush().unwrap();
}

fn file_hash(p: &Path) -> blake3::Hash {
    let mut h = blake3::Hasher::new();
    h.update_reader(std::fs::File::open(p).unwrap()).unwrap();
    h.finalize()
}

/// `uc copy` of a 200 MiB file on Alpha; wait until it is on Bravo's clipboard, intact.
/// `during` runs once while it is being received. Returns how long it took.
fn copy_200(a: &Agent, b: &Agent, limit: Duration, during: impl FnOnce()) -> Duration {
    let src = tempfile::tempdir().unwrap();
    let big = src.path().join("big.bin");
    write_big(&big, 200);
    let t = Instant::now();
    a.uc_json(&["copy", big.to_str().unwrap()]);
    during();
    assert!(
        until(limit, || files_on(b).len() == 1),
        "200 MiB did not arrive in {limit:?}: {}\nA:\n{}\nB:\n{}",
        b.uc_json(&["status"]),
        a.log(),
        b.log()
    );
    let took = t.elapsed();
    let got = files_on(b).remove(0);
    assert_eq!(std::fs::metadata(&got).unwrap().len(), 200 << 20);
    assert_eq!(file_hash(&got), file_hash(&big));
    assert_eq!(
        b.uc_json(&["status"])["activity"][0]["direction"],
        "received"
    );
    took
}

#[test]
#[ignore = "needs root and network namespaces (plan 6.3)"]
fn row11_files_200mb_over_lossy_wifi() {
    let lab = Lab::new("fa", &[1, 2]);
    // Busy Wi-Fi: 5 ms each way and 0.5% loss each way (no jitter: netem jitter reorders
    // packets far more than Wi-Fi does).
    lab.netem("delay 5ms loss 0.5%");
    let (a, b) = alpha_bravo(&lab);
    let took = copy_200(&a, &b, Duration::from_secs(900), || {});
    eprintln!(
        "200 MiB over lossy Wi-Fi: {took:?} ({:.1} MiB/s)",
        200.0 / took.as_secs_f64()
    );
    assert!(b.uc_json(&["status"])["last_transfer_error"].is_null());
}

#[test]
#[ignore = "needs root and network namespaces (plan 6.3)"]
fn row12_files_sleep_and_wake_mid_transfer() {
    let secs: u64 = std::env::var("UC_NETNS_SLEEP_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(40);
    let lab = Lab::new("fb", &[1, 2]);
    // 80 Mbit/s, so 200 MiB takes about 20 s and the sleep lands in the middle.
    lab.netem("rate 80mbit");
    let (a, b) = alpha_bravo(&lab);
    let pid = b.pid().unwrap();
    let took = copy_200(&a, &b, Duration::from_secs(300), || {
        let mut t = Value::Null;
        assert!(
            until(Duration::from_secs(60), || {
                t = b.uc_json(&["status"])["transfer"].clone();
                t["phase"] == "receiving" && t["done"].as_u64().unwrap_or(0) > 40 << 20
            }),
            "no transfer on Bravo: {t}"
        );
        // `ip netns exec` may fork: stop the agent itself (the child of `ip`, if any).
        let kids = sh_out(&format!("pgrep -P {pid} || echo {pid}"));
        let target = kids.split_whitespace().next().unwrap_or("").to_string();
        eprintln!(
            "Bravo sleeps for {secs} s at {} MiB",
            t["done"].as_u64().unwrap_or(0) >> 20
        );
        sh(&format!("kill -STOP {target}"));
        std::thread::sleep(Duration::from_secs(secs));
        sh(&format!("kill -CONT {target}"));
    });
    eprintln!("200 MiB with a {secs} s sleep in the middle: {took:?}");
    // The link breaks (wake seen, or the idle timeout) and Bravo asks again from the start.
    let asks = b.log().matches("asking for files").count();
    eprintln!("Bravo asked for the files {asks} times");
    if secs > 30 {
        assert!(
            asks >= 2,
            "the link must break after {secs} s:\n{}",
            b.log()
        );
    }
}
