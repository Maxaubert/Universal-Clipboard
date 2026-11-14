//! `latest.json` (plan 3.13): the version, the files with a signature each, and the rollout
//! percentage. The whole manifest is signed too, so nobody can change the rollout or point a
//! file to another URL.
//!
//! On the wire `latest.json` is `{"manifest": "<the manifest as a JSON string>", "signature":
//! "<minisign text>"}`. The signature covers the exact manifest string. Its trusted comment is
//! `uclip latest.json <version>`. Each file's trusted comment is `uclip <name> <version>`.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::UpdateError;
use crate::sign::{SecretKey, verify};

/// The manifest format version this code reads.
pub const SCHEMA: u32 = 1;

/// How a file is installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// The Windows per-user NSIS installer, run silently (`/S`).
    Nsis,
    /// A `.tar.gz` of the install folder (macOS: the `.app` bundle). Unpacked with the
    /// system `tar`, then swapped in.
    Tar,
}

/// One file of a release.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// For example `windows-x86_64`, `macos-aarch64` ([`current_target`]).
    pub target: String,
    pub kind: Kind,
    /// The file name, for example `UniversalClipboard-0.1.0-setup.exe`.
    pub name: String,
    pub url: String,
    pub size: u64,
    /// Lower-case hex.
    pub sha256: String,
    /// The minisign text over the file.
    pub signature: String,
}

/// The signed content of `latest.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub version: String,
    /// Release date, `YYYY-MM-DD`. Shown only.
    pub released: String,
    /// 0 to 100: the share of devices that take this update (plan 3.13).
    pub rollout: u8,
    pub files: Vec<FileEntry>,
}

/// `latest.json` as it is on the server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signed {
    pub manifest: String,
    pub signature: String,
}

/// The comment signed with a file.
pub fn file_comment(name: &str, version: &str) -> String {
    format!("uclip {name} {version}")
}

/// The comment signed with the manifest.
pub fn manifest_comment(version: &str) -> String {
    format!("uclip latest.json {version}")
}

/// Lower-case hex SHA-256.
pub fn sha256_hex(data: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(&Sha256::digest(data))
}

impl Signed {
    /// Sign a manifest.
    pub fn sign(m: &Manifest, key: &SecretKey) -> Result<Signed, UpdateError> {
        let manifest = serde_json::to_string(m).map_err(|e| UpdateError::Format(e.to_string()))?;
        let signature = key.sign(manifest.as_bytes(), &manifest_comment(&m.version))?;
        Ok(Signed {
            manifest,
            signature,
        })
    }

    /// Parse `latest.json` bytes.
    pub fn parse(bytes: &[u8]) -> Result<Signed, UpdateError> {
        serde_json::from_slice(bytes).map_err(|e| UpdateError::Format(e.to_string()))
    }

    /// Check the signature and read the manifest. Nothing in it is trusted before this.
    pub fn open(&self, public_b64: &str) -> Result<Manifest, UpdateError> {
        let m: Manifest =
            serde_json::from_str(&self.manifest).map_err(|e| UpdateError::Format(e.to_string()))?;
        verify(
            public_b64,
            self.manifest.as_bytes(),
            &self.signature,
            &manifest_comment(&m.version),
        )?;
        if m.schema != SCHEMA {
            return Err(UpdateError::Format(format!("schema {}", m.schema)));
        }
        if parse_version(&m.version).is_none() || m.rollout > 100 {
            return Err(UpdateError::Format("bad version or rollout".into()));
        }
        Ok(m)
    }
}

impl Manifest {
    /// The file for `target`, if this release has one.
    pub fn file_for(&self, target: &str) -> Option<&FileEntry> {
        self.files.iter().find(|f| f.target == target)
    }
}

impl FileEntry {
    /// Make an entry for `data`, signed with `key`.
    pub fn make(
        target: &str,
        kind: Kind,
        name: &str,
        url: &str,
        version: &str,
        data: &[u8],
        key: &SecretKey,
    ) -> Result<FileEntry, UpdateError> {
        Ok(FileEntry {
            target: target.into(),
            kind,
            name: name.into(),
            url: url.into(),
            size: data.len() as u64,
            sha256: sha256_hex(data),
            signature: key.sign(data, &file_comment(name, version))?,
        })
    }

    /// Check a downloaded file: size, hash and signature (bound to name and version).
    pub fn verify(&self, data: &[u8], version: &str, public_b64: &str) -> Result<(), UpdateError> {
        if data.len() as u64 != self.size {
            return Err(UpdateError::BadFile("wrong size".into()));
        }
        if sha256_hex(data) != self.sha256 {
            return Err(UpdateError::BadFile("wrong hash".into()));
        }
        verify(
            public_b64,
            data,
            &self.signature,
            &file_comment(&self.name, version),
        )
    }
}

/// The release target of this build.
pub fn current_target() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "windows-x86_64",
        ("windows", "aarch64") => "windows-aarch64",
        ("macos", "aarch64") => "macos-aarch64",
        ("macos", "x86_64") => "macos-x86_64",
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        _ => "unknown",
    }
}

/// `a.b.c` only (no pre-release part).
pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut it = v.trim().split('.');
    let a = it.next()?.parse().ok()?;
    let b = it.next()?.parse().ok()?;
    let c = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some((a, b, c))
}

/// True if `candidate` is newer than `current`. Unreadable versions are never newer.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(key: &SecretKey) -> Manifest {
        let f = FileEntry::make(
            "windows-x86_64",
            Kind::Nsis,
            "UC-setup.exe",
            "https://example.invalid/UC-setup.exe",
            "1.2.3",
            b"installer bytes",
            key,
        )
        .unwrap();
        Manifest {
            schema: SCHEMA,
            version: "1.2.3".into(),
            released: "2026-11-14".into(),
            rollout: 10,
            files: vec![f],
        }
    }

    #[test]
    fn signed_manifest_round_trip_and_tamper() {
        let k = SecretKey::generate();
        let pk = k.public_b64();
        let s = Signed::sign(&sample(&k), &k).unwrap();
        let bytes = serde_json::to_vec(&s).unwrap();
        let m = Signed::parse(&bytes).unwrap().open(&pk).unwrap();
        assert_eq!(m.rollout, 10);
        let f = m.file_for("windows-x86_64").unwrap();
        f.verify(b"installer bytes", &m.version, &pk).unwrap();
        assert!(f.verify(b"installer bytez", &m.version, &pk).is_err());
        // A file signature cannot be moved to another version.
        assert!(f.verify(b"installer bytes", "1.2.4", &pk).is_err());

        // Raise the rollout without the key: refused.
        let mut t = s.clone();
        t.manifest = t.manifest.replace("\"rollout\":10", "\"rollout\":100");
        assert!(t.open(&pk).is_err());
        // Signed by another key: refused.
        let other = SecretKey::generate();
        assert!(
            Signed::sign(&sample(&other), &other)
                .unwrap()
                .open(&pk)
                .is_err()
        );
    }

    #[test]
    fn versions() {
        assert!(is_newer("0.0.2", "0.0.1"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.0.1", "0.0.1"));
        assert!(!is_newer("0.0.1", "0.0.2"));
        assert!(!is_newer("1.0.0-beta", "0.0.1"));
        assert!(!is_newer("1.0", "0.0.1"));
    }
}
