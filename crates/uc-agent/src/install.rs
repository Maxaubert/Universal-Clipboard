//! What the installers ask the agent binary to do (M9). The package scripts stay small; the
//! logic is here, in Rust, where it is tested on every OS with test profiles.
//!
//! - `uc-agent --install-hook [--dir DIR]`: put the `uc` command on PATH. If a `uc` command
//!   already exists on this computer, ours is named `uclip` instead, and the hook says so
//!   (plan 3.2). Windows: the install folder goes on the user's PATH (`HKCU\Environment`).
//!   macOS: a link in `/usr/local/bin` when it is writable, else in `~/.local/bin`.
//!   Linux: the `.deb` does this in its `postinst` script with the same rule.
//! - `uc-agent --uninstall-hook [--dir DIR]`: plan 8.5 for the Windows exe build. Tell the
//!   group that this device leaves (2 s limit), stop the agent, remove start at login, the
//!   state folder and the key store entries, and take the folder off PATH.
//! - `uc-agent --stop`: ask the running agent to quit (installers and the updater).
//!
//! Test profiles touch only sandboxes: the registry sandbox key instead of
//! `HKCU\Environment`, `UCLIP_TEST_BIN_DIR` instead of `/usr/local/bin`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;

use crate::autostart::Autostart;
use crate::ipc::Client;
use crate::profile::Profile;

/// The key store entries the agent keeps.
pub const KEY_NAMES: &[&str] = &["device-key", "group-secret", "group-secret-prev"];

/// File names that count as a `uc` command in a PATH folder.
fn uc_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["uc.exe", "uc.cmd", "uc.bat", "uc.com", "uc.ps1"]
    } else {
        &["uc"]
    }
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        let s = p.display().to_string();
        let s = s.trim_end_matches(['\\', '/']).to_string();
        if cfg!(windows) { s.to_lowercase() } else { s }
    };
    norm(a) == norm(b)
}

/// The name of our CLI: `uc`, or `uclip` when a `uc` already exists in a PATH folder other
/// than `own_dirs` (plan 3.2).
pub fn choose_cli_name(path_var: &OsStr, own_dirs: &[&Path]) -> &'static str {
    for d in std::env::split_paths(path_var) {
        if d.as_os_str().is_empty() || own_dirs.iter().any(|o| same_dir(&d, o)) {
            continue;
        }
        for n in uc_names() {
            let f = d.join(n);
            // A link we made earlier in the link folder is ours, not a collision.
            if f.is_file() && !is_our_link(&f) {
                return "uclip";
            }
        }
    }
    "uc"
}

#[cfg(unix)]
fn is_our_link(f: &Path) -> bool {
    std::fs::read_link(f).is_ok_and(|t| {
        t.file_name() == Some(OsStr::new("uc"))
            && t.parent()
                .and_then(Path::file_name)
                .is_some_and(|n| n == "MacOS" || n == "universal-clipboard")
    })
}

#[cfg(not(unix))]
fn is_our_link(_f: &Path) -> bool {
    false
}

/// Add `dir` to a PATH-style list if it is missing. Returns the new list, or `None` if it was
/// there already.
pub fn path_with(list: &str, dir: &Path) -> Option<String> {
    let parts: Vec<&str> = list.split(';').filter(|s| !s.is_empty()).collect();
    if parts.iter().any(|p| same_dir(Path::new(p), dir)) {
        return None;
    }
    let mut v: Vec<String> = parts.iter().map(|s| s.to_string()).collect();
    v.push(dir.display().to_string());
    Some(v.join(";"))
}

/// Remove `dir` from a PATH-style list. `None` if it was not there.
pub fn path_without(list: &str, dir: &Path) -> Option<String> {
    let parts: Vec<&str> = list.split(';').filter(|s| !s.is_empty()).collect();
    let kept: Vec<&str> = parts
        .iter()
        .copied()
        .filter(|p| !same_dir(Path::new(p), dir))
        .collect();
    if kept.len() == parts.len() {
        None
    } else {
        Some(kept.join(";"))
    }
}

