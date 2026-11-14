//! A test profile must never use the user's Windows clipboard: outside a private window
//! station the agent refuses the OS clipboard.
#![cfg(windows)]

mod common;

use common::*;

#[test]
fn test_profile_refuses_the_user_clipboard_outside_winhost() {
    let mut a = Agent::new(53);
    a.extra_env
        .push(("UCLIP_TEST_CLIPBOARD".into(), "os".into()));
    a.start();
    let s = a.uc_json(&["status"]);
    assert_eq!(s["fix_code"], "desktop", "{s}");
}
