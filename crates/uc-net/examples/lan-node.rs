//! One lab node for the network-namespace tests (plan 6.3, `tests/netns.rs`).
//!
//! A real [`Net`] (real interface detection, `OsBinder`, `mdns-sd`) plus a `uc-core` sync
//! engine with a FAKE clipboard. It never touches a real clipboard.
//!
//! ```text
//! lan-node --key 1 --members 1,2,3 [--port 4500] [--physical lan0] [--peer 2=10.77.0.2:4500]
//!          [--idle-ms 3000] [--path-ms 20000] [--no-mdns]
//! ```
//!
//! Stdin: `copy <text>`, `status`, `quit`. Stdout, one event per line:
//! `READY <port>`, `UP <peer> <remote> <iface>`, `DOWN <peer>`, `PATH <peer> <state>`,
//! `CLIP <text>`, `IFACES vpn=<bool> <names>`, `STATUS ...`. Peers are printed as their key
//! number (the `--key` value of that node).

use std::collections::BTreeMap;
use std::io::BufRead;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use uc_clip::FakeClipboard;
use uc_core::sync::{Config, Engine, Event};
use uc_core::{DeviceId, DeviceKind, DeviceRecord, Role};
use uc_net::discovery::{Discovery, MdnsSd, NoDiscovery};
use uc_net::{ClassifyOptions, LanBook, Net, NetConfig, NetEvent, OsBinder, Secrets};

fn key(n: u8) -> SigningKey {
    SigningKey::from_bytes(&[n; 32])
}

fn id(n: u8) -> DeviceId {
    DeviceId(key(n).verifying_key().to_bytes())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn arg(args: &[String], name: &str) -> Vec<String> {
    args.windows(2)
        .filter(|w| w[0] == name)
        .map(|w| w[1].clone())
        .collect()
}

fn say(s: String) {
    println!("{s}");
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let me: u8 = arg(&args, "--key")
        .first()
        .and_then(|s| s.parse().ok())
        .expect("--key N");
    let members: Vec<u8> = arg(&args, "--members")
        .first()
        .map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect())
        .unwrap_or_default();
    let names: BTreeMap<DeviceId, u8> = (1..=32u8).map(|n| (id(n), n)).collect();
    let name = |d: &DeviceId| names.get(d).map_or("?".to_string(), |n| n.to_string());
    let port: u16 = arg(&args, "--port")
        .first()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let mut cfg = NetConfig::new(key(me), port);
    cfg.classify = ClassifyOptions {
        force_physical: arg(&args, "--physical"),
    };
    if let Some(ms) = arg(&args, "--idle-ms").first().and_then(|s| s.parse().ok()) {
        cfg.timing.idle_timeout = Duration::from_millis(ms);
        cfg.timing.keep_alive = Duration::from_millis(ms / 3);
    }
    if let Some(ms) = arg(&args, "--path-ms").first().and_then(|s| s.parse().ok()) {
        cfg.path_delay = Duration::from_millis(ms);
    }
    cfg.backoff_max = Duration::from_secs(2);
    cfg.iface_poll = Duration::from_millis(500);

    let disc: Arc<dyn Discovery> = if args.iter().any(|a| a == "--no-mdns") {
        Arc::new(NoDiscovery)
    } else {
        Arc::new(MdnsSd::new().expect("mdns"))
    };
    let mut net = Net::start(cfg, Arc::new(OsBinder), disc, LanBook::new()).expect("start");
    net.set_members(
        members
            .iter()
            .filter(|m| **m != me)
            .map(|m| id(*m))
            .collect(),
    );
    net.set_secrets(Secrets {
        current: Some([77; 32]),
        previous: None,
    });
    for p in arg(&args, "--peer") {
        if let Some((n, a)) = p.split_once('=')
            && let (Ok(n), Ok(a)) = (n.parse::<u8>(), a.parse())
        {
            net.add_addr(id(n), a);
        }
    }

    let mut engine = Engine::new(Config::new(
        DeviceRecord {
            id: id(me),
            name: format!("lab {me}"),
            kind: DeviceKind::Linux,
        },
        Role::Full,
    ));
    let mut clip = FakeClipboard::new();

    let (line_tx, mut line_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || {
        for l in std::io::stdin().lock().lines().map_while(Result::ok) {
            if line_tx.send(l).is_err() {
                break;
            }
        }
    });

    if let Some(st) = net.status().await {
        say(format!("READY {}", st.port));
    }
    loop {
        tokio::select! {
            l = line_rx.recv() => {
                let Some(l) = l else { break };
                if let Some(t) = l.strip_prefix("copy ") {
                    clip.copy_text(t);
                    engine.on_local_change(&mut clip, now_ms());
                } else if l == "status" {
                    if let Some(st) = net.status().await {
                        let paths: Vec<String> = st.paths.iter().map(|(d, s)| format!("{}={s:?}", name(d))).collect();
                        let links: Vec<String> = st.links.iter().map(|(d, a, i)| format!("{}@{a}/{i}", name(d))).collect();
                        let socks: Vec<String> = st.sockets.iter().map(|(i, a, t)| format!("{i}:{a}:{t:?}")).collect();
                        say(format!("STATUS port={} vpn={} refused={} paths=[{}] links=[{}] sockets=[{}]",
                            st.port, st.vpn_up, st.refused, paths.join(","), links.join(","), socks.join(",")));
                    }
                } else if l == "quit" {
                    break;
                }
            }
            e = net.next_event() => {
                let Some(e) = e else { break };
                match e {
                    NetEvent::Up(l) => {
                        say(format!("UP {} {} {}", name(&l.peer), l.remote, l.iface));
                        engine.connect(Box::new(l.peer_handle()), now_ms());
                        net.ready(l.peer);
                    }
                    NetEvent::Down { peer } => {
                        say(format!("DOWN {}", name(&peer)));
                        engine.disconnect(peer);
                    }
                    NetEvent::Frame { peer, frame } => {
                        for ev in engine.on_frame(peer, &frame, &mut clip, now_ms()) {
                            match ev {
                                Event::AddrUpdate { peer, signed } => net.addr_update(peer, signed),
                                Event::Applied { .. } => {
                                    say(format!("CLIP {}", clip.text().unwrap_or_default()));
                                }
                                _ => {}
                            }
                        }
                    }
                    NetEvent::Path { peer, state } => say(format!("PATH {} {state:?}", name(&peer))),
                    NetEvent::Pairing(_) => {}
                    NetEvent::PortChanged(p) => say(format!("PORT {p}")),
                    NetEvent::Interfaces { physical, vpn_up } => {
                        let n: Vec<String> = physical.iter().map(|i| i.name.clone()).collect();
                        say(format!("IFACES vpn={vpn_up} {}", n.join(",")));
                    }
                }
            }
        }
    }
    net.shutdown();
    tokio::time::sleep(Duration::from_millis(100)).await;
}
