//! Install an update with a health check and an automatic rollback (plan 3.13 steps 4 to 6).
//!
//! The update helper runs [`apply`]:
//! 1. note the baseline (how many peers are connected now),
//! 2. stop the agent,
//! 3. keep the old version in a `previous` folder,
//! 4. install the new version,
//! 5. start the new agent and wait for its health check (2 minutes),
//! 6. if it fails: stop it, put the old version back, start it, and mark the version bad.
//!
//! The steps that touch files are behind [`Files`], the steps that touch the agent are behind
//! [`AgentCtl`], so the whole flow is tested headless with fakes, and the real folder swap is
//! tested on temp folders.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::state::UpdateState;

/// What the agent looked like before the update.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Baseline {
    /// Peers connected before the update. With 0 the new agent only has to come up healthy on
    /// its own (IPC open, state loaded, LAN listener bound).
    pub connected: usize,
}

/// The file steps.
pub trait Files {
    /// Copy the installed version to the `previous` folder.
    fn backup(&mut self) -> io::Result<()>;
    /// Put the new version in place.
    fn install(&mut self) -> io::Result<()>;
    /// Put the `previous` folder back.
    fn restore(&mut self) -> io::Result<()>;
}

/// The agent steps.
pub trait AgentCtl {
    fn baseline(&mut self) -> Baseline;
    /// Stop the running agent and wait until it is gone.
    fn stop(&mut self) -> io::Result<()>;
    /// Start the installed agent.
    fn start(&mut self) -> io::Result<()>;
    /// Wait up to `within` for the agent of `version` to be healthy.
    fn healthy(&mut self, version: &str, base: &Baseline, within: Duration) -> Result<(), String>;
}

/// How an attempt ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The new version runs and passed its health check.
    Updated,
    /// Nothing changed (the agent could not be stopped, or the backup failed). The old agent
    /// was started again.
    NotStarted(String),
    /// The new version failed; the old one is back and running.
    RolledBack(String),
    /// The new version failed and the old one could not be put back. The worst case: the
    /// report says so, the next start of the app tries again.
    RollbackFailed(String),
}

/// Run one update attempt and record it in `st` (the caller saves it).
pub fn apply(
    files: &mut dyn Files,
    agent: &mut dyn AgentCtl,
    st: &mut UpdateState,
    version: &str,
    within: Duration,
) -> Outcome {
    let base = agent.baseline();
    if let Err(e) = agent.stop() {
        let why = format!("could not stop the agent: {e}");
        st.failed(version, &why, false);
        let _ = agent.start();
        return Outcome::NotStarted(why);
    }
    if let Err(e) = files.backup() {
        let why = format!("could not keep the old version: {e}");
        st.failed(version, &why, false);
        let _ = agent.start();
        return Outcome::NotStarted(why);
    }
    let failure = match files.install() {
        Err(e) => format!("install failed: {e}"),
        Ok(()) => match agent.start() {
            Err(e) => format!("the new version did not start: {e}"),
            Ok(()) => match agent.healthy(version, &base, within) {
                Ok(()) => {
                    st.succeeded(version);
                    return Outcome::Updated;
                }
                Err(e) => format!("health check failed: {e}"),
            },
        },
    };
    // Roll back. The version itself is bad: never try it again.
    st.failed(version, &failure, true);
    let _ = agent.stop();
    if let Err(e) = files.restore() {
        return Outcome::RollbackFailed(format!("{failure}; restore failed: {e}"));
    }
    if let Err(e) = agent.start() {
        return Outcome::RollbackFailed(format!("{failure}; old version did not start: {e}"));
    }
    Outcome::RolledBack(failure)
}

/// Copy a folder tree. Symbolic links are copied as links on Unix (app bundles may have them).
pub fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let ty = e.file_type()?;
        let dst = to.join(e.file_name());
        if ty.is_dir() {
            copy_dir(&e.path(), &dst)?;
        } else if ty.is_symlink() {
            #[cfg(unix)]
            {
                let target = std::fs::read_link(e.path())?;
                let _ = std::fs::remove_file(&dst);
                std::os::unix::fs::symlink(target, &dst)?;
            }
            #[cfg(not(unix))]
            {
                std::fs::copy(e.path(), &dst)?;
            }
        } else {
            std::fs::copy(e.path(), &dst)?;
        }
    }
    Ok(())
}

/// Remove everything inside `dir` (not `dir` itself), except names in `keep`.
pub fn clear_dir(dir: &Path, keep: &[&str]) -> io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        if keep.iter().any(|k| e.file_name() == *k) {
            continue;
        }
        if e.file_type()?.is_dir() {
            std::fs::remove_dir_all(e.path())?;
        } else {
            std::fs::remove_file(e.path())?;
        }
    }
    Ok(())
}

