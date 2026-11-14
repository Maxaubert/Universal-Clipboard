//! Plan 6.3: simulated networks in Linux network namespaces (WSL on the PC, or `lin-kde`).
//!
//! Needs root, `ip`, `bridge`, `nft`, `wg` and `tc` (netem), so every test is `#[ignore]`.
//! Run (as root, Linux only):
//!
//! ```text
//! cargo build -p uc-net --examples
//! cargo test -p uc-net --test netns -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! Each test builds its own LAN: a bridge in a "switch" namespace and one namespace per device,
//! linked by a veth pair (`lan0` inside the device). The devices run the `lan-node` example: a
//! real `Net` with `OsBinder` (SO_BINDTODEVICE), real `mdns-sd`, and a sync engine with a fake
//! clipboard. `lan0` is forced to count as physical (a veth is virtual by the classifier).

#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

fn node_bin() -> PathBuf {
    let exe = std::env::current_exe().expect("exe");
    let p = exe
        .parent()
        .and_then(|d| d.parent())
        .map(|d| d.join("examples").join("lan-node"))
        .expect("path");
    assert!(
        p.exists(),
        "{} missing: run `cargo build -p uc-net --examples` first",
        p.display()
    );
    p
}

/// A LAN of namespaces. Deleted on drop.
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
        // Let IPv6 DAD and the bridge settle.
        std::thread::sleep(Duration::from_millis(1500));
        lab
    }

    fn sw(&self) -> String {
        format!("uc{}-sw", self.tag)
    }

    fn ns(&self, d: u8) -> String {
        format!("uc{}-{d}", self.tag)
    }

    fn exec(&self, d: u8, cmd: &str) {
        sh(&format!("ip netns exec {} {cmd}", self.ns(d)));
    }

    /// An nftables table `t` with an output chain `o` and an input chain `i`.
    fn nft(&self, d: u8, t: &str, out: &[&str], inp: &[&str]) {
        self.exec(d, &format!("nft add table inet {t}"));
        self.exec(
            d,
            &format!("nft add chain inet {t} o '{{ type filter hook output priority 0; }}'"),
        );
        self.exec(
            d,
            &format!("nft add chain inet {t} i '{{ type filter hook input priority 0; }}'"),
        );
        for r in out {
            self.exec(d, &format!("nft add rule inet {t} o '{r}'"));
        }
        for r in inp {
            self.exec(d, &format!("nft add rule inet {t} i '{r}'"));
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

    /// A WireGuard interface `wg0` in device `d` with address `10.99.0.<d>/32`. The peer
    /// endpoint does not exist, so nothing goes through the tunnel (like a VPN whose server is
    /// unreachable or a tunnel that only carries other traffic).
    fn wireguard(&self, d: u8) {
        let ns = self.ns(d);
        sh(&format!("ip -n {ns} link add wg0 type wireguard"));
        sh(&format!(
            "ip netns exec {ns} bash -c 'wg genkey > /tmp/{ns}.key && wg set wg0 private-key /tmp/{ns}.key peer $(wg genkey | wg pubkey) allowed-ips 0.0.0.0/0 endpoint 192.0.2.1:51820'"
        ));
        sh(&format!("ip -n {ns} addr add 10.99.0.{d}/32 dev wg0"));
        sh(&format!("ip -n {ns} link set wg0 up"));
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        self.cleanup();
    }
}

/// A running lan-node.
struct Node {
    child: Child,
    stdin: ChildStdin,
    lines: Arc<Mutex<Vec<(Instant, String)>>>,
}

impl Node {
    fn start(lab: &Lab, d: u8, members: &[u8], extra: &[String]) -> Node {
        let members: Vec<String> = members.iter().map(u8::to_string).collect();
        let mut cmd = Command::new("ip");
        cmd.args(["netns", "exec", &lab.ns(d)])
            .arg(node_bin())
            .args(["--key", &d.to_string()])
            .args(["--members", &members.join(",")])
            .args(["--physical", "lan0"])
            .args(["--port", "4500"])
            .args(extra)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = cmd.spawn().expect("spawn lan-node");
        let stdin = child.stdin.take().expect("stdin");
        let out = child.stdout.take().expect("stdout");
        let lines = Arc::new(Mutex::new(Vec::new()));
        let l2 = lines.clone();
        std::thread::spawn(move || {
            for l in BufReader::new(out).lines().map_while(Result::ok) {
                eprintln!("[{d}] {l}");
                l2.lock().unwrap().push((Instant::now(), l));
            }
        });
        Node {
            child,
            stdin,
            lines,
        }
    }

    fn send(&mut self, cmd: &str) {
        let _ = writeln!(self.stdin, "{cmd}");
        let _ = self.stdin.flush();
    }

    fn wait(&self, limit: Duration, pred: impl Fn(&str) -> bool) -> Option<Instant> {
        let start = Instant::now();
        loop {
            if let Some((t, _)) = self.lines.lock().unwrap().iter().find(|(_, l)| pred(l)) {
                return Some(*t);
            }
            if start.elapsed() > limit {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Wait for a line that appears after `after`.
    fn wait_after(
        &self,
        after: Instant,
        limit: Duration,
        pred: impl Fn(&str) -> bool,
    ) -> Option<Instant> {
        let start = Instant::now();
        loop {
            if let Some((t, _)) = self
                .lines
                .lock()
                .unwrap()
                .iter()
                .find(|(t, l)| *t >= after && pred(l))
            {
                return Some(*t);
            }
            if start.elapsed() > limit {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn status(&mut self) -> String {
        let t = Instant::now();
        self.send("status");
        self.wait_after(t, Duration::from_secs(3), |l| l.starts_with("STATUS"))
            .and_then(|t| {
                self.lines
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|(x, l)| *x == t && l.starts_with("STATUS"))
                    .map(|(_, l)| l.clone())
            })
            .unwrap_or_default()
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn up_to(peer: u8) -> impl Fn(&str) -> bool {
    move |l: &str| l.starts_with(&format!("UP {peer} "))
}

fn peer_arg(d: u8) -> Vec<String> {
    vec!["--peer".into(), format!("{d}={SUBNET}.{d}:4500")]
}

const T: Duration = Duration::from_secs(15);

#[test]
#[ignore = "needs root and network namespaces (plan 6.3)"]
fn row01_same_lan_three_nodes_by_mdns() {
    let lab = Lab::new("r1", &[1, 2, 3]);
    let t0 = Instant::now();
    let n: Vec<Node> = [1, 2, 3]
        .iter()
        .map(|&d| Node::start(&lab, d, &[1, 2, 3], &[]))
        .collect();
    for (i, a) in [1u8, 2, 3].iter().enumerate() {
        for b in [1u8, 2, 3] {
            if *a != b {
                let t = n[i].wait(T, up_to(b)).expect("mesh");
                eprintln!("{a}->{b} up after {:?}", t - t0);
            }
        }
    }
    let first_up = n[0].wait(T, up_to(2)).unwrap();
    let ready = n[0].wait(T, |l| l.starts_with("READY")).unwrap();
    eprintln!("discovery+connect after ready: {:?}", first_up - ready);
    // Direct LAN path, on lan0.
    for x in &n {
        assert!(
            x.wait(T, |l| l.starts_with("UP")
                && l.contains(SUBNET)
                && l.ends_with("lan0"))
                .is_some()
        );
    }
}

#[test]
#[ignore = "needs root and network namespaces (plan 6.3)"]
fn row02_multicast_blocked() {
    let lab = Lab::new("r2", &[1, 2, 3]);
    for d in [1, 2, 3] {
        lab.nft(
            d,
            "mc",
            &[
                "ip daddr 224.0.0.0/4 drop",
                "ip6 daddr ff02::/16 udp dport 5353 drop",
            ],
            &[
                "ip daddr 224.0.0.0/4 drop",
                "ip6 daddr ff02::/16 udp dport 5353 drop",
            ],
        );
    }
    // 1 remembers 2's address. Nobody knows 3.
    let n1 = Node::start(
        &lab,
        1,
        &[1, 2, 3],
        &[peer_arg(2), vec!["--path-ms".into(), "3000".into()]].concat(),
    );
    let n2 = Node::start(&lab, 2, &[1, 2, 3], &["--path-ms".into(), "3000".into()]);
    let n3 = Node::start(&lab, 3, &[1, 2, 3], &["--path-ms".into(), "3000".into()]);
    n1.wait(T, up_to(2)).expect("found by remembered address");
    n2.wait(T, up_to(1)).expect("found by remembered address");
    n1.wait(T, |l| l == "PATH 3 NoPath(NotFound)")
        .expect("3 never seen: no path");
    n3.wait(T, |l| l == "PATH 1 NoPath(NotFound)")
        .expect("3 finds nobody");
    assert!(
        n3.wait(Duration::from_secs(2), |l| l.starts_with("UP"))
            .is_none()
    );
}

#[test]
#[ignore = "needs root and network namespaces (plan 6.3)"]
fn row03_vpn_full_tunnel_lan_allowed() {
    let lab = Lab::new("r3", &[1, 2]);
    lab.wireguard(1);
    // Full tunnel: default route into wg0. The LAN /24 route stays (LAN allowed).
    lab.exec(1, "ip route add default dev wg0");
    let n1 = Node::start(&lab, 1, &[1, 2], &[]);
    let n2 = Node::start(&lab, 2, &[1, 2], &[]);
    n1.wait(T, |l| l.starts_with("IFACES vpn=true"))
        .expect("VPN seen");
    let up = n1.wait(T, up_to(2)).expect("direct LAN path");
    let _ = up;
    assert!(
        n1.wait(T, |l| l.starts_with("UP 2 10.77.0.2:4500 lan0"))
            .is_some(),
        "via lan0, not wg0"
    );
    assert!(n2.wait(T, up_to(1)).is_some());
}

#[test]
#[ignore = "needs root and network namespaces (plan 6.3)"]
fn row04_vpn_overlapping_route() {
    let lab = Lab::new("r4", &[1, 2]);
    lab.wireguard(1);
    // Two /25 routes into the tunnel beat the on-link /24: the route table sends LAN unicast
    // into wg0.
    lab.exec(1, "ip route add 10.77.0.0/25 dev wg0");
    lab.exec(1, "ip route add 10.77.0.128/25 dev wg0");
    let route = sh_out(&format!(
        "ip netns exec {} ip route get 10.77.0.2",
        lab.ns(1)
    ));
    assert!(route.contains("dev wg0"), "the overlap is real: {route}");
    let mut n1 = Node::start(&lab, 1, &[1, 2], &peer_arg(2));
    let n2 = Node::start(&lab, 2, &[1, 2], &[]);
    // SO_BINDTODEVICE(lan0) wins over the route table on Linux: direct LAN path.
    n1.wait(T, |l| l.starts_with("UP 2 10.77.0.2:4500 lan0"))
        .expect("direct LAN path despite the overlapping route");
    n2.wait(T, up_to(1)).expect("2 sees 1");
    let st = n1.status();
    assert!(
        st.contains("lan0:10.77.0.1:4500:Device"),
        "socket tied to lan0: {st}"
    );
}

#[test]
#[ignore = "needs root and network namespaces (plan 6.3)"]
fn row05_vpn_kill_switch() {
    let lab = Lab::new("r5", &[1, 2]);
    lab.wireguard(1);
    let fast = vec!["--idle-ms".to_string(), "3000".into()];
    let n1 = Node::start(&lab, 1, &[1, 2], &fast);
    let n2 = Node::start(&lab, 2, &[1, 2], &fast);
    n1.wait(T, up_to(2)).expect("up before the kill switch");
    n2.wait(T, up_to(1)).expect("up");
    std::thread::sleep(Duration::from_millis(500));
    // Kill switch: drop everything that is not loopback or the tunnel.
    lab.nft(
        1,
        "ks",
        &["oifname != { lo, wg0 } drop"],
        &["iifname != { lo, wg0 } drop"],
    );
    let t = Instant::now();
    n1.wait_after(t, Duration::from_secs(40), |l| {
        l == "PATH 2 NoPath(VpnBlocksLan)"
    })
    .expect("1: blocked by VPN (needs attention)");
    // 2 has no VPN, but 1 announced one: same help on 2.
    n2.wait_after(t, Duration::from_secs(40), |l| {
        l == "PATH 1 NoPath(VpnBlocksLan)"
    })
    .expect("2: blocked by VPN (from 1's announcement)");
}

#[test]
#[ignore = "needs root and network namespaces (plan 6.3)"]
fn row06_client_isolation() {
    let lab = Lab::new("r6", &[1, 2]);
    let fast = vec!["--idle-ms".to_string(), "3000".into()];
    let n1 = Node::start(&lab, 1, &[1, 2], &fast);
    let n2 = Node::start(&lab, 2, &[1, 2], &fast);
    n1.wait(T, up_to(2)).expect("up");
    n2.wait(T, up_to(1)).expect("up");
    let sw = lab.sw();
    sh(&format!("bridge -n {sw} link set dev v1 isolated on"));
    sh(&format!("bridge -n {sw} link set dev v2 isolated on"));
    let t = Instant::now();
    n1.wait_after(t, Duration::from_secs(40), |l| {
        l == "PATH 2 NoPath(Unreachable)"
    })
    .expect("no path (guest network help), no VPN false alarm");
    n2.wait_after(t, Duration::from_secs(40), |l| {
        l == "PATH 1 NoPath(Unreachable)"
    })
    .expect("no path on 2");
    assert!(
        n1.wait(Duration::from_millis(1), |l| l.contains("VpnBlocksLan"))
            .is_none()
    );
}

#[test]
#[ignore = "needs root and network namespaces (plan 6.3)"]
fn row07_both_sides_inbound_blocked_then_one_opens() {
    let lab = Lab::new("r7", &[1, 2]);
    for d in [1, 2] {
        lab.nft(d, "ib", &[], &["udp dport 4500 drop"]);
    }
    let a = [peer_arg(2), vec!["--path-ms".into(), "3000".into()]].concat();
    let b = [peer_arg(1), vec!["--path-ms".into(), "3000".into()]].concat();
    let mut n1 = Node::start(&lab, 1, &[1, 2], &a);
    let mut n2 = Node::start(&lab, 2, &[1, 2], &b);
    n1.wait(T, |l| l.starts_with("PATH 2 NoPath"))
        .expect("no path");
    n2.wait(T, |l| l.starts_with("PATH 1 NoPath"))
        .expect("no path");
    assert!(
        !n1.status().is_empty() && !n2.status().is_empty(),
        "no crash"
    );
    // 2 opens fully; 1 keeps blocking unsolicited inbound but lets replies in.
    lab.exec(2, "nft delete table inet ib");
    lab.exec(1, "nft flush chain inet ib i");
    lab.exec(
        1,
        "nft add rule inet ib i 'udp dport 4500 ct state new drop'",
    );
    n1.wait(Duration::from_secs(20), up_to(2)).expect("healed");
}

#[test]
#[ignore = "needs root and network namespaces (plan 6.3)"]
fn row08_address_change() {
    let lab = Lab::new("r8", &[1, 2, 3]);
    let n1 = Node::start(&lab, 1, &[1, 2, 3], &[]);
    let n2 = Node::start(&lab, 2, &[1, 2, 3], &[]);
    let n3 = Node::start(&lab, 3, &[1, 2, 3], &[]);
    for (n, p) in [(&n1, 2), (&n1, 3), (&n2, 3)] {
        n.wait(T, up_to(p)).expect("mesh");
    }
    std::thread::sleep(Duration::from_millis(500));
    let t = Instant::now();
    lab.exec(2, &format!("ip addr add {SUBNET}.12/24 dev lan0"));
    lab.exec(2, &format!("ip addr del {SUBNET}.2/24 dev lan0"));
    let a = n1
        .wait_after(t, Duration::from_secs(10), |l| {
            l.starts_with("UP 2 10.77.0.12:")
        })
        .expect("1 reconnected to 2's new address");
    let b = n3
        .wait_after(t, Duration::from_secs(10), |l| {
            l.starts_with("UP 2 10.77.0.12:")
        })
        .expect("3 reconnected to 2's new address");
    let worst = (a - t).max(b - t);
    eprintln!("address change: reconnect after {worst:?}");
    assert!(
        worst < Duration::from_secs(5),
        "plan: under 5 s, took {worst:?}"
    );
}

#[test]
#[ignore = "needs root and network namespaces (plan 6.3)"]
fn row09_sleep_and_wake() {
    let secs: u64 = std::env::var("UC_NETNS_SLEEP_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    let lab = Lab::new("r9", &[1, 2]);
    let mut n1 = Node::start(&lab, 1, &[1, 2], &[]);
    let n2 = Node::start(&lab, 2, &[1, 2], &[]);
    n1.wait(T, up_to(2)).expect("up");
    n2.wait(T, up_to(1)).expect("up");
    std::thread::sleep(Duration::from_millis(500));
    // `ip netns exec` forks the node: stop the real process (the child of `ip`).
    let pid = n2.pid();
    let kids = sh_out(&format!("pgrep -P {pid} || echo {pid}"));
    let target = kids.split_whitespace().next().unwrap_or("").to_string();
    sh(&format!("kill -STOP {target}"));
    std::thread::sleep(Duration::from_secs(2));
    n1.send("copy while you slept");
    std::thread::sleep(Duration::from_secs(secs));
    let t = Instant::now();
    sh(&format!("kill -CONT {target}"));
    n2.wait_after(t, Duration::from_secs(20), |l| l == "CLIP while you slept")
        .expect("newest clip arrives after wake");
    n1.wait_after(t, Duration::from_secs(20), up_to(2))
        .expect("reconnected after wake");
    eprintln!("slept {secs} s; healed after {:?}", t.elapsed());
}

#[test]
#[ignore = "needs root and network namespaces (plan 6.3)"]
fn row10_lossy_wifi() {
    let lab = Lab::new("ra", &[1, 2, 3]);
    let sw = lab.sw();
    for d in [1, 2, 3] {
        sh(&format!(
            "ip netns exec {sw} tc qdisc add dev v{d} root netem loss 5% delay 100ms"
        ));
    }
    let mut n1 = Node::start(&lab, 1, &[1, 2, 3], &[]);
    let n2 = Node::start(&lab, 2, &[1, 2, 3], &[]);
    let n3 = Node::start(&lab, 3, &[1, 2, 3], &[]);
    for (n, p) in [(&n1, 2), (&n1, 3), (&n2, 3)] {
        n.wait(Duration::from_secs(30), up_to(p))
            .expect("mesh over lossy links");
    }
    std::thread::sleep(Duration::from_secs(1));
    for i in 0..20 {
        n1.send(&format!("copy lossy {i}"));
        std::thread::sleep(Duration::from_millis(150));
    }
    n2.wait(Duration::from_secs(30), |l| l == "CLIP lossy 19")
        .expect("last copy on 2");
    n3.wait(Duration::from_secs(30), |l| l == "CLIP lossy 19")
        .expect("last copy on 3");
}

#[test]
#[ignore = "needs root and network namespaces"]
fn record_netns_fixture_with_wireguard() {
    // Writes tests/fixtures/ifaces-linux-netns-wg.json (used by tests/fixtures.rs).
    let lab = Lab::new("fx", &[1]);
    lab.wireguard(1);
    lab.exec(
        1,
        "ip link add br-x type bridge && ip link add dum0 type dummy",
    );
    let bin = node_bin().with_file_name("ifaces");
    let json = sh_out(&format!(
        "ip netns exec {} {} --raw",
        lab.ns(1),
        bin.display()
    ));
    assert!(json.contains("wg0"));
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ifaces-linux-netns-wg.json");
    std::fs::write(out, json).expect("write fixture");
}
