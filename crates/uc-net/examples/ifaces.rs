//! Print this machine's interfaces: the raw records (the fixture format) and the
//! classification. `cargo run -p uc-net --example ifaces -- --raw` prints only the raw JSON,
//! ready to save as `tests/fixtures/ifaces-<os>-<name>.json`.

use uc_net::ifaces::{self, ClassifyOptions, Os};

fn main() {
    let raw = ifaces::raw_interfaces();
    if std::env::args().any(|a| a == "--raw") {
        println!("{}", serde_json::to_string_pretty(&raw).unwrap_or_default());
        return;
    }
    let list = ifaces::classify_all(Os::current(), &raw, &ClassifyOptions::default());
    for i in &list {
        let addrs: Vec<String> = i
            .addrs
            .iter()
            .map(|a| format!("{}/{}", a.ip, a.prefix))
            .collect();
        println!(
            "{:>3} {:<10} {:<9} up={:<5} {} [{}]",
            i.index,
            format!("{:?}", i.kind),
            i.name.chars().take(9).collect::<String>(),
            i.up,
            i.description,
            addrs.join(", ")
        );
    }
    println!("vpn_up={}", ifaces::vpn_up(&list));
}
