//! The headless background program (plan 3.1, 3.2, milestone M7) and the IPC that the `uc`
//! CLI and the settings window use.
//!
//! | Module | What |
//! |---|---|
//! | [`agent`] | the core: engine, network, pairing, membership, key store, clipboard |
//! | [`ipc`] | JSON-lines IPC: named pipe (Windows) or Unix socket, hardened (3.2) |
//! | [`profile`] | `UCLIP_PROFILE`, state folder, IPC name, port, test switches |
//! | [`state`] | `state.json` (never content, never keys) |
//! | [`settings`] | the settings of the window, as keys |
//! | [`autostart`] | start at login per OS, sandboxed in test profiles |
//! | [`clip`] | the clipboard backend, isolated in test profiles |
//! | [`pairing`] | the pairing stream I/O |
//! | [`supervise`] | `uc-agent --supervise`: restart after a crash (Linux) |
//! | [`log`] | rotating log without content |

pub mod agent;
pub mod autostart;
pub mod clip;
pub mod ipc;
pub mod log;
pub mod pairing;
pub mod profile;
pub mod settings;
pub mod state;
pub mod supervise;
#[cfg(windows)]
pub mod win;