#[cfg(windows)]
fn env_key(p: &Profile) -> String {
    if p.is_test() {
        format!(r"Software\UniversalClipboard-test\{}\Environment", p.name)
    } else {
        "Environment".to_string()
    }
}

/// This program's path with symlinks resolved. On macOS and Linux `uc` is often run through a
/// link in `/usr/local/bin`, `~/.local/bin` or `/usr/bin`, and `current_exe` then returns the
/// link, so sibling programs (`uc-agent`) would be looked up next to the link. Windows keeps
/// `current_exe` as is (canonical paths there get a `\\?\` prefix).
pub fn real_exe() -> std::io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    if cfg!(windows) {
        return Ok(exe);
    }
    Ok(std::fs::canonicalize(&exe).unwrap_or(exe))
}

/// The folder of this program.
pub fn own_dir() -> PathBuf {
    real_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
        .unwrap_or_default()
}

/// `--install-hook`. Returns the CLI name and the lines to show.
pub fn install_hook(p: &Profile, dir: &Path) -> std::io::Result<(String, Vec<String>)> {
    let mut lines = Vec::new();
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    #[cfg(windows)]
    {
        let name = choose_cli_name(&path_var, &[dir]);
        let uc = dir.join("uc.exe");
        let uclip = dir.join("uclip.exe");
        if name == "uclip" && uc.exists() {
            let _ = std::fs::remove_file(&uclip);
            std::fs::rename(&uc, &uclip)?;
        }
        let key = env_key(p);
        let (old, ty) = crate::win::reg_get(&key, "Path")?.unwrap_or((String::new(), 2));
        if let Some(new) = path_with(&old, dir) {
            // Keep REG_EXPAND_SZ (2) so %VARS% in the user's PATH still work.
            crate::win::reg_set_typed(&key, "Path", &new, if ty == 1 { 1 } else { 2 })?;
            if !p.is_test() {
                crate::win::broadcast_env_change();
            }
        }
        std::fs::write(dir.join("cli-name.txt"), name)?;
        lines.push(cli_line(name));
        if name == "uc" {
            lines.push("Open a new terminal to use it.".into());
        }
        Ok((name.to_string(), lines))
    }
    #[cfg(not(windows))]
    {
        // A link to the disk image or a translocated copy breaks when that path goes away.
        if !p.is_test() && crate::launch::app_location_problem().is_some() {
            return Err(std::io::Error::other(
                "Move Universal Clipboard to the Applications folder and open it from there first.",
            ));
        }
        let src = dir.join("uc");
        let link_dir = unix_link_dir(p);
        std::fs::create_dir_all(&link_dir)?;
        let name = choose_cli_name(&path_var, &[dir, &link_dir]);
        // Our earlier link in the link folder does not count; another `uc` there does.
        let name = if link_dir.join("uc").exists() && !is_our_link(&link_dir.join("uc")) {
            "uclip"
        } else {
            name
        };
        let link = link_dir.join(name);
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&src, &link)?;
        lines.push(cli_line(name));
        let on_path = std::env::split_paths(&path_var).any(|d| same_dir(&d, &link_dir));
        if !on_path {
            lines.push(format!(
                "Add {} to your PATH, or run {}.",
                link_dir.display(),
                link.display()
            ));
        }
        Ok((name.to_string(), lines))
    }
}

fn cli_line(name: &str) -> String {
    if name == "uc" {
        "The command line tool is \"uc\".".into()
    } else {
        "A \"uc\" command already exists on this computer, so the command line tool is named \"uclip\".".into()
    }
}

