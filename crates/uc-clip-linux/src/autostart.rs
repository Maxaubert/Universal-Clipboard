//! XDG autostart entry (plan 3.2): a `.desktop` file in `~/.config/autostart`.
//!
//! Every desktop (GNOME, KDE, XFCE, Sway with a session helper, ...) starts these entries
//! inside the user's graphical session, so `DISPLAY` and `WAYLAND_DISPLAY` are set. There is
//! no systemd unit on purpose. The supervisor loop (`uc-agent --supervise`) is in the agent.
//!
//! This module is pure file work and is tested on every OS.

use std::io::Write;
use std::path::{Path, PathBuf};

/// The file name of our entry.
pub const FILE_NAME: &str = "io.github.maxaubert.uclip.desktop";

/// The entry to write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The absolute path of the agent binary.
    pub exec: PathBuf,
    /// Arguments, for example `--supervise`.
    pub args: Vec<String>,
}

impl Entry {
    /// The agent, started under its supervisor.
    pub fn agent(exec: impl Into<PathBuf>) -> Self {
        Self {
            exec: exec.into(),
            args: vec!["--supervise".to_owned()],
        }
    }

    /// The `.desktop` file text.
    pub fn render(&self) -> String {
        let mut exec = quote_arg(&self.exec.to_string_lossy());
        for a in &self.args {
            exec.push(' ');
            exec.push_str(&quote_arg(a));
        }
        format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Universal Clipboard\n\
             Comment=Keeps the clipboard the same on your devices\n\
             Exec={exec}\n\
             Icon=universal-clipboard\n\
             Terminal=false\n\
             NoDisplay=true\n\
             X-GNOME-Autostart-enabled=true\n\
             X-KDE-autostart-after=panel\n"
        )
    }
}

/// Quote one `Exec` argument (Desktop Entry spec, "The Exec key").
///
/// An argument with a reserved character is put in double quotes, and `"`, `` ` ``, `$` and
/// `\` inside are escaped with a backslash. Then the whole value is escaped as a string value,
/// so each backslash is doubled. A `%` becomes `%%` (field codes).
pub fn quote_arg(arg: &str) -> String {
    const RESERVED: &[char] = &[
        ' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(',
        ')', '`',
    ];
    let arg = arg.replace('%', "%%");
    if !arg.is_empty() && !arg.contains(RESERVED) {
        return arg;
    }
    let mut out = String::from("\"");
    for c in arg.chars() {
        match c {
            // Quoting escape gives `\x`; string escaping then doubles the backslash.
            '"' | '`' | '$' => {
                out.push_str("\\\\");
                out.push(c);
            }
            '\\' => out.push_str("\\\\\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The autostart folder: `$XDG_CONFIG_HOME/autostart`, else `$HOME/.config/autostart`.
pub fn dir<F: Fn(&str) -> Option<String>>(env: F) -> Option<PathBuf> {
    let abs = |v: Option<String>| v.filter(|v| v.starts_with('/')).map(PathBuf::from);
    abs(env("XDG_CONFIG_HOME"))
        .or_else(|| abs(env("HOME")).map(|h| h.join(".config")))
        .map(|c| c.join("autostart"))
}

/// The autostart folder from the process environment.
pub fn dir_from_env() -> Option<PathBuf> {
    dir(|k| std::env::var(k).ok())
}

/// Write the entry into `dir` (created if missing). The write is atomic: a temporary file,
/// then a rename. Returns the path of the entry.
pub fn install(dir: &Path, entry: &Entry) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(FILE_NAME);
    let tmp = dir.join(format!(".{FILE_NAME}.tmp"));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(entry.render().as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Remove the entry. Not an error when it is not there.
pub fn remove(dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(dir.join(FILE_NAME)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        r => r,
    }
}

/// The entry exists and is not switched off (`Hidden=true` or
/// `X-GNOME-Autostart-enabled=false`, which desktop settings apps write).
pub fn is_enabled(dir: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(dir.join(FILE_NAME)) else {
        return false;
    };
    !text.lines().any(|l| {
        let l = l.trim().replace(' ', "");
        l.eq_ignore_ascii_case("Hidden=true")
            || l.eq_ignore_ascii_case("X-GNOME-Autostart-enabled=false")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "uclip-autostart-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn render_has_required_keys() {
        let t = Entry::agent("/usr/bin/uc-agent").render();
        assert!(t.starts_with("[Desktop Entry]\n"));
        assert!(t.contains("\nType=Application\n"));
        assert!(t.contains("\nExec=/usr/bin/uc-agent --supervise\n"));
        assert!(t.contains("\nNoDisplay=true\n"));
        assert!(t.contains("\nX-GNOME-Autostart-enabled=true\n"));
        assert!(!t.contains('\u{2014}'));
    }

    #[test]
    fn quoting() {
        assert_eq!(quote_arg("/usr/bin/uc-agent"), "/usr/bin/uc-agent");
        assert_eq!(
            quote_arg("/opt/My App/uc-agent"),
            "\"/opt/My App/uc-agent\""
        );
        assert_eq!(quote_arg("50%"), "50%%");
        assert_eq!(quote_arg("a$b"), "\"a\\\\$b\"");
        assert_eq!(quote_arg("a\\b"), "\"a\\\\\\\\b\"");
        assert_eq!(quote_arg(""), "\"\"");
    }

    #[test]
    fn dir_rules() {
        let d = dir(|k| match k {
            "XDG_CONFIG_HOME" => Some("/x/cfg".into()),
            "HOME" => Some("/home/u".into()),
            _ => None,
        });
        assert_eq!(d, Some(PathBuf::from("/x/cfg").join("autostart")));
        // A relative XDG_CONFIG_HOME is invalid by the spec and ignored.
        let d = dir(|k| match k {
            "XDG_CONFIG_HOME" => Some("cfg".into()),
            "HOME" => Some("/home/u".into()),
            _ => None,
        });
        assert_eq!(
            d,
            Some(PathBuf::from("/home/u").join(".config").join("autostart"))
        );
        assert_eq!(dir(|_| None), None);
    }

    #[test]
    fn install_enable_remove() {
        let d = temp_dir("cycle");
        assert!(!is_enabled(&d));
        let p = install(&d, &Entry::agent("/usr/bin/uc-agent")).unwrap();
        assert_eq!(p, d.join(FILE_NAME));
        assert!(is_enabled(&d));
        // Writing again replaces the file and leaves no temporary file.
        install(&d, &Entry::agent("/opt/uc/uc-agent")).unwrap();
        let names: Vec<_> = std::fs::read_dir(&d)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1);
        assert!(
            std::fs::read_to_string(&p)
                .unwrap()
                .contains("/opt/uc/uc-agent")
        );
        // A desktop settings app switched it off.
        let off = std::fs::read_to_string(&p).unwrap() + "Hidden=true\n";
        std::fs::write(&p, off).unwrap();
        assert!(!is_enabled(&d));
        remove(&d).unwrap();
        remove(&d).unwrap();
        assert!(!p.exists());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
