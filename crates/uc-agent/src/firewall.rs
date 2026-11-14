//! The Windows firewall rule (plan 4.1, 8.5; M9).
//!
//! One inbound rule for the agent program: UDP (our QUIC sync and mDNS), profile **Private
//! only**, remote addresses **LocalSubnet only**. Never Public. The rule needs an administrator,
//! so it is added by a one-time helper: `uc-agent --firewall add`, which asks Windows for one
//! UAC prompt when it is not elevated. `uc setup` (and the window) run it when the user says
//! yes to "Let your devices find this PC"; the agent itself never shows a UAC prompt.
//!
//! - Microsoft Store build: the rule is in the package manifest (spike S8), so no helper.
//! - Test profiles never elevate. They get their own rule name, so a CI test can add and
//!   remove a rule without touching the real one.
//! - Removal: `uc-agent --firewall remove`, offered by the uninstaller (plan 8.5).

use std::path::Path;

use crate::profile::Profile;

/// The rule name.
pub fn rule_name(p: &Profile) -> String {
    if p.is_test() {
        format!("Universal Clipboard test {}", p.name)
    } else {
        "Universal Clipboard".to_string()
    }
}

/// The `netsh` arguments that add the rule for `exe`.
pub fn add_args(name: &str, exe: &Path) -> Vec<String> {
    vec![
        "advfirewall".into(),
        "firewall".into(),
        "add".into(),
        "rule".into(),
        format!("name=\"{name}\""),
        "dir=in".into(),
        "action=allow".into(),
        format!("program=\"{}\"", exe.display()),
        "protocol=UDP".into(),
        "profile=private".into(),
        "remoteip=localsubnet".into(),
        "enable=yes".into(),
        "description=\"Lets your own devices on this local network reach Universal Clipboard.\""
            .into(),
    ]
}

/// The `netsh` arguments that delete the rule.
pub fn delete_args(name: &str) -> Vec<String> {
    vec![
        "advfirewall".into(),
        "firewall".into(),
        "delete".into(),
        "rule".into(),
        format!("name=\"{name}\""),
    ]
}

/// The `netsh` arguments that show the rule.
pub fn show_args(name: &str) -> Vec<String> {
    vec![
        "advfirewall".into(),
        "firewall".into(),
        "show".into(),
        "rule".into(),
        format!("name=\"{name}\""),
        "verbose".into(),
    ]
}

/// What the helper was asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Add,
    Remove,
    Check,
}

impl Action {
    pub fn parse(s: &str) -> Option<Action> {
        match s {
            "add" => Some(Action::Add),
            "remove" => Some(Action::Remove),
            "check" => Some(Action::Check),
            _ => None,
        }
    }
    #[cfg_attr(not(windows), allow(dead_code))]
    fn word(self) -> &'static str {
        match self {
            Action::Add => "add",
            Action::Remove => "remove",
            Action::Check => "check",
        }
    }
}

