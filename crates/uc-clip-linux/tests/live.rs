//! Live tests against a real, headless display server.
//!
//! They run only when the test script sets `UCLIP_LIVE_X11` (an X display, for example
//! `:99` from Xvfb) or `UCLIP_LIVE_WAYLAND` (a Wayland socket name, for example from
//! `sway` with `WLR_BACKENDS=headless`). The script is `crates/uc-clip-linux/live/run.sh`,
//! and it runs in Docker or in CI. Without those variables every test here passes at once,
//! so `cargo test` on a desktop never touches the user's real clipboard.
//!
//! Each backend has one test that runs all cases in order, because the cases share one
//! clipboard.
#![cfg(target_os = "linux")]

use std::process::{Command, Stdio};
use std::sync::mpsc::Receiver;
use std::time::Duration;

use uc_clip::{Clipboard, Format, Snapshot};
use uc_clip_linux::{BackendKind, DataControl, LinuxClipboard, mime};

const WAIT: Duration = Duration::from_secs(3);

/// The value of a live-test variable, or `None` (skip). Never runs inside WSLg, where the
/// display is bridged to the Windows clipboard.
fn live(var: &str) -> Option<String> {
    let v = std::env::var(var).ok().filter(|v| !v.is_empty())?;
    if std::path::Path::new("/mnt/wslg").exists() {
        eprintln!("skip: WSLg bridges to the Windows clipboard");
        return None;
    }
    Some(v)
}

