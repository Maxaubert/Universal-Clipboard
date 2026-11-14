//! `uc`: the command line interface (plan 3.2). It is a full user interface, not only a test
//! tool: everything the settings window can do, `uc` can do, on every desktop OS, also over
//! SSH and on a computer with no screen. It talks only to the local agent through the IPC
//! pipe or socket and holds no keys.
//!
//! There is no "send" and no "get" command: the latest copy is on every device on its own.

use std::io::{BufRead, IsTerminal, Write};
use std::time::Duration;

use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use uc_agent::ipc::{Client, IpcError};
use uc_agent::profile::Profile;

/// Universal Clipboard: copy on one device, paste on another.
#[derive(Debug, Parser)]
#[command(name = "uc", version, about, disable_help_subcommand = true)]
pub struct Cli {
    /// Print the result as JSON (for scripts).
    #[arg(long, global = true)]
    pub json: bool,
    /// What to do.
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

/// The commands of plan 3.2.
#[derive(Debug, Subcommand)]
pub enum Cmd {
    /// Working, Paused or Needs attention, the reason, and the device count.
    Status,
    /// Your devices: name, OS, Connected or Asleep. This device is marked.
    Devices,
    /// Give a device a new name.
    Rename {
        /// The device: its name or the start of its ID (see "uc devices").
        device: String,
        /// The new name.
        name: String,
    },
    /// Remove a device from your devices.
    Remove {
        /// The device: its name or the start of its ID.
        device: String,
        /// Do not ask to confirm.
        #[arg(long)]
        yes: bool,
    },
    /// Add a device: show a code here, or enter the code of another device.
    Pair {
        /// show, enter or cancel.
        #[command(subcommand)]
        action: PairCmd,
    },
    /// Pause sync.
    Pause {
        /// For how long, for example 1h or 30m. Without it: until "uc resume".
        #[arg(long = "for", value_name = "TIME")]
        for_: Option<String>,
    },
    /// Turn sync on again.
    Resume,
    /// Show all settings, or change one.
    Settings {
        /// set KEY VALUE.
        #[command(subcommand)]
        action: Option<SettingsCmd>,
    },
    /// The diagnostic report (no clipboard content, no keys).
    Diagnostics,
    /// The first-run steps (permissions, start at sign-in) for a computer that never opens the
    /// window.
    Setup {
        /// Answer yes to every step.
        #[arg(long)]
        yes: bool,
    },
    /// Remove this device from your devices and forget everything.
    Leave {
        /// Do not ask to confirm.
        #[arg(long)]
        yes: bool,
    },
    /// Show help.
    Help,
}

/// `uc pair ...`.
#[derive(Debug, Subcommand)]
pub enum PairCmd {
    /// Show a 6-digit code and a QR code, and wait for the other device.
    Show,
    /// Enter the code that another device shows.
    Enter {
        /// The 6 digits. Asked for if not given.
        code: Option<String>,
        /// If this device has other devices already: leave them without asking.
        #[arg(long)]
        yes: bool,
    },
    /// Stop showing the code.
    Cancel,
}

/// `uc settings set ...`.
#[derive(Debug, Subcommand)]
pub enum SettingsCmd {
    /// Change one setting, for example "uc settings set sync-images off".
    Set {
        /// The key (see "uc settings").
        key: String,
        /// The new value.
        value: String,
    },
}

impl Cmd {
    /// The IPC calls this command makes (for the mapping test).
    pub fn calls(&self) -> Vec<&'static str> {
        match self {
            Cmd::Status => vec!["status"],
            Cmd::Devices => vec!["devices.list"],
            Cmd::Rename { .. } => vec!["devices.rename"],
            Cmd::Remove { .. } => vec!["devices.remove"],
            Cmd::Pair { action } => match action {
                PairCmd::Show => vec!["subscribe", "pair.start", "pair.cancel"],
                PairCmd::Enter { .. } => vec!["pair.join"],
                PairCmd::Cancel => vec!["pair.cancel"],
            },
            Cmd::Pause { .. } | Cmd::Resume => vec!["pause.set"],
            Cmd::Settings { action: None } => vec!["settings.get"],
            Cmd::Settings { action: Some(_) } => vec!["settings.set"],
            Cmd::Diagnostics => vec!["diagnostics"],
            Cmd::Setup { .. } => vec!["setup.status", "setup.step"],
            Cmd::Leave { .. } => vec!["leave"],
            Cmd::Help => vec![],
        }
    }
}

