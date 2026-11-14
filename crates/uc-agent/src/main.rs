//! `uc-agent`: the headless background program.
//!
//! Usage: `uc-agent [--profile NAME] [--supervise]`, `uc-agent --version`.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use uc_agent::agent::{self, EXIT_ALREADY_RUNNING};
use uc_agent::profile::Profile;

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
    if let Some(i) = args.iter().position(|a| a == "--profile") {
        let Some(name) = args.get(i + 1).cloned() else {
            eprintln!("--profile needs a name");
            std::process::exit(2);
        };
        // SAFETY of set_var: single-threaded here, before any thread starts.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("UCLIP_PROFILE", name)
        };
        args.drain(i..i + 2);
    }
    let profile = match Profile::from_env() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    if let Some(i) = args.iter().position(|a| a == "--supervise") {
        args.remove(i);
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
