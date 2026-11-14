//! `winhost [--timeout SECS] <exe> [args...]`: run a program inside a new private window
//! station and desktop, print its output, and exit with its exit code.

#[cfg(windows)]
fn main() {
    use std::ffi::OsString;
    use std::time::Duration;

    let mut args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let mut timeout = Duration::from_secs(600);
    if args.first().is_some_and(|a| a == "--timeout") {
        let secs = args
            .get(1)
            .and_then(|s| s.to_str()?.parse::<u64>().ok())
            .unwrap_or_else(|| usage());
        timeout = Duration::from_secs(secs);
        args.drain(..2);
    }
    if args.is_empty() {
        usage();
    }
    let exe = std::path::PathBuf::from(args.remove(0));
    match uc_winhost::run(&exe, &args, &[], timeout) {
        Ok(out) => {
            eprintln!("winhost: station {}", out.station);
            print!("{}", out.output);
            if out.timed_out {
                eprintln!("winhost: timed out");
            }
            std::process::exit(out.exit_code as i32);
        }
        Err(e) => {
            eprintln!("winhost: {e}");
            std::process::exit(2);
        }
    }
}

#[cfg(windows)]
fn usage() -> ! {
    eprintln!("usage: winhost [--timeout SECS] <exe> [args...]");
    std::process::exit(2);
}

#[cfg(not(windows))]
fn main() {
    eprintln!("winhost runs on Windows only");
    std::process::exit(2);
}