/// Every action of the settings window (plan 2.5, 2.7) and the `uc` command that does the
/// same. A test checks that each command parses and makes the right IPC call.
pub const UI_ACTIONS: &[(&str, &[&str], &str)] = &[
    ("State line", &["uc", "status"], "status"),
    ("Last activity", &["uc", "status"], "status"),
    ("Devices list", &["uc", "devices"], "devices.list"),
    (
        "Device menu: Rename",
        &["uc", "rename", "Mac mini", "Studio Mac"],
        "devices.rename",
    ),
    (
        "Device menu: Remove",
        &["uc", "remove", "Mac mini", "--yes"],
        "devices.remove",
    ),
    (
        "Add a device: This is my first device (QR and code)",
        &["uc", "pair", "show"],
        "pair.start",
    ),
    (
        "Add a device: I have a code",
        &["uc", "pair", "enter", "418207"],
        "pair.join",
    ),
    (
        "Add a device: Cancel",
        &["uc", "pair", "cancel"],
        "pair.cancel",
    ),
    (
        "Pause switch: Until I turn it on",
        &["uc", "pause"],
        "pause.set",
    ),
    (
        "Pause switch: Pause for 1 hour",
        &["uc", "pause", "--for", "1h"],
        "pause.set",
    ),
    ("Pause switch off", &["uc", "resume"], "pause.set"),
    ("Settings (show)", &["uc", "settings"], "settings.get"),
    (
        "Start at login",
        &["uc", "settings", "set", "start-at-login", "off"],
        "settings.set",
    ),
    (
        "Sync images",
        &["uc", "settings", "set", "sync-images", "off"],
        "settings.set",
    ),
    (
        "Image size cap",
        &["uc", "settings", "set", "image-limit-mb", "10"],
        "settings.set",
    ),
    (
        "Keep formatting",
        &["uc", "settings", "set", "keep-formatting", "off"],
        "settings.set",
    ),
    (
        "macOS: Also pass on what Apple's Universal Clipboard brings",
        &["uc", "settings", "set", "forward-apple-clips", "on"],
        "settings.set",
    ),
    (
        "This device's name",
        &["uc", "settings", "set", "device-name", "Max's PC"],
        "settings.set",
    ),
    (
        "Help: Copy a diagnostic report",
        &["uc", "diagnostics"],
        "diagnostics",
    ),
    ("Help: version", &["uc", "--version"], ""),
    (
        "Help: Remove this device and forget everything",
        &["uc", "leave", "--yes"],
        "leave",
    ),
    (
        "Needs attention: Fix (permissions)",
        &["uc", "setup"],
        "setup.step",
    ),
    (
        "First run: permissions and start at login",
        &["uc", "setup", "--yes"],
        "setup.status",
    ),
];

fn out(json_mode: bool, v: &Value, text: impl FnOnce() -> String) {
    if json_mode {
        println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
    } else {
        println!("{}", text());
    }
}

fn ask(question: &str) -> Option<String> {
    if !std::io::stdin().is_terminal() {
        return None;
    }
    print!("{question} ");
    let _ = std::io::stdout().flush();
    let mut s = String::new();
    std::io::stdin().lock().read_line(&mut s).ok()?;
    Some(s.trim().to_string())
}

fn yes_no(question: &str) -> Option<bool> {
    ask(&format!("{question} [y/n]")).map(|a| matches!(a.to_lowercase().as_str(), "y" | "yes"))
}

