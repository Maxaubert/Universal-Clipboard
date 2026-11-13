//! Linux clipboard backends (plan 3.9, milestone M6).
//!
//! - **Wayland** (KDE Plasma 6, Sway, Hyprland, other wlroots, COSMIC): the data-control
//!   protocol, `ext-data-control-v1` first, else `wlr-data-control-unstable-v1`.
//! - **X11** (X11 sessions, and XWayland): XFIXES selection-owner events on `CLIPBOARD`. We
//!   serve our own writes as the selection owner.
//! - **GNOME on Wayland**: the X11 path through XWayland, marked **experimental**. Spike S4 on a
//!   real GNOME machine is still open, so we do not claim GNOME support.
//!
//! Only `CLIPBOARD` is synced, never the primary selection. Formats: UTF-8 text, `text/html`,
//! `image/png`. Private hints (`x-kde-passwordManagerHint=secret`) and file clips are
//! reported, not read. Our writes carry [`mime::MARKER`].
//!
//! The pure parts ([`mime`], [`session`], [`autostart`]) build on every OS. The backends build
//! on Linux only.

pub mod autostart;
pub mod mime;
pub mod session;

#[cfg(target_os = "linux")]
mod backend;
#[cfg(target_os = "linux")]
mod wayland;
#[cfg(target_os = "linux")]
mod x11;

#[cfg(target_os = "linux")]
pub use backend::{DataControl, LinuxClipboard, OpenError};

/// Which backend is in use, for the diagnostic report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendKind {
    /// Wayland `ext-data-control-v1`.
    ExtDataControl,
    /// Wayland `wlr-data-control-unstable-v1`.
    WlrDataControl,
    /// X11 in an X11 session.
    X11,
    /// X11 through XWayland in a Wayland session that offers no data-control (GNOME).
    /// Experimental: not proven to see every copy (spike S4).
    XWayland,
}

impl BackendKind {
    /// Not proven on a real desktop yet. The app must not claim support.
    pub fn is_experimental(self) -> bool {
        self == BackendKind::XWayland
    }

    /// A short name for logs.
    pub fn name(self) -> &'static str {
        match self {
            BackendKind::ExtDataControl => "ext-data-control-v1",
            BackendKind::WlrDataControl => "wlr-data-control-unstable-v1",
            BackendKind::X11 => "x11",
            BackendKind::XWayland => "xwayland (experimental)",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::BackendKind;

    #[test]
    fn only_xwayland_is_experimental() {
        assert!(BackendKind::XWayland.is_experimental());
        assert!(!BackendKind::X11.is_experimental());
        assert!(!BackendKind::ExtDataControl.is_experimental());
        assert!(BackendKind::XWayland.name().contains("experimental"));
    }
}
