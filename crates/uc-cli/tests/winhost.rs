//! The Windows clipboard path end to end (plan 6.4): one agent runs inside a private window
//! station made by `tools/winhost`, so it uses the real Windows clipboard backend on a
//! clipboard that is not the user's, and nothing appears on screen. It pairs with a second
//! agent (fake clipboard) through the CLI and runs the copy matrix both ways.
#![cfg(windows)]

mod common;

use std::ffi::OsString;
use std::time::Duration;

use common::*;

#[test]
fn real_windows_clipboard_in_a_private_station() {
    let mut a = Agent::new(51);
    a.extra_env
        .push(("UCLIP_TEST_CLIPBOARD".into(), "os".into()));
    let env: Vec<(OsString, OsString)> = a
        .env()
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
    let exe = agent_exe();
    let host = std::thread::spawn(move || {
        uc_winhost::run(&exe, &[], &env, Duration::from_secs(120)).expect("winhost")
    });
    a.wait_ready();
    let st = a.uc_json(&["diagnostics"]);
    assert!(
        st["report"]
            .as_str()
            .unwrap()
            .contains("Clipboard: windows"),
        "the agent in winhost must use the Windows backend: {st}"
    );
    let mut b = Agent::new(52);
    b.start();
    pair(&a, &mut b);
    copy_matrix(&[&a, &b], "winhost");
    // Our own write carries the marker: it is not sent back (echo layers 1 and 2).
    std::thread::sleep(Duration::from_millis(500));
    let log = a.log();
    assert!(!log.contains("panicked"), "{log}");
    a.stop();
    let out = host.join().unwrap();
    assert!(!out.timed_out, "agent in winhost did not stop");
    assert!(out.station.starts_with("Service-0x0-"), "{}", out.station);
}
