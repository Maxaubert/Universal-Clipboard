//! Starting the agent, and the two checks around it (2026-11-14, Mac review).
//!
//! - **macOS app bundle: the agent is always a launchd job.** macOS gives Local Network and
//!   "Paste from Other Apps" permission to the *responsible* process. A child of the window
//!   counts as the window, a child of `uc` counts as the terminal. A launchd job is its own
//!   responsible process, so the agent always asks for and keeps its own permissions. The
//!   agent always runs as the session-only job `<MAC_LABEL>.run`, loaded from the state folder
//!   (gone after logout). The login item (`SMAppService`, label [`MAC_LABEL`]) is only a
//!   launcher: it starts that job and exits 0. So turning start at login off (unregister) never
//!   ends the running agent.
//! - **Update in progress.** The update helper writes `<state>/update/applying` while the old
//!   agent is stopped and the files are swapped. Meanwhile `uc` and the window do not start an
//!   agent, and an agent that launchd restarts exits at once (exit 0, so `KeepAlive` stops).
//! - **App location.** An app run from the disk image or from App Translocation (a downloaded
//!   app opened in place) has a path that goes away. Then start at login, the `uc` link and
//!   self-update are refused and the status says to move the app to Applications.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use crate::autostart::MAC_LABEL;
use crate::profile::Profile;

/// The longest an update marker counts, even if its helper still runs (backstop; the NSIS
/// install alone may take 5 minutes).
const MARKER_MAX_AGE: Duration = Duration::from_secs(900);

/// The update marker file.
pub fn update_marker(p: &Profile) -> PathBuf {
    p.state_dir().join("update").join("applying")
}

/// True while the update helper has the agent stopped: the marker is there, not too old, and
/// the helper that wrote it still runs. A stale marker is removed.
pub fn update_in_progress(p: &Profile) -> bool {
    let f = update_marker(p);
    let Ok(m) = std::fs::metadata(&f) else {
        return false;
    };
    let fresh = m
        .modified()
        .is_ok_and(|t| t.elapsed().is_ok_and(|age| age < MARKER_MAX_AGE));
    let alive = std::fs::read_to_string(&f)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .is_some_and(process_alive);
    if fresh && alive {
        return true;
    }
    let _ = std::fs::remove_file(&f);
    false
}

/// True if a process with this ID runs (no unsafe code: `kill -0` / `tasklist`).
fn process_alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    #[cfg(unix)]
    {
        Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
    #[cfg(windows)]
    {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let mut c = Command::new(Path::new(&root).join("System32").join("tasklist.exe"));
        c.args(["/NH", "/FO", "CSV", "/FI", &format!("PID eq {pid}")]);
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000);
        }
        c.output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(&format!("\"{pid}\"")))
    }
}

/// Write or remove the update marker.
pub fn set_update_marker(p: &Profile, on: bool) {
    let f = update_marker(p);
    if on {
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(&f, std::process::id().to_string());
    } else {
        let _ = std::fs::remove_file(&f);
    }
}

/// Why the app cannot stay where it is, or `None`. macOS only. Worked out once per process.
pub fn app_location_problem() -> Option<&'static str> {
    static ONCE: std::sync::OnceLock<Option<&'static str>> = std::sync::OnceLock::new();
    *ONCE.get_or_init(|| {
        let exe = crate::install::real_exe().ok()?;
        location_problem(&exe, read_only_volume)
    })
}

/// True if `path` is on a read-only volume (a mounted disk image), from `mount`'s list.
fn read_only_volume(path: &Path) -> bool {
    let Ok(o) = Command::new("/sbin/mount").output() else {
        return false;
    };
    let s = path.to_string_lossy();
    let mut best: Option<(usize, bool)> = None;
    for line in String::from_utf8_lossy(&o.stdout).lines() {
        // "/dev/disk4s1 on /Volumes/Universal Clipboard (hfs, local, read-only, ...)"
        let Some((_, rest)) = line.split_once(" on ") else {
            continue;
        };
        let Some((point, flags)) = rest.rsplit_once(" (") else {
            continue;
        };
        let inside = s == point || s.starts_with(&format!("{}/", point.trim_end_matches('/')));
        if inside && best.is_none_or(|(n, _)| point.len() > n) {
            best = Some((point.len(), flags.contains("read-only")));
        }
    }
    best.is_some_and(|(_, ro)| ro)
}

fn location_problem(exe: &Path, read_only: impl Fn(&Path) -> bool) -> Option<&'static str> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let s = exe.to_string_lossy();
    if !s.contains(".app/Contents/MacOS/") {
        return None;
    }
    if s.contains("/AppTranslocation/") {
        Some("translocated")
    } else if s.starts_with("/Volumes/") && read_only(exe) {
        // Another drive is fine; a mounted disk image is read-only.
        Some("disk-image")
    } else {
        None
    }
}

/// True when `exe` is the agent inside a macOS app bundle and `p` is the real profile: then it
/// is started through launchd.
fn via_launchd(p: &Profile, exe: &Path) -> bool {
    cfg!(target_os = "macos")
        && !p.is_test()
        && exe.to_string_lossy().contains(".app/Contents/MacOS/")
}

/// Start the agent `exe` for profile `p`, detached. Returns the child when it is a direct child
/// (tests, Windows, Linux); `None` when launchd started it.
pub fn start(p: &Profile, exe: &Path) -> std::io::Result<Option<Child>> {
    if via_launchd(p, exe) {
        launchd_start(p, exe)?;
        return Ok(None);
    }
    let mut c = Command::new(exe);
    if p.is_test() {
        c.args(["--profile", &p.name]);
    }
    detach(&mut c);
    c.spawn().map(Some)
}

