//! The settings window of Universal Clipboard (plan 3.2, M8b).
//!
//! The window holds no keys and no clipboard content. It only proxies the agent's IPC calls:
//! `ipc_call` forwards one JSON call to the local pipe or socket (the same client as the `uc`
//! CLI, with the same peer checks) and a background task forwards the agent's events to the
//! page as `uc-event`. If no agent answers, the window starts it, like `uc` does.
//!
//! The window opens only when the user starts the app. Closing it ends this process and
//! leaves the agent running.

use serde::Serialize;
use serde_json::{Value, json};
use tauri::{Emitter, Manager};
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
    "version",
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
        // A second launch only brings the first window to the front.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .setup(|app| {
            tauri::async_runtime::spawn(event_loop(app.handle().clone()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![ipc_call])
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
    fn list_matches_the_agent() {
        for c in ALLOWED {
            assert!(uc_agent::ipc::CALLS.contains(c), "{c} is not an agent call");
        }
        assert_eq!(ALLOWED.len(), uc_agent::ipc::CALLS.len());
    }
}