/// Make `to` hold exactly what `from` holds. Retries for up to 5 s, because Windows keeps a
/// binary locked for a moment after its process ended.
pub fn replace_dir(from: &Path, to: &Path) -> io::Result<()> {
    let t = std::time::Instant::now();
    loop {
        let r = clear_dir(to, &[]).and_then(|()| copy_dir(from, to));
        match r {
            Ok(()) => return Ok(()),
            Err(e) if t.elapsed() > Duration::from_secs(5) => return Err(e),
            Err(_) => std::thread::sleep(Duration::from_millis(250)),
        }
    }
}

/// Replace the install folder with a staged folder (macOS app bundle, and the tests).
pub struct DirFiles {
    pub install_dir: PathBuf,
    pub previous_dir: PathBuf,
    pub staged_dir: PathBuf,
}

impl Files for DirFiles {
    fn backup(&mut self) -> io::Result<()> {
        if self.previous_dir.exists() {
            std::fs::remove_dir_all(&self.previous_dir)?;
        }
        copy_dir(&self.install_dir, &self.previous_dir)
    }

    fn install(&mut self) -> io::Result<()> {
        replace_dir(&self.staged_dir, &self.install_dir)
    }

    fn restore(&mut self) -> io::Result<()> {
        replace_dir(&self.previous_dir, &self.install_dir)
    }
}

/// Windows exe build: run the new per-user NSIS installer silently (no UAC: it installs per
/// user). Backup and restore copy the install folder.
pub struct NsisFiles {
    pub install_dir: PathBuf,
    pub previous_dir: PathBuf,
    pub setup_exe: PathBuf,
    /// Longest wait for the installer.
    pub timeout: Duration,
}

impl Files for NsisFiles {
    fn backup(&mut self) -> io::Result<()> {
        if self.previous_dir.exists() {
            std::fs::remove_dir_all(&self.previous_dir)?;
        }
        copy_dir(&self.install_dir, &self.previous_dir)
    }