/// No inherited stdio, own process group (Unix) or a detached process (Windows).
pub fn detach(c: &mut Command) {
    c.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW
        c.creation_flags(0x0000_0008 | 0x0000_0200 | 0x0800_0000);
        crate::win::no_inherit_std_handles();
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
}

fn launchctl(args: &[&str]) -> bool {
    Command::new("/bin/launchctl")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn gui_domain() -> std::io::Result<String> {
    let out = Command::new("/usr/bin/id").arg("-u").output()?;
    let uid = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if uid.is_empty() || !uid.bytes().all(|b| b.is_ascii_digit()) {
        return Err(std::io::Error::other("no user id"));
    }
    Ok(format!("gui/{uid}"))
}

/// The label of the session-only job.
pub fn run_label() -> String {
    format!("{MAC_LABEL}.run")
}

fn launchd_start(p: &Profile, exe: &Path) -> std::io::Result<()> {
    // An agent that holds the lock (running, or still starting) is never touched.
    if crate::install::agent_running(p) {
        return Ok(());
    }
    let domain = gui_domain()?;
    let label = run_label();
    let job = format!("{domain}/{label}");
    let dir = p.state_dir().join("launchd");
    std::fs::create_dir_all(&dir)?;
    let plist = dir.join(format!("{label}.plist"));
    let want = run_plist(&label, exe);
    let same = std::fs::read_to_string(&plist).is_ok_and(|have| have == want);
    // Loaded with the same program: start it (nothing happens if it already runs).
    if same && launchctl(&["kickstart", &job]) {
        return Ok(());
    }
    // Not loaded, or the app moved: load it fresh. The agent is not running (checked above).
    std::fs::write(&plist, &want)?;
    if !same {
        let _ = launchctl(&["bootout", &job]);
    }
    if launchctl(&["bootstrap", &domain, &plist.to_string_lossy()]) {
        return Ok(());
    }
    // Someone else loaded it at the same moment, or bootout was not finished yet.
    std::thread::sleep(Duration::from_millis(300));
    if launchctl(&["kickstart", &job])
        || launchctl(&["bootstrap", &domain, &plist.to_string_lossy()])
    {
        return Ok(());
    }
    Err(std::io::Error::other("launchctl could not start the agent"))
}

/// The login item ran us: start the session job and leave (exit 0, so `KeepAlive` is done).
pub fn started_as_login_item() -> bool {
    cfg!(target_os = "macos") && std::env::var("XPC_SERVICE_NAME").is_ok_and(|n| n == MAC_LABEL)
}

/// Unload the session job (forget this device). The agent has already quit.
pub fn stop_session_job() {
    if !cfg!(target_os = "macos") {
        return;
    }
    if let Ok(domain) = gui_domain() {
        let _ = launchctl(&["bootout", &format!("{domain}/{}", run_label())]);
    }
}

/// The session-only job: starts now (`RunAtLoad`), restarts after a crash, not after a clean
/// exit (same `KeepAlive` as the login item).
fn run_plist(label: &str, exe: &Path) -> String {
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
  </array>
  <key>AssociatedBundleIdentifiers</key>
  <array>
    <string>io.github.maxaubert.uclip</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>ProcessType</key>
  <string>Background</string>
</dict>
</plist>
"#,
        label = esc(label),
        exe = esc(&exe.to_string_lossy()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locations() {
        let mac = cfg!(target_os = "macos");
        let t = Path::new(
            "/private/var/folders/x/T/AppTranslocation/ABC/d/Universal Clipboard.app/Contents/MacOS/uc-agent",
        );
        assert_eq!(
            location_problem(t, |_| false),
            mac.then_some("translocated")
        );
        let v = Path::new("/Volumes/Universal Clipboard/Universal Clipboard.app/Contents/MacOS/uc");
        assert_eq!(location_problem(v, |_| true), mac.then_some("disk-image"));
        // An app on a second, writable drive is fine.
        assert_eq!(location_problem(v, |_| false), None);
        let ok = Path::new("/Applications/Universal Clipboard.app/Contents/MacOS/uc-agent");
        assert_eq!(location_problem(ok, |_| true), None);
        assert_eq!(
            location_problem(Path::new("/Volumes/x/uc-agent"), |_| true),
            None
        );
    }

    #[test]
    fn run_plist_is_valid_and_escaped() {
        let s = run_plist("l.run", Path::new("/A & B/x.app/Contents/MacOS/uc-agent"));
        assert!(s.contains("<string>/A &amp; B/x.app/Contents/MacOS/uc-agent</string>"));
        assert!(s.contains("<key>RunAtLoad</key>"));
    }

    #[test]
    fn marker_on_and_off() {
        let name = format!("launch-marker-{}", std::process::id());
        let p = Profile::test(&name, std::env::temp_dir().join(&name)).unwrap();
        set_update_marker(&p, true);
        assert!(update_in_progress(&p));
        set_update_marker(&p, false);
        assert!(!update_in_progress(&p));
        // A marker whose helper is gone is stale and removed.
        std::fs::write(update_marker(&p), "4294967290").unwrap();
        assert!(!update_in_progress(&p));
        assert!(!update_marker(&p).exists());
        let _ = std::fs::remove_dir_all(p.state_dir());
    }

    #[test]
    fn tests_never_use_launchd() {
        let p = Profile::test("launch-x", std::env::temp_dir().join("launch-x")).unwrap();
        assert!(!via_launchd(
            &p,
            Path::new("/Applications/U.app/Contents/MacOS/uc-agent")
        ));
    }
}