/// Parse `1h`, `30m`, `90` (minutes).
pub fn parse_minutes(s: &str) -> Option<u64> {
    let s = s.trim().to_lowercase();
    if let Some(h) = s.strip_suffix('h') {
        return h.trim().parse::<u64>().ok().map(|h| h * 60);
    }
    if let Some(m) = s.strip_suffix('m') {
        return m.trim().parse().ok();
    }
    s.parse().ok()
}

/// Draw the QR code in the terminal (light modules as spaces, so it reads on a dark terminal).
pub fn qr_text(payload: &str) -> Option<String> {
    use qrcode::render::unicode::Dense1x2;
    let code = qrcode::QrCode::with_error_correction_level(payload, qrcode::EcLevel::M).ok()?;
    Some(
        code.render::<Dense1x2>()
            .dark_color(Dense1x2::Light)
            .light_color(Dense1x2::Dark)
            .quiet_zone(true)
            .build(),
    )
}

/// A CLI failure: the message and the exit code.
#[derive(Debug)]
pub struct Fail(pub String, pub i32);

impl From<IpcError> for Fail {
    fn from(e: IpcError) -> Self {
        Fail(e.message, 1)
    }
}

impl From<std::io::Error> for Fail {
    fn from(e: std::io::Error) -> Self {
        Fail(format!("Cannot talk to the agent: {e}"), 2)
    }
}

/// Connect, starting the agent if none answers (like the window does, plan 3.2).
pub async fn connect(p: &Profile) -> Result<Client, Fail> {
    if let Ok(c) = Client::connect(p).await {
        return Ok(c);
    }
    if std::env::var("UCLIP_NO_START").is_err() {
        start_agent(p);
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if let Ok(c) = Client::connect(p).await {
                return Ok(c);
            }
        }
    }
    Err(Fail(
        "The background service is not running and could not be started.".into(),
        2,
    ))
}

fn start_agent(p: &Profile) {
    let Ok(me) = std::env::current_exe() else {
        return;
    };
    let name = if cfg!(windows) {
        "uc-agent.exe"
    } else {
        "uc-agent"
    };
    let exe = me.with_file_name(name);
    if !exe.exists() {
        return;
    }
    let mut c = std::process::Command::new(exe);
    if p.is_test() {
        c.args(["--profile", &p.name]);
    }
    c.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NO_WINDOW: no console, nothing on screen.
        c.creation_flags(0x0000_0008 | 0x0800_0000);
        // The agent must not hold our output pipe ("uc status | ..." would never end).
        uc_agent::win::no_inherit_std_handles();
    }
    let _ = c.spawn();
}

async fn call(c: &mut Client, name: &str, params: Value) -> Result<Value, Fail> {
    Ok(c.call(name, params).await??)
}

fn when(ms: u64) -> String {
    let now = uc_agent::agent::now_ms();
    let s = now.saturating_sub(ms) / 1000;
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86_399 => format!("{} h ago", s / 3600),
        _ => format!("{} days ago", s / 86_400),
    }
}

