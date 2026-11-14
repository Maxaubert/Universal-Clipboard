//! Start at login, per OS (plan 3.2).
//!
//! | OS | Real app | Test profiles |
//! |---|---|---|
//! | Windows | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `UniversalClipboard` | a sandbox key `HKCU\Software\UniversalClipboard-test\<profile>\Run`, which Windows never reads |
//! | macOS | `~/Library/LaunchAgents/io.github.maxaubert.uclip.agent.plist` (`KeepAlive` on crash). The app bundle build (M9) moves this to `SMAppService.agent`. | the same plist in `<state>/autostart/LaunchAgents`, which launchd never reads |
//! | Linux | XDG autostart `~/.config/autostart/io.github.maxaubert.uclip.desktop`, `Exec=uc-agent --supervise` | the same file in `<state>/autostart` |
//!
//! So a test can register and unregister freely: a test profile never touches the real
//! login items of the machine. The Microsoft Store build (spike S8) uses a startup task in
//! the package instead; that is set up in M9.

use std::path::{Path, PathBuf};

use crate::profile::Profile;

/// Where the entry goes.
#[derive(Clone, Debug)]
pub struct Autostart {
    profile: Profile,
    exe: PathBuf,
    #[cfg_attr(windows, allow(dead_code))]
    state_dir: PathBuf,
}

/// The LaunchAgent label.
pub const MAC_LABEL: &str = "io.github.maxaubert.uclip.agent";

impl Autostart {
    /// For this profile, starting `exe`.
    pub fn new(profile: &Profile, exe: PathBuf, state_dir: &Path) -> Autostart {
        Autostart {
            profile: profile.clone(),
            exe,
            state_dir: state_dir.to_path_buf(),
        }
    }

    /// The agent's arguments at login.
    fn args(&self) -> Vec<String> {
        let mut a = Vec::new();
        if cfg!(target_os = "linux") {
            a.push("--supervise".to_string());
        }
        if self.profile.is_test() {
            a.push("--profile".to_string());
            a.push(self.profile.name.clone());
        }
        a
    }

    /// True for a test profile: the entry goes to a sandbox.
    pub fn sandboxed(&self) -> bool {
        self.profile.is_test()
    }

    /// A short description of where the entry is, for the diagnostic report.
    pub fn location(&self) -> String {
        #[cfg(windows)]
        {
            format!(r"HKCU\{}\{}", self.win_key(), self.win_value())
        }
        #[cfg(not(windows))]
        {
            self.file().display().to_string()
        }
    }

    #[cfg(windows)]
    fn win_key(&self) -> String {
        if self.sandboxed() {
            format!(
                r"Software\UniversalClipboard-test\{}\Run",
                self.profile.name
            )
        } else {
            r"Software\Microsoft\Windows\CurrentVersion\Run".to_string()
        }
    }

    #[cfg(windows)]
    fn win_value(&self) -> String {
        if self.sandboxed() {
            format!("UniversalClipboard-{}", self.profile.name)
        } else {
            "UniversalClipboard".to_string()
        }
    }

    #[cfg(not(windows))]
    fn dir(&self) -> PathBuf {
        let sandbox = self.state_dir.join("autostart");
        if cfg!(target_os = "macos") {
            if self.sandboxed() {
                sandbox.join("LaunchAgents")
            } else {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_default()
                    .join("Library/LaunchAgents")
            }
        } else if self.sandboxed() {
            sandbox
        } else {
            uc_clip_linux::autostart::dir_from_env().unwrap_or(sandbox)
        }
    }

    #[cfg(not(windows))]
    fn file(&self) -> PathBuf {
        if cfg!(target_os = "macos") {
            self.dir().join(format!("{MAC_LABEL}.plist"))
        } else {
            self.dir().join("io.github.maxaubert.uclip.desktop")
        }
    }

    /// Turn start at login on.
    pub fn enable(&self) -> std::io::Result<()> {
        #[cfg(windows)]
        {
            let mut line = format!("\"{}\"", self.exe.display());
            for a in self.args() {
                line.push(' ');
                line.push_str(&a);
            }
            crate::win::reg_set(&self.win_key(), &self.win_value(), &line)
        }
        #[cfg(target_os = "macos")]
        {
            std::fs::create_dir_all(self.dir())?;
            std::fs::write(self.file(), mac_plist(&self.exe, &self.args()))
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let mut e = uc_clip_linux::autostart::Entry::agent(self.exe.clone());
            e.args = self.args();
            uc_clip_linux::autostart::install(&self.dir(), &e).map(|_| ())
        }
    }

    /// Turn start at login off.
    pub fn disable(&self) -> std::io::Result<()> {
        #[cfg(windows)]
        {
            crate::win::reg_delete(&self.win_key(), &self.win_value())
        }
        #[cfg(target_os = "macos")]
        {
            match std::fs::remove_file(self.file()) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            }
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            uc_clip_linux::autostart::remove(&self.dir())
        }
    }

    /// Is start at login on?
    pub fn is_enabled(&self) -> bool {
        #[cfg(windows)]
        {
            crate::win::reg_exists(&self.win_key(), &self.win_value())
        }
        #[cfg(target_os = "macos")]
        {
            self.file().exists()
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            uc_clip_linux::autostart::is_enabled(&self.dir())
        }
    }

    /// Remove the test sandbox (Windows: the sandbox registry key). Test cleanup.
    pub fn remove_sandbox(&self) {
        if !self.sandboxed() {
            return;
        }
        #[cfg(windows)]
        {
            let _ = crate::win::reg_delete_tree(&format!(
                r"Software\UniversalClipboard-test\{}",
                self.profile.name
            ));
        }
        #[cfg(not(windows))]
        {
            let _ = std::fs::remove_dir_all(self.state_dir.join("autostart"));
        }
    }
}

/// The LaunchAgent plist: runs the agent at login in the user's GUI session, restarts it on
/// a crash (`KeepAlive` with `SuccessfulExit` false), not after a clean exit.
pub fn mac_plist(exe: &Path, args: &[String]) -> String {
    fn esc(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }
    let mut prog = format!("    <string>{}</string>\n", esc(&exe.display().to_string()));
    for a in args {
        prog.push_str(&format!("    <string>{}</string>\n", esc(a)));
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{MAC_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
{prog}  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>LimitLoadToSessionType</key>
  <string>Aqua</string>
  <key>ProcessType</key>
  <string>Background</string>
</dict>
</plist>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profiles_use_a_sandbox_and_round_trip() {
        let p = Profile::new(&format!("auto-{}", std::process::id())).unwrap();
        let dir = std::env::temp_dir().join(format!("uc-auto-{}", std::process::id()));
        let a = Autostart::new(&p, PathBuf::from("/opt/uc/uc-agent"), &dir);
        assert!(a.sandboxed());
        assert!(!a.location().contains(r"CurrentVersion\Run"));
        assert!(
            !a.location().contains("Library/LaunchAgents/io") || a.location().contains("autostart")
        );
        assert!(!a.is_enabled());
        a.enable().unwrap();
        assert!(a.is_enabled());
        a.disable().unwrap();
        assert!(!a.is_enabled());
        a.remove_sandbox();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plist_keeps_alive_on_crash_only() {
        let s = mac_plist(
            Path::new("/A/uc-agent"),
            &["--profile".into(), "dev".into()],
        );
        assert!(s.contains("<string>/A/uc-agent</string>"));
        assert!(s.contains("<string>dev</string>"));
        assert!(s.contains("SuccessfulExit"));
    }
}
