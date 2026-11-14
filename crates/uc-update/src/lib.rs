//! Update check, signature check, staged rollout, health check and rollback (plan 3.13, 8.4).
//!
//! Who uses it: the Windows exe build and the macOS `.dmg` build. The Microsoft Store build,
//! the Linux `.deb` and the phones update through their store or package manager.
//!
//! | Module | What |
//! |---|---|
//! | [`sign`] | minisign-format signatures, the secret key file |
//! | [`manifest`] | `latest.json`: version, files, rollout, all signed |
//! | [`rollout`] | which devices take a release at a given percentage |
//! | [`state`] | `update.json`: bad versions, failures in a row |
//! | [`fetch`] | download (system `curl`, HTTPS only) and unpack (system `tar`) |
//! | [`apply`] | stop, back up, install, health check, roll back |
//!
//! The binary `uc-release` makes keys, signs files and writes `latest.json` (used by
//! `release.yml`).

pub mod apply;
pub mod fetch;
pub mod manifest;
pub mod rollout;
pub mod sign;
pub mod state;

use std::path::{Path, PathBuf};

use manifest::{FileEntry, Kind, Manifest, Signed};
use state::UpdateState;

/// The public updater key (minisign form). The secret half is not in the repo: it is in the
/// GitHub environment `release` and in Max's offline backup (plan 8.4).
pub const UPDATER_PUBLIC_KEY: &str = "RWT+8yLErcCQ5xMELo4ZbtGumYTct/xDw17Bj/pMuQ6KAKYt13um8NKb";

/// Where the agent looks for updates (plan 3.13 step 1: a static file, not the GitHub API).
pub const LATEST_URL: &str =
    "https://github.com/Maxaubert/Universal-Clipboard/releases/latest/download/latest.json";

/// Largest `latest.json` accepted.
pub const MAX_MANIFEST: u64 = 256 * 1024;
/// Largest update file accepted.
pub const MAX_FILE: u64 = 200 * 1024 * 1024;

/// An update error, in plain words for the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    BadKey(String),
    BadSignature(String),
    BadFile(String),
    Format(String),
    Fetch(String),
    Install(String),
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateError::BadKey(e) => write!(f, "bad key: {e}"),
            UpdateError::BadSignature(e) => write!(f, "bad signature: {e}"),
            UpdateError::BadFile(e) => write!(f, "bad file: {e}"),
            UpdateError::Format(e) => write!(f, "bad manifest: {e}"),
            UpdateError::Fetch(e) => write!(f, "download failed: {e}"),
            UpdateError::Install(e) => write!(f, "install failed: {e}"),
        }
    }
}

impl std::error::Error for UpdateError {}

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// Nothing newer.
    UpToDate,
    /// Newer, but not for this device now (rollout, bad version, gave up, no file).
    Skip(String),
    /// Take this file.
    Take(Manifest, FileEntry),
}

/// The settings of one check.
#[derive(Debug, Clone)]
pub struct Source {
    pub url: String,
    pub public_key: String,
    /// `file://` allowed (test profiles only).
    pub allow_file: bool,
    pub target: String,
}

impl Source {
    /// The real source for this build.
    pub fn real() -> Source {
        Source {
            url: LATEST_URL.into(),
            public_key: UPDATER_PUBLIC_KEY.into(),
            allow_file: false,
            target: manifest::current_target().into(),
        }
    }
}

/// Fetch and check `latest.json` and decide.
pub fn check(
    src: &Source,
    current: &str,
    device_id: &[u8],
    st: &UpdateState,
) -> Result<Check, UpdateError> {
    let bytes = fetch::fetch(&src.url, MAX_MANIFEST, src.allow_file)?;
    let m = Signed::parse(&bytes)?.open(&src.public_key)?;
    Ok(decide(m, &src.target, current, device_id, st))
}

/// The decision part of [`check`] (no I/O).
pub fn decide(
    m: Manifest,
    target: &str,
    current: &str,
    device_id: &[u8],
    st: &UpdateState,
) -> Check {
    if !manifest::is_newer(&m.version, current) {
        return Check::UpToDate;
    }
    if st.is_bad(&m.version) {
        return Check::Skip(format!("{} failed before", m.version));
    }
    if st.gave_up() {
        return Check::Skip("three updates failed in a row".into());
    }
    if !rollout::in_rollout(device_id, m.rollout) {
        return Check::Skip(format!(
            "{} is at {}% and this device is not in it yet",
            m.version, m.rollout
        ));
    }
    match m.file_for(target).cloned() {
        Some(f) => Check::Take(m, f),
        None => Check::Skip(format!("{} has no file for {target}", m.version)),
    }
}