/// Run one command. Returns the exit code.
pub async fn run(cli: Cli) -> Result<(), Fail> {
    let j = cli.json;
    let Some(cmd) = cli.cmd else {
        print_help();
        return Ok(());
    };
    if let Cmd::Help = cmd {
        print_help();
        return Ok(());
    }
    let p = Profile::from_env().map_err(|e| Fail(e.to_string(), 2))?;
    let mut c = connect(&p).await?;
    match cmd {
        Cmd::Status => {
            let v = call(&mut c, "status", json!({})).await?;
            out(j, &v, || status_text(&v));
        }
        Cmd::Devices => {
            let v = call(&mut c, "devices.list", json!({})).await?;
            out(j, &v, || {
                let mut s = String::new();
                for d in v.as_array().into_iter().flatten() {
                    let mark = if d["this"].as_bool() == Some(true) {
                        "*"
                    } else {
                        " "
                    };
                    s.push_str(&format!(
                        "{mark} {:<24} {:<14} {:<28} {}\n",
                        d["name"].as_str().unwrap_or(""),
                        d["os"].as_str().unwrap_or(""),
                        d["state"].as_str().unwrap_or(""),
                        &d["id"].as_str().unwrap_or("")
                            [..8.min(d["id"].as_str().unwrap_or("").len())],
                    ));
                }
                s.push_str("* this device");
                s
            });
        }
        Cmd::Rename { device, name } => {
            let v = call(
                &mut c,
                "devices.rename",
                json!({"device": device, "name": name}),
            )
            .await?;
            out(j, &v, || {
                format!("Renamed to {}.", v["name"].as_str().unwrap_or(""))
            });
        }
        Cmd::Remove { device, yes } => {
            if !yes {
                match yes_no(&format!("Remove \"{device}\" from your devices?")) {
                    Some(true) => {}
                    Some(false) => return Ok(()),
                    None => return Err(Fail("Add --yes to remove without a question.".into(), 2)),
                }
            }
            let v = call(&mut c, "devices.remove", json!({"device": device})).await?;
            out(j, &v, || {
                format!("{} is removed.", v["name"].as_str().unwrap_or(""))
            });
        }
        Cmd::Pair { action } => pair(&mut c, j, action).await?,
        Cmd::Pause { for_ } => {
            let params = match for_ {
                None => json!({"mode": "forever"}),
                Some(t) => {
                    let m = parse_minutes(&t)
                        .filter(|m| *m > 0)
                        .ok_or_else(|| Fail("Use a time like 1h or 30m.".into(), 2))?;
                    json!({"mode": "minutes", "minutes": m})
                }
            };
            let v = call(&mut c, "pause.set", params).await?;
            out(j, &v, || "Sync is paused.".into());
        }
        Cmd::Resume => {
            let v = call(&mut c, "pause.set", json!({"mode": "off"})).await?;
            out(j, &v, || "Sync is on.".into());
        }
        Cmd::Settings { action: None } => {
            let v = call(&mut c, "settings.get", json!({})).await?;
            out(j, &v, || {
                let mut s = String::new();
                if let Some(m) = v.as_object() {
                    for (k, x) in m {
                        s.push_str(&format!(
                            "{:<22} {:<10} {}\n",
                            k,
                            x["value"].as_str().unwrap_or(""),
                            x["label"].as_str().unwrap_or("")
                        ));
                    }
                }
                s.push_str("Change one: uc settings set <key> <value>");
                s
            });
        }
        Cmd::Settings {
            action: Some(SettingsCmd::Set { key, value }),
        } => {
            let v = call(&mut c, "settings.set", json!({"key": key, "value": value})).await?;
            out(j, &v, || {
                format!("{key} is now {}.", v["value"].as_str().unwrap_or(&value))
            });
        }
        Cmd::Diagnostics => {
            let v = call(&mut c, "diagnostics", json!({})).await?;
            out(j, &v, || v["report"].as_str().unwrap_or("").to_string());
        }
        Cmd::Setup { yes } => setup(&mut c, &p, j, yes).await?,
        Cmd::Leave { yes } => {
            if !yes {
                match yes_no(
                    "Remove this device and forget everything? Your other devices keep working.",
                ) {
                    Some(true) => {}
                    Some(false) => return Ok(()),
                    None => return Err(Fail("Add --yes to leave without a question.".into(), 2)),
                }
            }
            let v = call(&mut c, "leave", json!({"forget": true})).await?;
            out(j, &v, || {
                "This device left your devices and forgot everything.".into()
            });
        }
        Cmd::Help => {}
    }
    Ok(())
}

