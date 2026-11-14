//! The macOS clipboard path end to end (plan 6.4): one agent uses the real macOS backend on
//! a **named** pasteboard (`UCLIP_TEST_PASTEBOARD`), never the general one, so the user's
//! clipboard is never touched and no prompt shows. It pairs with a second agent (fake
//! clipboard) through the CLI and runs the copy matrix both ways.
#![cfg(target_os = "macos")]

mod common;

use common::*;

#[test]
fn real_macos_backend_on_a_named_pasteboard() {
    let pb = format!(
        "io.github.maxaubert.uclip.test.agent.{}",
        std::process::id()
    );
    let mut a = Agent::new(61);
    a.extra_env
        .push(("UCLIP_TEST_CLIPBOARD".into(), "os".into()));
    a.extra_env.push(("UCLIP_TEST_PASTEBOARD".into(), pb));
    a.start();
    let r = a.uc_json(&["diagnostics"]);
    assert!(
        r["report"]
            .as_str()
            .unwrap()
            .contains("named test pasteboard"),
        "{r}"
    );
    let mut b = Agent::new(62);
    b.start();
    pair(&a, &mut b);
    copy_matrix(&[&a, &b], "mac");
}

/// A test profile must never open the general pasteboard: without a named pasteboard the
/// agent refuses the OS clipboard.
#[test]
fn test_profile_refuses_the_general_pasteboard() {
    let mut a = Agent::new(63);
    a.extra_env
        .push(("UCLIP_TEST_CLIPBOARD".into(), "os".into()));
    a.start();
    let s = a.uc_json(&["status"]);
    assert_eq!(s["fix_code"], "desktop", "{s}");
}
