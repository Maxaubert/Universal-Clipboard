//! The settings window of Universal Clipboard (plan 3.2, M8b).
//!
//! The window holds no keys and no clipboard content. It only proxies the agent's IPC calls:
//! `ipc_call` forwards one JSON call to the local pipe or socket (the same client as the `uc`
//! CLI, with the same peer checks) and a background task forwards the agent's events to the
//! page as `uc-event`. If no agent answers, the window starts it, like `uc` does.
//!
//! The window opens when the user starts the app, and once when the agent found a newer
//! version at its start (decision 2026-11-14): the agent starts it with `--route update`.
//! Closing it ends this process and leaves the agent running.
//!
//! "Copy files..." and "Copy folder..." on Home open the system file picker here
//! (`pick_paths`, tauri-plugin-dialog, called from Rust only, so the page gets no file access);
//! the page then sends the picked paths with `files.copy`.
//!
//! The tray icon and its panel (0.3.0) live in this process too: see [`tray`]. With the icon
//! on, the process starts with `--tray` (no window) and closing the window does not end it.

pub mod tray;

use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, RunEvent};
use uc_agent::ipc::Client;
use uc_agent::profile::Profile;

/// Calls the window may make (plan 3.2). There is no send, get or fetch, and the test-only
/// `debug.*` calls are not here.
pub const ALLOWED: &[&str] = &[
    "status",
    "subscribe",
    "pair.start",
    "pair.join",
    "pair.cancel",
    "devices.list",
    "devices.rename",
    "devices.remove",
    "pause.set",
    "settings.get",
    "settings.set",
    "files.copy",
    "transfer.stop",
    "diagnostics",
    "leave",
    "setup.status",
    "setup.step",
    "firewall.check",
    "version",
    "update.status",
    "update.apply",
    "update.later",
    "quit",
];

pub fn is_allowed(call: &str) -> bool {
    ALLOWED.contains(&call)
}

#[derive(Serialize)]
pub struct Failure {
    code: String,
    message: String,
}

fn fail(code: &str, message: impl Into<String>) -> Failure {
    Failure {
        code: code.into(),
        message: message.into(),
    }
}

pub(crate) fn profile() -> Result<Profile, Failure> {
    Profile::from_env().map_err(|_| fail("bad-profile", "UCLIP_PROFILE is not a valid name."))
}

#[tauri::command]
async fn ipc_call(call: String, params: Value) -> Result<Value, Failure> {
    agent_call(call, params).await
}

/// One allowed call to the agent, starting it if none answers (like `uc`).
pub(crate) async fn agent_call(call: String, params: Value) -> Result<Value, Failure> {
    if !is_allowed(&call) {
        return Err(fail("unknown-call", format!("Unknown call \"{call}\".")));
    }
    let p = profile()?;
    let mut c = uc_cli::connect(&p)
        .await
        .map_err(|e| fail("agent-down", e.0))?;
    match c.call(&call, params).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(Failure {
            code: e.code,
            message: e.message,
        }),
        Err(e) => Err(fail("agent-down", format!("Cannot talk to the agent: {e}"))),
    }
}

/// The window's Fix for a missing Windows firewall rule: the same path as `uc firewall add`
/// (the one-time helper, one UAC prompt), then `firewall.check` so the agent looks again.
#[tauri::command]
async fn firewall_add() -> Result<Value, Failure> {
    let p = profile()?;
    let p2 = p.clone();
    let code = tauri::async_runtime::spawn_blocking(move || uc_cli::add_firewall_rule(&p2))
        .await
        .unwrap_or(1);
    match code {
        0 => {}
        3 => {
            return Err(fail(
                "refused",
                "You said no. This PC only connects out; sync works when the other device accepts connections.",
            ));
        }
        _ => return Err(fail("firewall", "The firewall rule could not be added.")),
    }
    agent_call("firewall.check".into(), json!({})).await
}