fn status_text(v: &Value) -> String {
    let mut s = v["text"].as_str().unwrap_or("").to_string();
    if let Some(r) = v["reason"].as_str() {
        s.push_str(&format!("\n{r}"));
    }
    if let Some(f) = v["fix"].as_str() {
        s.push_str(&format!("\nFix: {f}"));
    }
    s.push_str(&format!(
        "\nDevices: {} ({} connected). This device: {}.",
        v["devices"],
        v["connected"],
        v["this_device"].as_str().unwrap_or("")
    ));
    if let Some(a) = v["activity"].as_array().and_then(|a| a.first()) {
        let dev = a["device"]
            .as_str()
            .map(|d| format!(", from {d}"))
            .unwrap_or_default();
        let note = a["note"]
            .as_str()
            .map(|n| format!(" {n}"))
            .unwrap_or_default();
        s.push_str(&format!(
            "\nLast activity: {}, {}{}, {}.{}",
            a["kind"].as_str().unwrap_or(""),
            a["direction"].as_str().unwrap_or(""),
            dev,
            when(a["time_ms"].as_u64().unwrap_or(0)),
            note
        ));
    }
    s
}

async fn pair(c: &mut Client, j: bool, action: PairCmd) -> Result<(), Fail> {
    match action {
        PairCmd::Cancel => {
            let v = call(c, "pair.cancel", json!({})).await?;
            out(j, &v, || "The code is closed.".into());
        }
        PairCmd::Enter { code, yes } => {
            let code = match code {
                Some(c) => c,
                None => ask("Type the 6-digit code from the other device:")
                    .ok_or_else(|| Fail("Give the code: uc pair enter 418207".into(), 2))?,
            };
            if !j {
                println!(
                    "Both devices must be on the same local network. Looking for the other device..."
                );
            }
            let test_addrs = Profile::from_env()
                .ok()
                .and_then(|p| p.test_env("UCLIP_TEST_PAIR_ADDRS"));
            let mut r = c
                .call(
                    "pair.join",
                    json!({"code": code, "leave_other": yes, "test_addrs": test_addrs}),
                )
                .await?;
            if let Err(e) = &r
                && e.code == "in-group"
            {
                match yes_no(&e.message) {
                    Some(true) => {
                        r = c
                            .call(
                                "pair.join",
                                json!({"code": code, "leave_other": true, "test_addrs": test_addrs}),
                            )
                            .await?;
                    }
                    Some(false) => return Ok(()),
                    None => return Err(Fail(format!("{} Add --yes to say yes.", e.message), 2)),
                }
            }
            let v = r?;
            out(j, &v, || {
                format!("Connected to {}.", v["name"].as_str().unwrap_or(""))
            });
        }
        PairCmd::Show => {
            call(c, "subscribe", json!({})).await?;
            let v = call(c, "pair.start", json!({})).await?;
            if j {
                println!("{}", serde_json::to_string(&v).unwrap_or_default());
            } else {
                println!("Both devices must be on the same local network.");
                if let Some(n) = v["notice"].as_str() {
                    println!("{n}");
                }
                if let Some(q) = v["qr"].as_str().and_then(qr_text) {
                    println!("Scan this with a phone or tablet:\n{q}");
                }
                if let Some(code) = v["code"].as_str() {
                    println!("Or type this code on the other device:  {code}");
                }
                println!(
                    "The code works for 10 minutes. Waiting for the other device (Ctrl+C stops)..."
                );
            }
            let ctrl_c = tokio::signal::ctrl_c();
            tokio::pin!(ctrl_c);
            loop {
                tokio::select! {
                    e = c.next_event() => {
                        let Some(e) = e else {
                            return Err(Fail("The agent stopped.".into(), 2));
                        };
                        match e.event.as_str() {
                            "pair.added" => {
                                if j {
                                    println!("{}", serde_json::to_string(&e).unwrap_or_default());
                                } else {
                                    println!("{} is added.", e.data["name"].as_str().unwrap_or("The device"));
                                }
                                return Ok(());
                            }
                            "pair.tried" => {
                                if j {
                                    println!("{}", serde_json::to_string(&e).unwrap_or_default());
                                } else {
                                    println!("{}.", e.data["notice"].as_str().unwrap_or(""));
                                    if let Some(l) = e.data["lockout"].as_str() {
                                        println!("{l}");
                                    }
                                    if e.data["code_dead"].as_bool() == Some(true) {
                                        println!("The code no longer works. The QR code still works.");
                                    }
                                }
                            }
                            "pair.closed" => {
                                return Err(Fail(
                                    if e.data["reason"] == "expired" {
                                        "The code ran out. Run \"uc pair show\" again.".into()
                                    } else {
                                        "The code was closed.".into()
                                    },
                                    1,
                                ));
                            }
                            _ => {}
                        }
                    }
                    _ = &mut ctrl_c => {
                        let _ = c.call("pair.cancel", json!({})).await;
                        return Err(Fail("Stopped. The code is closed.".into(), 130));
                    }
                }
            }
        }
    }
    Ok(())
}