/// What is ready to install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Staged {
    pub version: String,
    pub kind: Kind,
    /// The NSIS installer, or the unpacked folder.
    pub path: PathBuf,
}

/// Download the file, check it, and unpack it under `dir` (`dir/<version>/`).
pub fn stage(src: &Source, m: &Manifest, f: &FileEntry, dir: &Path) -> Result<Staged, UpdateError> {
    let data = fetch::fetch(&f.url, MAX_FILE.min(f.size), src.allow_file)?;
    f.verify(&data, &m.version, &src.public_key)?;
    let vdir = dir.join(&m.version);
    let io = |e: std::io::Error| UpdateError::Install(e.to_string());
    if vdir.exists() {
        std::fs::remove_dir_all(&vdir).map_err(io)?;
    }
    std::fs::create_dir_all(&vdir).map_err(io)?;
    // Only the file name part of the name, never a path.
    let name = Path::new(&f.name)
        .file_name()
        .ok_or_else(|| UpdateError::BadFile("bad name".into()))?;
    let file = vdir.join(name);
    std::fs::write(&file, &data).map_err(io)?;
    let path = match f.kind {
        Kind::Nsis => file,
        Kind::Tar => {
            let out = vdir.join("unpacked");
            fetch::untar(&file, &out)?;
            out
        }
    };
    Ok(Staged {
        version: m.version.clone(),
        kind: f.kind,
        path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use manifest::SCHEMA;
    use sign::SecretKey;

    #[test]
    fn the_built_in_public_key_parses() {
        minisign_verify::PublicKey::from_base64(UPDATER_PUBLIC_KEY).unwrap();
    }

    fn feed(dir: &Path, key: &SecretKey, version: &str, rollout: u8) -> Source {
        let payload = dir.join("payload");
        std::fs::create_dir_all(&payload).unwrap();
        std::fs::write(payload.join("agent"), version).unwrap();
        let arc = dir.join("app.tar.gz");
        fetch::tar_dir(&payload, &arc).unwrap();
        let data = std::fs::read(&arc).unwrap();
        let url = |p: &Path| {
            format!(
                "file:///{}",
                p.display()
                    .to_string()
                    .replace('\\', "/")
                    .trim_start_matches('/')
            )
        };
        let f = FileEntry::make(
            "test-target",
            Kind::Tar,
            "app.tar.gz",
            &url(&arc),
            version,
            &data,
            key,
        )
        .unwrap();
        let m = Manifest {
            schema: SCHEMA,
            version: version.into(),
            released: "2026-11-14".into(),
            rollout,
            files: vec![f],
        };
        let latest = dir.join("latest.json");
        std::fs::write(
            &latest,
            serde_json::to_vec(&Signed::sign(&m, key).unwrap()).unwrap(),
        )
        .unwrap();
        Source {
            url: url(&latest),
            public_key: key.public_b64(),
            allow_file: true,
            target: "test-target".into(),
        }
    }

    #[test]
    fn check_and_stage_from_a_signed_feed() {
        let d = tempfile::tempdir().unwrap();
        let key = SecretKey::generate();
        let src = feed(d.path(), &key, "9.9.9", 100);
        let st = UpdateState::default();
        let Check::Take(m, f) = check(&src, "0.0.1", b"dev", &st).unwrap() else {
            panic!("expected an update");
        };
        let s = stage(&src, &m, &f, &d.path().join("staging")).unwrap();
        assert_eq!(
            std::fs::read_to_string(s.path.join("agent")).unwrap(),
            "9.9.9"
        );
        assert_eq!(check(&src, "9.9.9", b"dev", &st).unwrap(), Check::UpToDate);
        // Another key: refused before anything is decided.
        let mut wrong = src.clone();
        wrong.public_key = SecretKey::generate().public_b64();
        assert!(check(&wrong, "0.0.1", b"dev", &st).is_err());
    }

    #[test]
    fn rollout_zero_and_bad_versions_skip() {
        let d = tempfile::tempdir().unwrap();
        let key = SecretKey::generate();
        let src = feed(d.path(), &key, "9.9.9", 0);
        let st = UpdateState::default();
        assert!(matches!(
            check(&src, "0.0.1", b"dev", &st).unwrap(),
            Check::Skip(_)
        ));
        let src = feed(d.path(), &key, "9.9.9", 100);
        let mut st = UpdateState::default();
        st.failed("9.9.9", "health", true);
        assert!(matches!(
            check(&src, "0.0.1", b"dev", &st).unwrap(),
            Check::Skip(_)
        ));
    }
}
