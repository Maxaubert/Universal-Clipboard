//! The interfaces we pick for mDNS must be the ones `mdns-sd` turns on. `mdns-sd` lists
//! interfaces with `if-addrs`, and we select them by OS index (`discovery::mdns_if_kinds`).
//! This checks, on the machine that runs the test (CI: Windows, macOS, Linux), that every
//! address of a usable interface has the same index in `if-addrs` as in our scan.
//! Before 2026-11-14 we selected by name, and on Windows the names never matched.

use uc_net::ifaces::{self, ClassifyOptions};

#[test]
fn usable_interfaces_have_the_same_index_in_mdns_sd() {
    let ours = ifaces::detect(&ClassifyOptions::default());
    let theirs = if_addrs::get_if_addrs().expect("if-addrs lists interfaces");
    for i in ours.iter().filter(|i| i.usable()) {
        for a in &i.addrs {
            let Some(t) = theirs.iter().find(|t| t.ip() == a.ip) else {
                // An address that came up between the two scans; nothing to compare.
                continue;
            };
            assert_eq!(
                t.index,
                Some(i.index),
                "{} on {} ({}): if-addrs says {:?} index {:?}",
                a.ip,
                i.name,
                i.index,
                t.name,
                t.index
            );
        }
    }
}