async fn setup(c: &mut Client, p: &Profile, j: bool, all_yes: bool) -> Result<(), Fail> {
    let v = call(c, "setup.status", json!({})).await?;
    if !j {
        println!("Copy on one device, paste on another.");
        println!("Privacy: https://maxaubert.github.io/Universal-Clipboard/privacy");
    }
    for row in v["rows"].as_array().cloned().unwrap_or_default() {
        let id = row["row"].as_str().unwrap_or("");
        let title = row["title"].as_str().unwrap_or("");
        let state = row["state"].as_str().unwrap_or("");
        if !j {
            println!("\n{title}\n  Why: {}", row["why"].as_str().unwrap_or(""));
        }
        if state == "done" || id == "desktop" {
            if !j {
                println!(
                    "  {}",
                    if state == "done" {
                        "Done."
                    } else {
                        "Not possible here."
                    }
                );
            }
            continue;
        }
        let answer = if all_yes {
            true
        } else {
            match yes_no(&format!("  {title}?")) {
                Some(a) => a,
                None => return Err(Fail("Add --yes to answer yes to every step.".into(), 2)),
            }
        };
        let answer = if id == "firewall" && answer {
            firewall_rule(p, j)
        } else {
            answer
        };
        call(c, "setup.step", json!({"row": id, "answer": answer})).await?;
        if !j {
            if answer {
                println!("  Done.");
            } else if let Some(r) = row["if_refused"].as_str() {
                println!("  {r}");
            }
        }
    }
    // macOS app bundle: put "uc" (or "uclip") on PATH. The installers do this on Windows and
    // Linux; a dragged app has no install step.
    if cfg!(target_os = "macos")
        && !p.is_test()
        && let Ok(me) = std::env::current_exe()
        && me.to_string_lossy().contains(".app/Contents/MacOS/")
    {
        let o = std::process::Command::new(me.with_file_name("uc-agent"))
            .arg("--install-hook")
            .output();
        if let Ok(o) = o
            && !j
        {
            print!("\n{}", String::from_utf8_lossy(&o.stdout));
        }
    }
    let v = call(c, "setup.step", json!({"row": "done", "answer": true})).await?;
    out(j, &v, || {
        "\nSetup is done. Next: \"uc pair show\" on this device, or \"uc pair enter\" with a code from another device.".into()
    });
    Ok(())
}

/// "Let your devices find this PC": add the firewall rule with the one-time helper (one UAC
/// prompt). The Store build has the rule in its package, and test profiles never elevate, so
/// both skip the helper. A "No" in the UAC prompt counts as "no" (outbound-only mode).
fn firewall_rule(p: &Profile, j: bool) -> bool {
    if p.is_test() || !uc_agent::firewall::needs_helper() {
        return true;
    }
    let agent = std::env::current_exe()
        .map(|e| {
            e.with_file_name(if cfg!(windows) {
                "uc-agent.exe"
            } else {
                "uc-agent"
            })
        })
        .unwrap_or_default();
    if !j {
        println!("  Windows asks for permission once, to add the firewall rule.");
    }
    let st = std::process::Command::new(&agent)
        .arg("--firewall")
        .arg("add")
        .arg("--exe")
        .arg(&agent)
        .status();
    matches!(st, Ok(s) if s.success())
}

