//! A real, headless mDNS check on this machine: start the real `mdns-sd` daemon the way the
//! agent does ([`MdnsSd`] with the usable interfaces), browse `_uclip-pair._udp` and
//! `_uclip._udp` for a few seconds, and print what it finds and which interfaces the daemon
//! turned on. It only listens and asks: it never advertises and never dials.
//!
//! ```text
//! cargo run -p uc-net --example mdns-check [-- --secs 5]
//! ```

use std::time::{Duration, Instant};

use uc_net::discovery::{Discovery, MdnsSd, mdns_indexes};
use uc_net::ifaces::{self, ClassifyOptions};

fn main() {
    let secs: u64 = std::env::args()
        .skip_while(|a| a != "--secs")
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);

    let list = ifaces::detect(&ClassifyOptions::default());
    let usable: Vec<_> = list.iter().filter(|i| i.usable()).cloned().collect();
    println!("usable interfaces (ours, netdev):");
    for i in &usable {
        let addrs: Vec<String> = i.addrs.iter().map(|a| a.ip.to_string()).collect();
        println!(
            "  index={} name={} ({}) [{}]",
            i.index,
            i.name,
            i.short_name(),
            addrs.join(", ")
        );
    }
    println!("mDNS indexes: {:?}", mdns_indexes(&usable));

    println!("the same addresses as mdns-sd sees them (if-addrs):");
    let theirs = if_addrs::get_if_addrs().unwrap_or_default();
    for i in &usable {
        for a in &i.addrs {
            match theirs.iter().find(|t| t.ip() == a.ip) {
                Some(t) => println!(
                    "  {} -> mdns-sd name={:?} index={:?} (ours: name={} index={}) name match={} index match={}",
                    a.ip,
                    t.name,
                    t.index,
                    i.name,
                    i.index,
                    t.name == i.name,
                    t.index == Some(i.index)
                ),
                None => println!("  {} -> not seen by if-addrs", a.ip),
            }
        }
    }

    let d = match MdnsSd::new() {
        Ok(d) => d,
        Err(e) => {
            println!("daemon failed: {e}");
            std::process::exit(1);
        }
    };
    let monitor = d.daemon().monitor().ok();
    if let Err(e) = d.set_interfaces(&usable) {
        println!("set_interfaces failed: {e}");
        std::process::exit(1);
    }

    let end = Instant::now() + Duration::from_secs(secs);
    let mut pair = Vec::new();
    let mut sync = Vec::new();
    let mut enabled = Vec::new();
    while Instant::now() < end {
        for f in d.poll_pair() {
            if !pair.contains(&f) {
                pair.push(f);
            }
        }
        for f in d.poll() {
            if !sync.contains(&f) {
                sync.push(f);
            }
        }
        if let Some(m) = &monitor {
            while let Ok(ev) = m.try_recv() {
                match ev {
                    mdns_sd::DaemonEvent::IpAdd(ip) => enabled.push(ip.to_string()),
                    mdns_sd::DaemonEvent::IpDel(ip) => enabled.retain(|x| x != &ip.to_string()),
                    _ => {}
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    println!("mdns-sd enabled addresses: [{}]", enabled.join(", "));
    println!("found _uclip-pair._udp: {}", pair.len());
    for f in &pair {
        println!("  {} port={} addrs={:?}", f.instance, f.port, f.addrs);
    }
    println!("found _uclip._udp: {}", sync.len());
    for f in &sync {
        println!("  {} port={} addrs={:?}", f.instance, f.port, f.addrs);
    }
}