#[cfg(not(windows))]
fn unix_link_dir(p: &Profile) -> PathBuf {
    if let Some(d) = p.test_env("UCLIP_TEST_BIN_DIR") {
        return PathBuf::from(d);
    }
    if p.is_test() {
        return p.state_dir().join("bin");
    }
    let usr = Path::new("/usr/local/bin");
    let writable =
        std::fs::metadata(usr).is_ok_and(|m| !m.permissions().readonly()) && tempfile_probe(usr);
    if writable {
        usr.to_path_buf()
    } else {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(".local/bin")
    }
}

#[cfg(not(windows))]
fn tempfile_probe(dir: &Path) -> bool {
    let f = dir.join(format!(".uclip-probe-{}", std::process::id()));
    let ok = std::fs::write(&f, b"").is_ok();
    let _ = std::fs::remove_file(&f);
    ok
}

/// Ask the running agent to quit, and wait until its lock is free (at most `wait`).
/// Returns true if no agent runs afterwards.
pub fn stop_agent(p: &Profile, wait: Duration) -> bool {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(_) => return false,
    };
    rt.block_on(async {
        if let Ok(Ok(mut c)) =
            tokio::time::timeout(Duration::from_secs(2), Client::connect(p)).await
        {
            let _ = tokio::time::timeout(Duration::from_secs(2), c.call("quit", json!({}))).await;
        }
    });
    let t = std::time::Instant::now();
    while t.elapsed() < wait {
        if !agent_running(p) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    !agent_running(p)
}

/// True if an agent holds the lock of this profile.
pub fn agent_running(p: &Profile) -> bool {
    let f = p.state_dir().join("agent.lock");
    let Ok(file) = std::fs::OpenOptions::new().write(true).open(&f) else {
        return false;
    };
    match file.try_lock() {
        Ok(()) => {
            let _ = file.unlock();
            false
        }
        Err(_) => true,
    }
}

/// `--uninstall-hook` (plan 8.5, Windows exe build; usable on every OS). Returns the lines to
/// show.
pub fn uninstall_hook(p: &Profile, dir: &Path) -> Vec<String> {
    let mut lines = Vec::new();
    // 1. Tell the group (2 s limit): "leave" signs our own removal and spreads it.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    let told = rt.is_ok_and(|rt| {
        rt.block_on(async {
            let Ok(Ok(mut c)) =
                tokio::time::timeout(Duration::from_millis(500), Client::connect(p)).await
            else {
                return false;
            };
            matches!(
                tokio::time::timeout(
                    Duration::from_secs(2),
                    c.call("leave", json!({"forget": true}))
                )
                .await,
                Ok(Ok(Ok(_)))
            )
        })
    });
    lines.push(if told {
        "Told your other devices that this device left.".into()
    } else {
        "Could not tell your other devices; they show this device as \"Asleep or away\" until you remove it there.".into()
    });
    // 2. Stop the agent.
    if !stop_agent(p, Duration::from_secs(5)) {
        lines.push("The background service did not stop in time.".into());
    }
    if !p.is_test() {
        crate::launch::stop_session_job();
    }
    // 3. Start at login.
    let state = p.state_dir();
    let auto = Autostart::new(p, dir.join(agent_exe_name()), &state);
    let _ = auto.disable();
    // 4. Key store entries.
    let store = uc_keys::open_store(p.keystore_name(), &state);
    for k in KEY_NAMES {
        let _ = store.delete(k);
    }
    // 5. State folder.
    if state.exists() && std::fs::remove_dir_all(&state).is_err() {
        lines.push(format!("Could not remove {} completely.", state.display()));
    }
    // 6. PATH.
    #[cfg(windows)]
    {
        let key = env_key(p);
        if let Ok(Some((old, ty))) = crate::win::reg_get(&key, "Path")
            && let Some(new) = path_without(&old, dir)
        {
            let _ = crate::win::reg_set_typed(&key, "Path", &new, ty);
            if !p.is_test() {
                crate::win::broadcast_env_change();
            }
        }
    }
    #[cfg(not(windows))]
    {
        let ld = unix_link_dir(p);
        for n in ["uc", "uclip"] {
            let l = ld.join(n);
            if std::fs::read_link(&l).is_ok_and(|t| t.parent() == Some(dir)) {
                let _ = std::fs::remove_file(&l);
            }
        }
    }
    lines.push("Removed start at login, the settings and the keys of this device.".into());
    lines
}