/// "Copy files..." (`folder` false: one or more files) and "Copy folder..." (`folder` true): the
/// system picker. Windows cannot pick files and folders in one dialog, so these are two. Returns
/// the picked paths, absolute, or none when the user cancelled.
#[tauri::command]
async fn pick_paths(app: AppHandle, folder: bool) -> Vec<String> {
    pick(&app, folder, true).await
}

/// The system picker. `parent`: attach it to the main window when there is one (the tray's
/// picker has none).
pub(crate) async fn pick(app: &AppHandle, folder: bool, parent: bool) -> Vec<String> {
    use tauri_plugin_dialog::{DialogExt, FilePath};
    let (tx, rx) = tokio::sync::oneshot::channel::<Vec<FilePath>>();
    let mut d = app.dialog().file();
    if parent && let Some(w) = app.get_webview_window("main") {
        d = d.set_parent(&w);
    }
    if folder {
        d.set_title("Copy folder")
            .pick_folder(move |p| drop(tx.send(p.into_iter().collect())));
    } else {
        d.set_title("Copy files")
            .pick_files(move |p| drop(tx.send(p.unwrap_or_default())));
    }
    picked_paths(rx.await.unwrap_or_default())
}

/// The picked items as path strings; anything that is not a local path (a mobile content URI)
/// is left out.
pub fn picked_paths(picked: Vec<tauri_plugin_dialog::FilePath>) -> Vec<String> {
    picked
        .into_iter()
        .filter_map(|p| p.into_path().ok())
        .filter(|p| p.is_absolute())
        .map(|p| p.display().to_string())
        .collect()
}

/// The screen the agent asked for with `--route <name>`. Only `update` is known.
pub fn route_arg(args: &[String]) -> Option<String> {
    let i = args.iter().position(|a| a == "--route")?;
    args.get(i + 1).filter(|r| *r == "update").cloned()
}

/// The screen this window was opened for: from the tray (Settings, VPN help), or
/// `--route update` from the agent (put there once at start). Read once: a later window of the
/// same process (the tray keeps it alive) opens on Home.
#[tauri::command]
fn launch_route(app: AppHandle) -> Option<String> {
    app.state::<tray::Tray>()
        .pending_route
        .lock()
        .unwrap()
        .take()
}

/// Calls the tray panel may make. They never start the agent: a stopped agent shows as
/// "Not running" (tray spec 3.4).
pub const PANEL_CALLS: &[&str] = &["status", "devices.list", "pause.set", "transfer.stop"];

#[tauri::command]
async fn panel_call(call: String, params: Value) -> Result<Value, Failure> {
    if !PANEL_CALLS.contains(&call.as_str()) {
        return Err(fail("unknown-call", format!("Unknown call \"{call}\".")));
    }
    let p = profile()?;
    let mut c = Client::connect(&p)
        .await
        .map_err(|_| fail("agent-down", "Universal Clipboard is not running."))?;
    match c.call(&call, params).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(Failure {
            code: e.code,
            message: e.message,
        }),
        Err(e) => Err(fail("agent-down", format!("Cannot talk to the agent: {e}"))),
    }
}

/// Bring the window up. `ask`: the agent opened it for an update: put it in the middle of the
/// screen that has the mouse, and above other windows once (without staying on top).
fn present(app: &AppHandle, ask: bool) {
    let Some(w) = app.get_webview_window("main") else {
        return;
    };
    if ask
        && let (Ok(c), Ok(size)) = (app.cursor_position(), w.outer_size())
        && let Ok(Some(m)) = app.monitor_from_point(c.x, c.y)
    {
        let (mp, ms) = (m.position(), m.size());
        let x = mp.x + (ms.width.saturating_sub(size.width) / 2) as i32;
        let y = mp.y + (ms.height.saturating_sub(size.height) / 2) as i32;
        let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
    }
    let _ = w.unminimize();
    let _ = w.show();
    let _ = w.set_focus();
    if ask {
        // Windows may refuse the focus to a window that a background process started: raise
        // it above the others once, then let it be a normal window again.
        let _ = w.set_always_on_top(true);
        let _ = w.set_always_on_top(false);
        let _ = w.request_user_attention(Some(tauri::UserAttentionType::Informational));
    }
}

