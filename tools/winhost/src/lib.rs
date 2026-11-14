//! Run a process inside a private Windows window station and desktop (spike S3).
//!
//! A window station has its own clipboard. A station made with a NULL name is not the
//! user's `WinSta0` and is never visible, so a process started on it cannot touch the user's
//! clipboard or screen. Tests use [`isolated_test`]: the test re-runs itself inside that
//! station and checks the result.
//!
//! Windows names a NULL-name station after the logon session (`Service-0x0-<LUID>$`), so all
//! runs in one session share it and its clipboard. [`run`] therefore holds a session-wide
//! named mutex while the child runs: one child at a time, even across parallel test
//! processes. A child must not assume the clipboard is empty when it starts.
//!
//! On other OSes this crate is empty.

#[cfg(windows)]
mod host;

#[cfg(windows)]
pub use host::{
    CHILD_ENV, Output, STATION_ENV, current_station, host_sequence_number, isolated_test,
    require_isolated, run,
};
