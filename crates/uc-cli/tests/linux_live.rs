//! The Linux clipboard path end to end (plan 6.4): two agents on the real Linux backends,
//! one on X11 (Xvfb) and one on Wayland (headless Sway), each with its own clipboard. They
//! pair through the CLI and run the copy matrix. Runs only where `live/run.sh` started the
//! display servers (`UCLIP_LIVE_X11`, `UCLIP_LIVE_WAYLAND`): Docker or CI, never a desktop.
#![cfg(target_os = "linux")]

mod common;

use common::*;

#[test]
fn x11_agent_and_wayland_agent_sync() {
    let (Ok(x), Ok(wl)) = (
        std::env::var("UCLIP_LIVE_X11"),
        std::env::var("UCLIP_LIVE_WAYLAND"),
    ) else {
        eprintln!("skipped: UCLIP_LIVE_X11 and UCLIP_LIVE_WAYLAND not set");
        return;
    };
    let mut a = Agent::new(71);
    a.extra_env
        .push(("UCLIP_TEST_CLIPBOARD".into(), "os".into()));
    a.extra_env.push(("DISPLAY".into(), x));
    a.extra_env.push(("XDG_SESSION_TYPE".into(), "x11".into()));
    a.remove_env.push("WAYLAND_DISPLAY".into());
    a.start();
    let mut b = Agent::new(72);
    b.extra_env
        .push(("UCLIP_TEST_CLIPBOARD".into(), "os".into()));
    b.extra_env.push(("WAYLAND_DISPLAY".into(), wl));
    b.extra_env
        .push(("XDG_SESSION_TYPE".into(), "wayland".into()));
    b.remove_env.push("DISPLAY".into());
    b.start();
    let ra = a.uc_json(&["diagnostics"])["report"]
        .as_str()
        .unwrap()
        .to_string();
    let rb = b.uc_json(&["diagnostics"])["report"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(ra.contains("Clipboard: linux x11"), "{ra}");
    assert!(rb.contains("data-control"), "{rb}");
    pair(&a, &mut b);
    copy_matrix(&[&a, &b], "linux");
}