fn print_help() {
    println!(
        "uc: Universal Clipboard. Copy on one device, paste on another.

  uc status                      Working, Paused or Needs attention
  uc devices                     your devices
  uc rename <device> <name>      give a device a new name
  uc remove <device> [--yes]     remove a device
  uc pair show                   show a code and a QR code for a new device
  uc pair enter [code]           enter the code of another device
  uc pair cancel                 close the code
  uc pause [--for 1h]            pause sync
  uc resume                      turn sync on again
  uc settings                    show all settings
  uc settings set <key> <value>  change a setting
  uc diagnostics                 report for help (no clipboard content, no keys)
  uc setup                       first-run steps
  uc leave [--yes]               remove this device and forget everything
  uc --version

Add --json to any command for scripts.
There is no send or get: the latest copy is on every device on its own.
Privacy: https://maxaubert.github.io/Universal-Clipboard/privacy"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plan 7, M7 accept: every action of the settings window works from `uc`.
    #[test]
    fn every_window_action_has_a_cli_command() {
        for (action, argv, ipc) in UI_ACTIONS {
            if argv.contains(&"--version") {
                let e = Cli::try_parse_from(*argv).unwrap_err();
                assert_eq!(e.kind(), clap::error::ErrorKind::DisplayVersion, "{action}");
                continue;
            }
            let cli = Cli::try_parse_from(*argv)
                .unwrap_or_else(|e| panic!("{action}: {argv:?} does not parse: {e}"));
            let cmd = cli.cmd.expect("a command");
            assert!(
                cmd.calls().contains(ipc),
                "{action}: {argv:?} must call {ipc}, calls {:?}",
                cmd.calls()
            );
            for c in cmd.calls() {
                assert!(
                    uc_agent::ipc::CALLS.contains(&c),
                    "{c} is not an agent call"
                );
            }
        }
    }

    /// Every IPC call of plan 3.2 is reachable from some command, and every settings key
    /// is in the table.
    #[test]
    fn every_ipc_call_and_setting_is_covered() {
        let used: Vec<&str> = UI_ACTIONS.iter().map(|(_, _, c)| *c).collect();
        for c in uc_agent::ipc::CALLS {
            if matches!(*c, "subscribe" | "version") {
                continue;
            }
            let reachable = used.contains(c)
                || UI_ACTIONS.iter().any(|(_, argv, _)| {
                    Cli::try_parse_from(*argv)
                        .ok()
                        .and_then(|x| x.cmd)
                        .is_some_and(|cmd| cmd.calls().contains(c))
                });
            assert!(reachable, "IPC call {c} has no uc command");
        }
        for k in uc_agent::settings::KEYS {
            assert!(
                UI_ACTIONS.iter().any(|(_, argv, _)| argv.contains(&k.name)),
                "setting {} has no row",
                k.name
            );
        }
    }

    #[test]
    fn json_flag_works_on_every_command() {
        for (_, argv, _) in UI_ACTIONS {
            if argv.contains(&"--version") {
                continue;
            }
            let mut a: Vec<&str> = argv.to_vec();
            a.push("--json");
            assert!(Cli::try_parse_from(&a).unwrap().json, "{a:?}");
        }
    }

    #[test]
    fn there_is_no_send_or_get() {
        for bad in ["send", "get", "fetch", "push", "pull"] {
            assert!(Cli::try_parse_from(["uc", bad]).is_err(), "{bad}");
        }
    }

    #[test]
    fn times_and_qr() {
        assert_eq!(parse_minutes("1h"), Some(60));
        assert_eq!(parse_minutes("30m"), Some(30));
        assert_eq!(parse_minutes("15"), Some(15));
        assert_eq!(parse_minutes("x"), None);
        let q = qr_text("UCLIP1:AAAA").unwrap();
        assert!(q.lines().count() > 10);
    }
}