    fn install(&mut self) -> io::Result<()> {
        // NSIS: /S silent, /D= the folder (must be last, no quotes). /UPDATE tells our script
        // not to start the app and not to touch the user's settings.
        let mut c = std::process::Command::new(&self.setup_exe);
        c.arg("/S")
            .arg("/UPDATE")
            .arg(format!("/D={}", self.install_dir.display()));
        let mut child = c.spawn()?;
        let t = std::time::Instant::now();
        loop {
            if let Some(st) = child.try_wait()? {
                return if st.success() {
                    Ok(())
                } else {
                    Err(io::Error::other(format!("installer exit {st}")))
                };
            }
            if t.elapsed() > self.timeout {
                let _ = child.kill();
                return Err(io::Error::other("installer timed out"));
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    fn restore(&mut self) -> io::Result<()> {
        // Keep the uninstaller: the old one would also work, but the new one may have been
        // written over it; copying `previous` back restores the old one.
        replace_dir(&self.previous_dir, &self.install_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A fake world: which version is installed, which is running, and a log.
    #[derive(Default)]
    struct World {
        installed: String,
        previous: Option<String>,
        running: Option<String>,
        log: Vec<String>,
    }

    struct FakeFiles {
        w: Rc<RefCell<World>>,
        new: String,
        fail_backup: bool,
        fail_install: bool,
    }

    impl Files for FakeFiles {
        fn backup(&mut self) -> io::Result<()> {
            let mut w = self.w.borrow_mut();
            w.log.push("backup".into());
            if self.fail_backup {
                return Err(io::Error::other("disk full"));
            }
            w.previous = Some(w.installed.clone());
            Ok(())
        }
        fn install(&mut self) -> io::Result<()> {
            let mut w = self.w.borrow_mut();
            w.log.push("install".into());
            if self.fail_install {
                w.installed = "half".into();
                return Err(io::Error::other("locked"));
            }
            w.installed = self.new.clone();
            Ok(())
        }
        fn restore(&mut self) -> io::Result<()> {
            let mut w = self.w.borrow_mut();
            w.log.push("restore".into());
            w.installed = w.previous.clone().unwrap();
            Ok(())
        }
    }

    struct FakeAgent {
        w: Rc<RefCell<World>>,
        broken: String,
        stop_fails: bool,
    }

    impl AgentCtl for FakeAgent {
        fn baseline(&mut self) -> Baseline {
            Baseline { connected: 1 }
        }
        fn stop(&mut self) -> io::Result<()> {
            let mut w = self.w.borrow_mut();
            w.log.push("stop".into());
            if self.stop_fails {
                return Err(io::Error::other("still running"));
            }
            w.running = None;
            Ok(())
        }
        fn start(&mut self) -> io::Result<()> {
            let mut w = self.w.borrow_mut();
            w.log.push("start".into());
            w.running = Some(w.installed.clone());
            Ok(())
        }
        fn healthy(&mut self, v: &str, b: &Baseline, _: Duration) -> Result<(), String> {
            assert_eq!(b.connected, 1);
            let w = self.w.borrow();
            if w.running.as_deref() == Some(v) && v != self.broken {
                Ok(())
            } else {
                Err("no peer within 2 minutes".into())
            }
        }
    }

    fn world() -> Rc<RefCell<World>> {
        Rc::new(RefCell::new(World {
            installed: "1.0.0".into(),
            running: Some("1.0.0".into()),
            ..Default::default()
        }))
    }

    fn run(
        new: &str,
        broken: &str,
        fb: bool,
        fi: bool,
        sf: bool,
        st: &mut UpdateState,
    ) -> (Outcome, Rc<RefCell<World>>) {
        let w = world();
        let mut f = FakeFiles {
            w: w.clone(),
            new: new.into(),
            fail_backup: fb,
            fail_install: fi,
        };
        let mut a = FakeAgent {
            w: w.clone(),
            broken: broken.into(),
            stop_fails: sf,
        };
        (apply(&mut f, &mut a, st, new, Duration::from_secs(1)), w)
    }

    #[test]
    fn good_update() {
        let mut st = UpdateState::default();
        let (o, w) = run("1.0.1", "", false, false, false, &mut st);
        assert_eq!(o, Outcome::Updated);
        assert_eq!(w.borrow().running.as_deref(), Some("1.0.1"));
        assert_eq!(w.borrow().previous.as_deref(), Some("1.0.0"));
        assert_eq!(st.failures_in_row, 0);
    }

    /// Plan 3.13: "version N, then a broken N+1, then rollback to N".
    #[test]
    fn broken_update_rolls_back_and_is_marked_bad() {
        let mut st = UpdateState::default();
        let (o, w) = run("1.0.1", "1.0.1", false, false, false, &mut st);
        assert!(matches!(o, Outcome::RolledBack(_)), "{o:?}");
        assert_eq!(w.borrow().installed, "1.0.0");
        assert_eq!(w.borrow().running.as_deref(), Some("1.0.0"));
        assert!(st.is_bad("1.0.1"));
        assert_eq!(
            w.borrow().log,
            [
                "stop", "backup", "install", "start", "stop", "restore", "start"
            ]
        );
    }

    #[test]
    fn failed_install_rolls_back() {
        let mut st = UpdateState::default();
        let (o, w) = run("1.0.1", "", false, true, false, &mut st);
        assert!(matches!(o, Outcome::RolledBack(_)));
        assert_eq!(w.borrow().installed, "1.0.0");
        assert_eq!(w.borrow().running.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn nothing_changes_when_stop_or_backup_fails() {
        let mut st = UpdateState::default();
        let (o, w) = run("1.0.1", "", false, false, true, &mut st);
        assert!(matches!(o, Outcome::NotStarted(_)));
        assert_eq!(w.borrow().installed, "1.0.0");
        let (o, w) = run("1.0.1", "", true, false, false, &mut st);
        assert!(matches!(o, Outcome::NotStarted(_)));
        assert_eq!(w.borrow().running.as_deref(), Some("1.0.0"));
        assert!(!st.is_bad("1.0.1"));
        assert_eq!(st.failures_in_row, 2);
        let _ = run("1.0.2", "1.0.2", false, false, false, &mut st);
        assert!(st.gave_up());
    }

    #[test]
    fn dir_files_swap_and_restore() {
        let d = tempfile::tempdir().unwrap();
        let inst = d.path().join("app");
        let staged = d.path().join("staged");
        std::fs::create_dir_all(inst.join("sub")).unwrap();
        std::fs::write(inst.join("agent"), "v1").unwrap();
        std::fs::write(inst.join("sub/old-only"), "x").unwrap();
        std::fs::create_dir_all(&staged).unwrap();
        std::fs::write(staged.join("agent"), "v2").unwrap();
        let mut f = DirFiles {
            install_dir: inst.clone(),
            previous_dir: d.path().join("previous"),
            staged_dir: staged,
        };
        f.backup().unwrap();
        f.install().unwrap();
        assert_eq!(std::fs::read_to_string(inst.join("agent")).unwrap(), "v2");
        assert!(!inst.join("sub/old-only").exists());
        f.restore().unwrap();
        assert_eq!(std::fs::read_to_string(inst.join("agent")).unwrap(), "v1");
        assert!(inst.join("sub/old-only").exists());
    }
}
