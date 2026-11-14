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

use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};
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
    "diagnostics",
    "leave",
    "setup.status",
    "setup.step",
    "firewall.check",
    "version",
    "update.status",
    "update.apply",
    "update.later",
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

fn profile() -> Result<Profile, Failure> {
    Profile::from_env().map_err(|_| fail("bad-profile", "UCLIP_PROFILE is not a valid name."))
}

#[tauri::command]
async fn ipc_call(call: String, params: Value) -> Result<Value, Failure> {
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
    ipc_call("firewall.check".into(), json!({})).await
}

/// The screen the agent asked for with `--route <name>`. Only `update` is known.
pub fn route_arg(args: &[String]) -> Option<String> {
    let i = args.iter().position(|a| a == "--route")?;
    args.get(i + 1).filter(|r| *r == "update").cloned()
}

/// The screen this process was started for (`--route update` from the agent), or none.
#[tauri::command]
fn launch_route() -> Option<String> {
    route_arg(&std::env::args().collect::<Vec<_>>())
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

/// Forward the agent's events to the page. Reconnects when the agent restarts.
async fn event_loop(app: tauri::AppHandle) {
    let Ok(p) = profile() else { return };
    loop {
        if let Ok(mut c) = Client::connect(&p).await
            && matches!(c.call("subscribe", json!({})).await, Ok(Ok(_)))
        {
            // (Re)connected, for example to the new agent after an update: the page checks
            // again, so it leaves the "agent is down" screen by itself.
            let _ = app.emit("uc-event", json!({"event": "status", "data": {}}));
            while let Some(e) = c.next_event().await {
                let _ = app.emit("uc-event", json!({"event": e.event, "data": e.data}));
            }
            // The agent went away: tell the page so it checks again.
            let _ = app.emit("uc-event", json!({"event": "status", "data": {}}));
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

pub fn run() {
    tauri::Builder::default()
        // A second launch only brings the first window to the front. The agent's
        // `--route update` also moves the page to the update screen.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            let route = route_arg(&args);
            if let Some(r) = &route {
                let _ = app.emit("uc-event", json!({"event": "open", "data": {"route": r}}));
            }
            present(app, route.is_some());
        }))
        .setup(|app| {
            if launch_route().is_some() {
                present(app.handle(), true);
            }
            tauri::async_runtime::spawn(event_loop(app.handle().clone()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc_call,
            firewall_add,
            launch_route
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Universal Clipboard window");
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
    fn list_matches_the_agent() {
        for c in ALLOWED {
            assert!(uc_agent::ipc::CALLS.contains(c), "{c} is not an agent call");
        }
        assert_eq!(ALLOWED.len(), uc_agent::ipc::CALLS.len());
    }
}