/// One subscription to the agent (it never starts the agent): forwards its events to the
/// pages, keeps the tray icon right, shows or removes the icon when `tray-icon` changes, and
/// ends this process when the agent quits. Reconnects when the agent restarts.
async fn agent_loop(app: tauri::AppHandle) {
    let Ok(p) = profile() else { return };
    let mut misses = 0u32;
    loop {
        // Started with `--tray` but no agent answers for a minute and nothing is on screen:
        // do not stay as an invisible process.
        if misses >= 30
            && app.webview_windows().is_empty()
            && !app
                .state::<tray::Tray>()
                .shown
                .load(std::sync::atomic::Ordering::SeqCst)
        {
            app.exit(0);
            return;
        }
        if let Ok(mut c) = Client::connect(&p).await
            && matches!(c.call("subscribe", json!({})).await, Ok(Ok(_)))
        {
            // (Re)connected, for example to the new agent after an update: the page checks
            // again, so it leaves the "agent is down" screen by itself.
            misses = 0;
            let _ = app.emit("uc-event", json!({"event": "status", "data": {}}));
            refresh_tray(&app, &p, true).await;
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
            tick.tick().await;
            loop {
                tokio::select! {
                    e = c.next_event() => {
                        let Some(e) = e else { break };
                        // Before the pages hear of it: an open page would call `status`, which
                        // starts the agent again.
                        if e.event == "quitting" {
                            app.exit(0);
                            return;
                        }
                        let _ = app.emit("uc-event", json!({"event": e.event, "data": e.data}));
                        match e.event.as_str() {
                            "transfer" => tray::set_transfer(&app, &e.data),
                            // macOS: the agent's `open -a ... --args --route update` reaches a
                            // running app only as a Reopen, without the route. The update event
                            // carries it here instead.
                            #[cfg(target_os = "macos")]
                            "update" if app.get_webview_window("main").is_none()
                                && matches!(e.data["state"].as_str(), Some("available" | "failed" | "updated"))
                                && e.data["dismissed"] != true =>
                            {
                                tray::open_window(&app, Some("update".into()));
                                present(&app, true);
                            }
                            "settings" if e.data["key"] == "tray-icon" => {
                                if e.data["value"] == "on" {
                                    tray::show(&app);
                                } else {
                                    tray::hide(&app);
                                }
                            }
                            _ => refresh_tray(&app, &p, false).await,
                        }
                    }
                    _ = tick.tick() => refresh_tray(&app, &p, false).await,
                }
            }
            // The agent went away: tell the page so it checks again.
            let _ = app.emit("uc-event", json!({"event": "status", "data": {}}));
        }
        misses += 1;
        tray::set_status(&app, None);
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// Read the status (and with `settings`, whether the icon is on) and update the icon.
async fn refresh_tray(app: &AppHandle, p: &Profile, settings: bool) {
    let Ok(mut c) = Client::connect(p).await else {
        tray::set_status(app, None);
        return;
    };
    if settings && let Ok(Ok(s)) = c.call("settings.get", json!({})).await {
        if s["tray-icon"]["value"] == "on" {
            tray::show(app);
        } else {
            tray::hide(app);
        }
    }
    match c.call("status", json!({})).await {
        Ok(Ok(s)) => tray::set_status(app, Some(&s)),
        _ => tray::set_status(app, None),
    }
}

pub fn run() {
    let app = tauri::Builder::default()
        // A second launch goes to this process: `--tray` (the agent at its start) changes
        // nothing, any other start opens the window. The agent's `--route update` also moves
        // the page to the update screen.
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if tray::tray_arg(&args) {
                return;
            }
            let route = route_arg(&args);
            // Not in this callback: building a webview here deadlocks on Windows.
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if app.get_webview_window("main").is_none() {
                    tray::open_window(&app, route.clone());
                } else if let Some(r) = &route {
                    let _ = app.emit("uc-event", json!({"event": "open", "data": {"route": r}}));
                }
                present(&app, route.is_some());
            });
        }))
        .manage(tray::Tray::default())
        .setup(|app| {
            let args: Vec<String> = std::env::args().collect();
            // `--tray`: the icon only. Otherwise the window, as before.
            #[cfg(target_os = "macos")]
            if tray::tray_arg(&args) {
                tray::dock_icon(app.handle(), false);
            }
            if !tray::tray_arg(&args) {
                tray::open_window(app.handle(), route_arg(&args));
                if route_arg(&args).is_some() {
                    present(app.handle(), true);
                }
            }
            tauri::async_runtime::spawn(agent_loop(app.handle().clone()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc_call,
            firewall_add,
            launch_route,
            panel_call,
            pick_paths,
            tray::panel_ready,
            tray::tray_copy,
            tray::tray_open,
            tray::tray_quit
        ])
        .build(tauri::generate_context!())
        .expect("error while building the Universal Clipboard window");
    app.run(|app, e| {
        // macOS: opening the app while it runs (the tray, no window) is a Reopen, not a new
        // process: open the window.
        #[cfg(target_os = "macos")]
        if let RunEvent::Reopen { .. } = e {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if app.get_webview_window("main").is_none() {
                    tray::open_window(&app, None);
                }
                present(&app, false);
            });
            return;
        }
        // macOS: the window closed, so the Dock icon goes; the menu bar icon stays.
        #[cfg(target_os = "macos")]
        if let RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::Destroyed,
            ..
        } = &e
            && label == "main"
        {
            tray::dock_icon(app, false);
        }
        // With the icon shown, closing the last window (the window or the panel) keeps the
        // process. Quit, "icon off" and the agent quitting exit with a code.
        if let RunEvent::ExitRequested {
            code: None, api, ..
        } = e
            && app
                .state::<tray::Tray>()
                .shown
                .load(std::sync::atomic::Ordering::SeqCst)
        {
            api.prevent_exit();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plan_calls_pass() {
        for c in ALLOWED {
            assert!(is_allowed(c));
        }
        for bad in [
            "debug.copy",
            "debug.clipboard",
            "send",
            "get",
            "fetch",
            "clipboard.get",
        ] {
            assert!(!is_allowed(bad), "{bad}");
        }
        assert!(
            !ALLOWED
                .iter()
                .any(|c| c.contains("send") || c.contains("fetch"))
        );
    }

    #[test]
    fn only_the_update_route_is_taken() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            route_arg(&a(&["uc", "--route", "update"])).as_deref(),
            Some("update")
        );
        assert_eq!(route_arg(&a(&["uc", "--route", "settings"])), None);
        assert_eq!(route_arg(&a(&["uc", "--route"])), None);
        assert_eq!(route_arg(&a(&["uc"])), None);
    }

    #[test]
    fn the_window_can_copy_files_and_stop_a_transfer() {
        assert!(is_allowed("files.copy"));
        assert!(is_allowed("transfer.stop"));
        assert!(!is_allowed("debug.copy_files"));
    }

    #[test]
    fn only_local_absolute_paths_are_picked() {
        use tauri_plugin_dialog::FilePath;
        let abs = std::env::temp_dir();
        let got = picked_paths(vec![
            FilePath::Path(abs.clone()),
            FilePath::Path(std::path::PathBuf::from("relative")),
        ]);
        assert_eq!(got, vec![abs.display().to_string()]);
    }

    #[test]
    fn list_matches_the_agent() {
        for c in ALLOWED {
            assert!(uc_agent::ipc::CALLS.contains(c), "{c} is not an agent call");
        }
        assert_eq!(ALLOWED.len(), uc_agent::ipc::CALLS.len());
    }
}