fn have(tool: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {tool}")])
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Wait for the next change event, then read. This is what the agent does.
fn read_after_event(c: &mut LinuxClipboard, rx: &Receiver<u64>) -> Snapshot {
    rx.recv_timeout(WAIT).expect("change event");
    // Drop extra events, as the settle time does.
    while rx.recv_timeout(Duration::from_millis(100)).is_ok() {}
    c.read().expect("read")
}

fn png_like(len: usize) -> Vec<u8> {
    // The backend does not decode images; any bytes with the PNG type are carried.
    let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
    v.extend((0..len).map(|i| (i * 31 % 251) as u8));
    v
}

/// All cases, for a pair of clipboards on the same display: `a` is the agent, `b` plays
/// "another app".
fn suite(mut a: LinuxClipboard, mut b: LinuxClipboard) {
    let rx = a.subscribe();
    let rb = b.subscribe();

    // 1. Another app copies text, HTML and PNG: we see an event and read all three.
    let png = png_like(4096);
    b.copy_raw(&[
        ("UTF8_STRING", b"hello"),
        ("text/plain;charset=utf-8", b"hello"),
        ("text/html", b"<b>hello</b>"),
        ("image/png", &png),
    ])
    .unwrap();
    let s = read_after_event(&mut a, &rx);
    assert!(!s.has_marker && !s.private && !s.has_files, "{s:?}");
    assert_eq!(s.parts.len(), 3, "{:?}", a.offered());
    assert!(s.parts.contains(&(Format::TextPlain, b"hello".to_vec())));
    assert!(
        s.parts
            .contains(&(Format::TextHtml, b"<b>hello</b>".to_vec()))
    );
    assert!(s.parts.contains(&(Format::ImagePng, png.clone())));

    // 2. Our own write: the other side sees text and our marker; we skip it by seq and marker.
    while rb.try_recv().is_ok() {}
    let seq = a
        .write(&[(Format::TextPlain, "grüß".as_bytes().to_vec())])
        .unwrap();
    while rx.try_recv().is_ok() {}
    rb.recv_timeout(WAIT).expect("b sees our write");
    let own = a.read().unwrap();
    assert_eq!(own.seq, seq);
    assert!(own.has_marker);
    let offered = b.offered().unwrap();
    assert!(offered.iter().any(|m| m == mime::MARKER), "{offered:?}");
    // A plain app that reads the text gets it.
    assert_eq!(
        b.read_type("UTF8_STRING").unwrap().as_deref(),
        Some("grüß".as_bytes())
    );

    // 3. Private copy (KeePassXC style): flagged, content not read.
    b.copy_raw(&[
        ("UTF8_STRING", b"hunter2"),
        (mime::KDE_PASSWORD_HINT, b"secret"),
    ])
    .unwrap();
    let s = read_after_event(&mut a, &rx);
    assert!(s.private && s.parts.is_empty(), "{s:?}");

    // 4. A hint with another value is a normal copy.
    b.copy_raw(&[
        ("UTF8_STRING", b"plain"),
        (mime::KDE_PASSWORD_HINT, b"accept"),
    ])
    .unwrap();
    let s = read_after_event(&mut a, &rx);
    assert!(!s.private);
    assert_eq!(s.parts, vec![(Format::TextPlain, b"plain".to_vec())]);

    // 5. File copy: skipped. A web link in a uri-list is not a file.
    b.copy_raw(&[
        ("text/uri-list", b"file:///home/u/report.pdf\r\n"),
        ("UTF8_STRING", b"report.pdf"),
    ])
    .unwrap();
    assert!(read_after_event(&mut a, &rx).has_files);
    b.copy_raw(&[
        ("text/uri-list", b"https://example.org/\r\n"),
        ("UTF8_STRING", b"https://example.org/"),
    ])
    .unwrap();
    let s = read_after_event(&mut a, &rx);
    assert!(!s.has_files && s.parts.len() == 1);

    // 6. A large image (X11: INCR transfer in chunks; Wayland: a long pipe write), both ways.
    let big = png_like(3 * 1024 * 1024 + 17);
    b.copy_raw(&[("image/png", &big)]).unwrap();
    let s = read_after_event(&mut a, &rx);
    assert!(
        s.parts == vec![(Format::ImagePng, big.clone())],
        "{:?}",
        s.parts.len()
    );
    while rb.try_recv().is_ok() {}
    a.write(&[(Format::ImagePng, big.clone())]).unwrap();
    rb.recv_timeout(WAIT).expect("b sees our write");
    let got = b.read_type("image/png").unwrap();
    assert!(got.as_ref() == Some(&big), "len {:?}", got.map(|g| g.len()));

    // 7. After another app takes over, our old write is gone and its content is read.
    b.copy_raw(&[("UTF8_STRING", b"after")]).unwrap();
    let s = read_after_event(&mut a, &rx);
    assert_eq!(s.parts, vec![(Format::TextPlain, b"after".to_vec())]);
    assert!(!s.has_marker);

    // 8. Seq rises on every change.
    let s1 = a.seq().unwrap();
    b.copy_raw(&[("UTF8_STRING", b"x")]).unwrap();
    rx.recv_timeout(WAIT).unwrap();
    assert!(a.seq().unwrap() > s1);
}

/// Copy with an outside tool and read; then write and paste with the outside tool.
fn interop(mut a: LinuxClipboard, copy: &[&str], paste: &[&str]) {
    let rx = a.subscribe();
    let mut child = Command::new(copy[0])
        .args(&copy[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("copy tool");
    {
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all("from tool ✓".as_bytes())
            .unwrap();
    }
    let s = read_after_event(&mut a, &rx);
    assert_eq!(
        s.parts,
        vec![(Format::TextPlain, "from tool ✓".as_bytes().to_vec())]
    );

    a.write(&[(Format::TextPlain, "from uclip ✓".as_bytes().to_vec())])
        .unwrap();
    let out = Command::new(paste[0])
        .args(&paste[1..])
        .output()
        .expect("paste tool");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "from uclip ✓");
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn x11_live() {
    let Some(display) = live("UCLIP_LIVE_X11") else {
        return;
    };
    let a = LinuxClipboard::open_x11(Some(&display)).unwrap();
    assert_eq!(a.kind(), BackendKind::X11);
    let b = LinuxClipboard::open_x11(Some(&display)).unwrap();
    suite(a, b);
    if have("xclip") {
        let sel = ["-display", display.as_str(), "-selection", "clipboard"];
        let copy: Vec<&str> = ["xclip", "-i"].iter().copied().chain(sel).collect();
        let paste: Vec<&str> = ["xclip", "-o"].iter().copied().chain(sel).collect();
        interop(
            LinuxClipboard::open_x11(Some(&display)).unwrap(),
            &copy,
            &paste,
        );
    }
}

fn wayland_live(protocol: DataControl, kind: BackendKind) {
    if live("UCLIP_LIVE_WAYLAND").is_none() {
        return;
    }
    let a = match LinuxClipboard::open_wayland(protocol) {
        Ok(a) => a,
        Err(e) if std::env::var("UCLIP_LIVE_REQUIRE_EXT").is_err() => {
            // Older compositors (for example Sway 1.9) have only the wlr protocol.
            eprintln!("skip {protocol:?}: {e}");
            return;
        }
        Err(e) => panic!("{protocol:?}: {e}"),
    };
    assert_eq!(a.kind(), kind);
    let b = LinuxClipboard::open_wayland(protocol).unwrap();
    suite(a, b);
    if have("wl-copy") && have("wl-paste") {
        interop(
            LinuxClipboard::open_wayland(protocol).unwrap(),
            &[
                "wl-copy",
                "--foreground",
                "--type",
                "text/plain;charset=utf-8",
            ],
            &["wl-paste", "--no-newline"],
        );
    }
}

#[test]
fn wayland_live_ext_and_wlr() {
    // One test, so the two protocols never share the clipboard at the same time.
    wayland_live(DataControl::Ext, BackendKind::ExtDataControl);
    wayland_live(DataControl::Wlr, BackendKind::WlrDataControl);
}

#[test]
fn open_detected_uses_the_session() {
    if live("UCLIP_LIVE_WAYLAND").is_none() {
        return;
    }
    let d = uc_clip_linux::session::detect_from_env();
    let c = LinuxClipboard::open_detected(&d).unwrap();
    assert!(!c.kind().is_experimental(), "{:?}", c.kind());
}
