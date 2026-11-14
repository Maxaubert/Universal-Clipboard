//! `uc-agent`: the headless background program.
//!
//! Usage:
//! - `uc-agent [--profile NAME] [--supervise]`: run the agent.
//! - `uc-agent --version`.
//! - Installer and updater helpers (M9): `--install-hook [--dir DIR]`,
//!   `--uninstall-hook [--dir DIR]`, `--stop`, `--firewall add|remove|check [--exe PATH]`,
//!   `--apply-update ...` (started by the agent itself).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::time::Duration;

use uc_agent::agent::{self, EXIT_ALREADY_RUNNING};
use uc_agent::profile::Profile;

fn take_value(args: &mut Vec<String>, flag: &str) -> Option<String> {
    let i = args.iter().position(|a| a == flag)?;
    let Some(v) = args.get(i + 1).cloned() else {
        eprintln!("{flag} needs a value");
        std::process::exit(2);
    };
    args.drain(i..i + 2);
    Some(v)
}

fn has(args: &mut Vec<String>, flag: &str) -> bool {
    match args.iter().position(|a| a == flag) {
        Some(i) => {
            args.remove(i);
            true
        }
        None => false,
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("uc-agent {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "uc-agent [--profile NAME] [--supervise]\nThe Universal Clipboard background program. Use \"uc\" to talk to it."
        );
        return;
    }
    if let Some(name) = take_value(&mut args, "--profile") {
        // SAFETY of set_var: single-threaded here, before any thread starts.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("UCLIP_PROFILE", name)
        };
    }
    let profile = match Profile::from_env() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let dir = take_value(&mut args, "--dir")
        .map(PathBuf::from)
        .unwrap_or_else(uc_agent::install::own_dir);
    if has(&mut args, "--install-hook") {
        match uc_agent::install::install_hook(&profile, &dir) {
            Ok((_, lines)) => {
                for l in lines {
                    println!("{l}");
                }
                return;
            }
            Err(e) => {
                eprintln!("install hook: {e}");
                std::process::exit(1);
            }
        }
    }
    if has(&mut args, "--uninstall-hook") {
        for l in uc_agent::install::uninstall_hook(&profile, &dir) {
            println!("{l}");
        }
        return;
    }
    if has(&mut args, "--stop") {
        let ok = uc_agent::install::stop_agent(&profile, Duration::from_secs(10));
        std::process::exit(if ok { 0 } else { 1 });
    }
    if let Some(a) = take_value(&mut args, "--firewall") {
        let Some(action) = uc_agent::firewall::Action::parse(&a) else {
            eprintln!("--firewall add, remove or check");
            std::process::exit(2);
        };
        let exe = take_value(&mut args, "--exe")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_exe().unwrap_or_default());
        std::process::exit(uc_agent::firewall::run(&profile, action, &exe));
    }
    if has(&mut args, "--apply-update") {
        std::process::exit(uc_agent::update::run_helper(&profile, &args));
    }
    if uc_agent::update::plays_broken(&profile) {
        // Test only: this copy plays a broken release.
        eprintln!("uc-agent: test-broken");
        std::process::exit(1);
    }
    if has(&mut args, "--supervise") {
        let exe = std::env::current_exe().unwrap_or_else(|_| "uc-agent".into());
        let mut child_args = args.clone();
        if profile.is_test() {
            child_args.extend(["--profile".to_string(), profile.name.clone()]);
        }
        std::process::exit(uc_agent::supervise::supervise(&exe, &child_args, None));
    }
    match agent::run(profile) {
        Ok(()) => {}
        Err(e) if e.0 == "already running" => {
            eprintln!("uc-agent: already running for this user");
            std::process::exit(EXIT_ALREADY_RUNNING);
        }
        Err(e) => {
            eprintln!("uc-agent: {e}");
            std::process::exit(1);
        }
    }
}
