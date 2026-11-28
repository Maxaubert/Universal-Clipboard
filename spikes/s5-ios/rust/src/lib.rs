//! Spike S5 (throwaway): the desktop agent core, run inside an iOS process.
//! Swift calls `start` once, then `call` with IPC methods (the same JSON the `uc` CLI sends).

use std::time::Instant;

uniffi::setup_scaffolding!();

static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// Start the agent core on its own thread with a test profile and state folder.
/// `file_keys`: use the file key store instead of the Keychain.
#[uniffi::export]
pub fn start(profile: String, state_dir: String, file_keys: bool) -> String {
    START.get_or_init(Instant::now);
    // SAFETY-free: env is set before the agent thread starts, from one thread.
    unsafe_env(&profile, &state_dir, file_keys);
    let p = match uc_agent::profile::Profile::from_env() {
        Ok(p) => p,
        Err(e) => return format!("profile: {e}"),
    };
    std::thread::Builder::new()
        .name("uc-agent".into())
        .spawn(move || {
            if let Err(e) = uc_agent::agent::run(p) {
                eprintln!("agent stopped: {}", e.0);
            }
        })
        .map(|_| String::new())
        .unwrap_or_else(|e| format!("thread: {e}"))
}

#[allow(unsafe_code)]
fn unsafe_env(profile: &str, dir: &str, file_keys: bool) {
    // The spike sets the test switches the agent reads. Called before any other thread reads env.
    unsafe {
        std::env::set_var("UCLIP_PROFILE", profile);
        std::env::set_var("UCLIP_STATE_DIR", dir);
        std::env::set_var("UCLIP_TEST_FAST", "1");
        std::env::set_var("UCLIP_LOG", "debug");
        if file_keys {
            std::env::set_var("UCLIP_TEST_KEYSTORE", "file");
        }
    }
}

/// Milliseconds since `start`.
#[uniffi::export]
pub fn elapsed_ms() -> u64 {
    START.get().map(|t| t.elapsed().as_millis() as u64).unwrap_or(0)
}

/// One IPC call. Returns `{"ok":...}` or `{"error":"..."}`.
#[uniffi::export]
pub fn call(method: String, params_json: String) -> String {
    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(r) => r,
        Err(e) => return serde_json::json!({"error": e.to_string()}).to_string(),
    };
    let params: serde_json::Value =
        serde_json::from_str(&params_json).unwrap_or(serde_json::json!({}));
    rt.block_on(async {
        let p = match uc_agent::profile::Profile::from_env() {
            Ok(p) => p,
            Err(e) => return serde_json::json!({"error": e.to_string()}),
        };
        let mut c = match uc_agent::ipc::Client::connect(&p).await {
            Ok(c) => c,
            Err(e) => return serde_json::json!({"error": format!("connect: {e}")}),
        };
        match c.call(&method, params).await {
            Ok(Ok(v)) => serde_json::json!({"ok": v}),
            Ok(Err(e)) => serde_json::json!({"error": e.message, "code": e.code}),
            Err(e) => serde_json::json!({"error": e.to_string()}),
        }
    })
    .to_string()
}