fn agent_exe_name() -> &'static str {
    if cfg!(windows) {
        "uc-agent.exe"
    } else {
        "uc-agent"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_name_rule() {
        let d = tempfile::tempdir().unwrap();
        let ours = d.path().join("ours");
        let other = d.path().join("other");
        std::fs::create_dir_all(&ours).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let uc = if cfg!(windows) { "uc.exe" } else { "uc" };
        std::fs::write(ours.join(uc), b"").unwrap();
        let path = std::env::join_paths([&other, &ours]).unwrap();
        // Our own folder does not count.
        assert_eq!(choose_cli_name(&path, &[&ours]), "uc");
        // Another `uc` on PATH: we become `uclip`.
        std::fs::write(other.join(uc), b"").unwrap();
        assert_eq!(choose_cli_name(&path, &[&ours]), "uclip");
    }

    #[test]
    fn path_list_edits() {
        let d = if cfg!(windows) {
            Path::new(r"C:\Users\a\AppData\Local\Programs\Universal Clipboard")
        } else {
            Path::new("/opt/uc")
        };
        let base = "C:\\x;%USERPROFILE%\\bin";
        let added = path_with(base, d).unwrap();
        assert!(added.starts_with(base));
        assert!(path_with(&added, d).is_none());
        assert_eq!(path_without(&added, d).unwrap(), base);
        assert!(path_without(base, d).is_none());
    }

    /// The install and uninstall hooks in a test profile: only sandboxes are touched.
    #[test]
    fn hooks_round_trip_in_a_sandbox() {
        let tmp = tempfile::tempdir().unwrap();
        let name = format!("ih-{}", std::process::id());
        let p = Profile::test(&name, tmp.path().join("state")).unwrap();
        // SAFETY of set_var: nextest runs each test in its own process.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("UCLIP_TEST_KEYSTORE", "file")
        };
        let dir = tmp.path().join("app");
        std::fs::create_dir_all(&dir).unwrap();
        let uc = if cfg!(windows) { "uc.exe" } else { "uc" };
        std::fs::write(dir.join(uc), b"").unwrap();
        std::fs::create_dir_all(p.state_dir()).unwrap();
        std::fs::write(p.state_dir().join("state.json"), b"{}").unwrap();
        #[cfg(not(windows))]
        // SAFETY of set_var: this test is the only user of this variable.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("UCLIP_TEST_BIN_DIR", tmp.path().join("bin"))
        };
        let (n, lines) = install_hook(&p, &dir).unwrap();
        assert!(n == "uc" || n == "uclip", "{n}");
        assert!(!lines.is_empty());
        #[cfg(windows)]
        {
            let (v, _) = crate::win::reg_get(&env_key(&p), "Path").unwrap().unwrap();
            assert!(v.contains(&dir.display().to_string()));
        }
        // Start at login on, then the uninstall hook removes everything.
        let auto = Autostart::new(&p, dir.join(agent_exe_name()), &p.state_dir());
        auto.enable().unwrap();
        let out = uninstall_hook(&p, &dir);
        assert!(!out.is_empty());
        assert!(!auto.is_enabled());
        assert!(!p.state_dir().exists());
        #[cfg(windows)]
        {
            let v = crate::win::reg_get(&env_key(&p), "Path").unwrap();
            assert!(v.is_none_or(|(s, _)| !s.contains(&dir.display().to_string())));
            let _ =
                crate::win::reg_delete_tree(&format!(r"Software\UniversalClipboard-test\{name}"));
        }
        #[cfg(not(windows))]
        assert!(!tmp.path().join("bin").join(&n).exists());
        auto.remove_sandbox();
    }
}