/// True when this build needs our helper for the rule: Windows, not the Store package.
pub fn needs_helper() -> bool {
    #[cfg(windows)]
    {
        !crate::win::is_packaged()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
fn netsh(args: &[String]) -> std::io::Result<std::process::Output> {
    use std::os::windows::process::CommandExt;
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let mut c = std::process::Command::new(Path::new(&root).join("System32").join("netsh.exe"));
    // netsh wants name="a b" exactly as written, so pass the arguments raw.
    for a in args {
        c.raw_arg(a);
    }
    c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    c.output()
}

/// Is the rule there? (No administrator needed.)
#[cfg(windows)]
pub fn exists(p: &Profile) -> bool {
    netsh(&show_args(&rule_name(p))).is_ok_and(|o| o.status.success())
}

/// Run the helper action in this process. Add and remove need an administrator: when this
/// process is not elevated, the same command is started elevated (one UAC prompt) and its
/// result is returned. Exit codes: 0 done, 1 failed, 3 the user said no in the UAC prompt.
#[cfg(windows)]
pub fn run(p: &Profile, action: Action, exe: &Path) -> i32 {
    let name = rule_name(p);
    if action == Action::Check {
        let yes = exists(p);
        println!("{}", if yes { "present" } else { "absent" });
        return if yes { 0 } else { 1 };
    }
    if !crate::win::is_elevated() {
        if p.is_test() && std::env::var("UCLIP_TEST_FIREWALL").as_deref() != Ok("1") {
            eprintln!("test profiles never ask for an administrator");
            return 1;
        }
        let me = std::env::current_exe().unwrap_or_else(|_| "uc-agent.exe".into());
        let mut args = format!("--firewall {} --exe \"{}\"", action.word(), exe.display());
        if p.is_test() {
            args.push_str(&format!(" --profile {}", p.name));
        }
        return match crate::win::run_elevated(&me, &args) {
            Ok(code) => code as i32,
            Err(e) if e.raw_os_error() == Some(1223) => 3,
            Err(e) => {
                eprintln!("could not start the helper: {e}");
                1
            }
        };
    }
    let r = match action {
        Action::Add => {
            // Replace, never stack two rules with the same name.
            let _ = netsh(&delete_args(&name));
            netsh(&add_args(&name, exe))
        }
        Action::Remove => netsh(&delete_args(&name)),
        Action::Check => unreachable!(),
    };
    match r {
        Ok(o) if o.status.success() => 0,
        // Deleting a rule that is not there is fine.
        Ok(_) if action == Action::Remove && !exists(p) => 0,
        Ok(o) => {
            eprintln!("netsh: {}", String::from_utf8_lossy(&o.stdout).trim());
            1
        }
        Err(e) => {
            eprintln!("netsh: {e}");
            1
        }
    }
}

/// Not Windows: nothing to do.
#[cfg(not(windows))]
pub fn run(_p: &Profile, action: Action, _exe: &Path) -> i32 {
    if action == Action::Check {
        println!("absent");
        return 1;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rule_is_private_and_local_subnet_only() {
        let a = add_args("Universal Clipboard", Path::new(r"C:\U\uc-agent.exe")).join(" ");
        assert!(a.contains("profile=private"), "{a}");
        assert!(!a.to_lowercase().contains("public"), "{a}");
        assert!(a.contains("remoteip=localsubnet"));
        assert!(a.contains("dir=in"));
        assert!(a.contains("protocol=UDP"));
        assert!(a.contains(r#"program="C:\U\uc-agent.exe""#));
    }

    #[test]
    fn test_profiles_have_their_own_rule() {
        let real = Profile::new("").unwrap();
        let t = Profile::new("dev").unwrap();
        assert_eq!(rule_name(&real), "Universal Clipboard");
        assert_ne!(rule_name(&t), rule_name(&real));
    }

    /// Real add, check and remove with a test rule. Needs an administrator, so it runs only
    /// where `UCLIP_TEST_FIREWALL=1` is set (the Windows CI job).
    #[cfg(windows)]
    #[test]
    fn real_rule_round_trip_in_ci() {
        if std::env::var("UCLIP_TEST_FIREWALL").as_deref() != Ok("1") || !crate::win::is_elevated()
        {
            return;
        }
        let p = Profile::new(&format!("fw-{}", std::process::id())).unwrap();
        let exe = std::env::current_exe().unwrap();
        assert_eq!(run(&p, Action::Add, &exe), 0);
        assert!(exists(&p));
        let o = netsh(&show_args(&rule_name(&p))).unwrap();
        let s = String::from_utf8_lossy(&o.stdout).to_lowercase();
        assert!(s.contains("private"), "{s}");
        assert!(!s.contains("public"), "{s}");
        assert!(s.contains("localsubnet"), "{s}");
        assert_eq!(run(&p, Action::Remove, &exe), 0);
        assert!(!exists(&p));
        // Removing again is fine.
        assert_eq!(run(&p, Action::Remove, &exe), 0);
    }
}
