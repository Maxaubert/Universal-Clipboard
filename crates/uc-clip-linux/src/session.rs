//! Session detection: which clipboard backends to try, in which order.
//!
//! Pure logic over environment variables, so it is tested on every OS.
//!
//! | Session | Order |
//! |---|---|
//! | Wayland (KDE, Sway, wlroots, COSMIC, ...) | data-control (ext, then wlr), then X11 through XWayland |
//! | Wayland, GNOME | the same; data-control is not offered by GNOME today, so the XWayland path is used, **experimental** (spike S4 is still open) |
//! | X11 | X11 |
//! | none | nothing: "This desktop blocks background clipboard access." |

/// One backend to try.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Candidate {
    /// Wayland data-control (`ext-data-control-v1`, else `wlr-data-control-unstable-v1`).
    DataControl,
    /// X11 with XFIXES. `xwayland` is true inside a Wayland session.
    X11 {
        /// The X server is XWayland inside a Wayland session.
        xwayland: bool,
    },
}

/// What the environment says about the session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detection {
    /// `WAYLAND_DISPLAY` is set, or `XDG_SESSION_TYPE` is `wayland`.
    pub wayland: bool,
    /// `DISPLAY` is set.
    pub x11: bool,
    /// The desktop is GNOME (from `XDG_CURRENT_DESKTOP`, for example `ubuntu:GNOME`).
    pub gnome: bool,
    /// The desktop names, for the diagnostic report.
    pub desktop: String,
    /// Backends to try, in order.
    pub candidates: Vec<Candidate>,
}

impl Detection {
    /// The X11 path inside a Wayland session that does not offer data-control is not proven.
    /// On GNOME this waits for spike S4 on a real GNOME machine. Do not claim support.
    pub fn xwayland_is_experimental(&self) -> bool {
        self.wayland
    }
}

/// Detect from a lookup function (for tests).
pub fn detect<F: Fn(&str) -> Option<String>>(env: F) -> Detection {
    let set = |k: &str| env(k).is_some_and(|v| !v.trim().is_empty());
    let session_type = env("XDG_SESSION_TYPE").unwrap_or_default();
    let wayland = set("WAYLAND_DISPLAY") || session_type.eq_ignore_ascii_case("wayland");
    let x11 = set("DISPLAY");
    let desktop = env("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let gnome = desktop
        .split(':')
        .any(|d| d.trim().eq_ignore_ascii_case("GNOME"));
    let mut candidates = Vec::new();
    if wayland {
        candidates.push(Candidate::DataControl);
        if x11 {
            candidates.push(Candidate::X11 { xwayland: true });
        }
    } else if x11 {
        candidates.push(Candidate::X11 { xwayland: false });
    }
    Detection {
        wayland,
        x11,
        gnome,
        desktop,
        candidates,
    }
}

/// Detect from the process environment.
pub fn detect_from_env() -> Detection {
    detect(|k| std::env::var(k).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn kde_wayland_tries_data_control_then_xwayland() {
        let d = detect(env(&[
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("DISPLAY", ":1"),
            ("XDG_CURRENT_DESKTOP", "KDE"),
        ]));
        assert!(d.wayland && d.x11 && !d.gnome);
        assert_eq!(
            d.candidates,
            vec![Candidate::DataControl, Candidate::X11 { xwayland: true }]
        );
        assert!(d.xwayland_is_experimental());
    }

    #[test]
    fn gnome_wayland_is_detected() {
        let d = detect(env(&[
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("DISPLAY", ":0"),
            ("XDG_CURRENT_DESKTOP", "ubuntu:GNOME"),
        ]));
        assert!(d.gnome);
        assert_eq!(
            d.candidates.last(),
            Some(&Candidate::X11 { xwayland: true })
        );
    }

    #[test]
    fn sway_without_xwayland_has_only_data_control() {
        let d = detect(env(&[
            ("WAYLAND_DISPLAY", "wayland-1"),
            ("XDG_CURRENT_DESKTOP", "sway"),
        ]));
        assert_eq!(d.candidates, vec![Candidate::DataControl]);
    }

    #[test]
    fn session_type_alone_means_wayland() {
        let d = detect(env(&[("XDG_SESSION_TYPE", "wayland")]));
        assert!(d.wayland);
    }

    #[test]
    fn x11_session() {
        let d = detect(env(&[
            ("DISPLAY", ":0"),
            ("XDG_SESSION_TYPE", "x11"),
            ("XDG_CURRENT_DESKTOP", "XFCE"),
        ]));
        assert_eq!(d.candidates, vec![Candidate::X11 { xwayland: false }]);
        assert!(!d.xwayland_is_experimental());
    }

    #[test]
    fn no_session_has_no_candidates() {
        let d = detect(env(&[("DISPLAY", " ")]));
        assert!(d.candidates.is_empty());
    }
}
