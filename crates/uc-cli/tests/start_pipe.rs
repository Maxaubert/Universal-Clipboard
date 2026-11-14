//! `uc` starts the agent when none answers. The started agent must not keep `uc`'s output
//! pipe open, or a script like `uc status | grep Working` would never end. (Found by the
//! Windows install test in CI: Windows passes every inheritable handle to a child.)

mod common;

use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use common::{Agent, agent_exe, until};

#[test]
fn uc_that_starts_the_agent_does_not_hold_its_pipe() {
    let _ = agent_exe(); // build it next to uc
    let mut a = Agent::new(7);
    let env: Vec<(String, String)> = a
        .env()
        .into_iter()
        .filter(|(k, _)| k != "UCLIP_NO_START")
        .collect();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        // output() returns only when every writer of the pipe has closed it.
        let o = Command::new(env!("CARGO_BIN_EXE_uc"))
            .args(["status", "--json"])
            .envs(env)
            .env_remove("UCLIP_NO_START")
            .stdin(Stdio::null())
            .output();
        let _ = tx.send(o);
    });
    let got = rx.recv_timeout(Duration::from_secs(30));
    // The agent that uc started: stop it in every case.
    until(Duration::from_secs(5), || {
        a.try_call("status", serde_json::json!({})).is_ok()
    });
    let _ = a.try_call("debug.shutdown", serde_json::json!({}));
    until(Duration::from_secs(10), || {
        !uc_agent::install::agent_running(&a.profile)
    });
    a.child = None;
    let o = got.expect("uc status did not end: the started agent holds its output pipe");
    let o = o.unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(String::from_utf8_lossy(&o.stdout).contains("\"state\""));
}
