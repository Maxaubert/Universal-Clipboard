//! `uc-agent --supervise` (plan 3.2, Linux autostart): the agent is started again after a
//! crash, and the supervisor stops when the agent exits cleanly.
#![cfg(unix)]

mod common;

use std::process::{Command, Stdio};
use std::time::Duration;

use common::*;
use serde_json::json;

#[test]
fn supervisor_restarts_a_crashed_agent() {
    let a = Agent::new(81);
    let mut sup = Command::new(agent_exe())
        .arg("--supervise")
        .envs(a.env())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    a.wait_ready();
    let pid1 = a.call("status", json!({}))["pid"].as_u64().unwrap();
    // Crash it.
    Command::new("kill")
        .args(["-9", &pid1.to_string()])
        .status()
        .unwrap();
    let restarted = until(Duration::from_secs(15), || {
        a.try_call("status", json!({}))
            .ok()
            .and_then(|v| v["pid"].as_u64())
            .is_some_and(|p| p != pid1)
    });
    assert!(restarted, "the supervisor did not start the agent again");
    // A clean stop ends the supervisor too.
    a.call("debug.shutdown", json!({}));
    let ended = until(Duration::from_secs(10), || {
        matches!(sup.try_wait(), Ok(Some(_)))
    });
    if !ended {
        let _ = sup.kill();
    }
    assert!(ended, "the supervisor must stop after a clean exit");
}
