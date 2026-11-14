//! Device key store trait and per-OS backends.
//!
//! A [`KeyStore`] keeps small secrets (the device key seed, the key that wraps the group secret)
//! by name. Backends:
//!
//! - [`MemoryKeyStore`]: in memory, for tests.
//! - [`FileKeyStore`]: one file per secret in a folder. Used in test mode
//!   (`UCLIP_TEST_KEYSTORE=file`) and as the Linux fallback when no Secret Service runs. On Unix
//!   the files are created with mode `0600`.
//! - [`OsKeyStore`]: the OS key store through the `keyring` crate. Windows: Credential Manager
//!   (DPAPI, per user). macOS: the login Keychain. Linux: the Secret Service (GNOME Keyring,
//!   KWallet).
//!
//! [`DeviceKey`] is the device's Ed25519 key pair. Its public key is the device ID (plan 3.12,
//! protocol 5.1).

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ed25519_dalek::{SigningKey, VerifyingKey};
use rand_core::OsRng;
use zeroize::{Zeroize, Zeroizing};

/// Name of the device key seed in a key store.
pub const DEVICE_KEY_NAME: &str = "device-key";

/// Service name used in the OS key store. The profile is added after a dot when it is not empty.
pub const SERVICE: &str = "UniversalClipboard";

/// Largest secret a store accepts, in bytes. Windows Credential Manager allows 2,560.
pub const MAX_SECRET_LEN: usize = 2048;

/// A key store error. It never holds secret bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyError {
    /// The name is empty, too long, or holds characters other than `a-z`, `0-9`, `-`, `_`, `.`.
    BadName,
    /// The secret is empty or longer than [`MAX_SECRET_LEN`].
    BadSize,
    /// The stored value is not what was expected (for example a device key that is not 32 bytes).
    Corrupt,
    /// The backend is not available (for example no Secret Service on this Linux session).
    Unavailable(String),
    /// Any other backend error.
    Backend(String),
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyError::BadName => f.write_str("bad key name"),
            KeyError::BadSize => f.write_str("bad secret size"),
            KeyError::Corrupt => f.write_str("stored key is corrupt"),
            KeyError::Unavailable(e) => write!(f, "key store not available: {e}"),
            KeyError::Backend(e) => write!(f, "key store error: {e}"),
        }
    }
}

impl std::error::Error for KeyError {}

/// What kind of store is in use. The diagnostic report shows it (plan 3.12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreKind {
    Memory,
    /// A file store chosen on purpose (test mode).
    File,
    /// A file store used because the OS store is not available (Linux without Secret Service).
    FileFallback,
    Os,
}

/// A store for small named secrets.
pub trait KeyStore: Send + Sync {
    /// Reads a secret. `Ok(None)` if there is none with that name.
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, KeyError>;
    /// Writes a secret, replacing any old one.
    fn set(&self, name: &str, secret: &[u8]) -> Result<(), KeyError>;
    /// Deletes a secret. Deleting a missing secret is not an error.
    fn delete(&self, name: &str) -> Result<(), KeyError>;
    /// The kind of this store.
    fn kind(&self) -> StoreKind;
}

fn check_name(name: &str) -> Result<(), KeyError> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_.".contains(&b))
        && !name.starts_with('.');
    if ok { Ok(()) } else { Err(KeyError::BadName) }
}

fn check_secret(secret: &[u8]) -> Result<(), KeyError> {
    if secret.is_empty() || secret.len() > MAX_SECRET_LEN {
        Err(KeyError::BadSize)
    } else {
        Ok(())
    }
}

/// In-memory store for tests. Values are wiped when replaced, deleted or dropped.
#[derive(Default)]
pub struct MemoryKeyStore {
    items: Mutex<HashMap<String, Zeroizing<Vec<u8>>>>,
}

impl MemoryKeyStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of stored secrets.
    pub fn len(&self) -> usize {
        self.items.lock().map(|m| m.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl KeyStore for MemoryKeyStore {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, KeyError> {
        check_name(name)?;
        let m = self
            .items
            .lock()
            .map_err(|_| KeyError::Backend("poisoned".into()))?;
        Ok(m.get(name).cloned())
    }

    fn set(&self, name: &str, secret: &[u8]) -> Result<(), KeyError> {
        check_name(name)?;
        check_secret(secret)?;
        let mut m = self
            .items
            .lock()
            .map_err(|_| KeyError::Backend("poisoned".into()))?;
        m.insert(name.to_string(), Zeroizing::new(secret.to_vec()));
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<(), KeyError> {
        check_name(name)?;
        let mut m = self
            .items
            .lock()
            .map_err(|_| KeyError::Backend("poisoned".into()))?;
        m.remove(name);
        Ok(())
    }

    fn kind(&self) -> StoreKind {
        StoreKind::Memory
    }
}

/// One file per secret in a folder. On Unix the folder is `0700` and the files are `0600`.
pub struct FileKeyStore {
    dir: PathBuf,
    kind: StoreKind,
}

impl FileKeyStore {
    /// A file store in `dir`, chosen on purpose (test mode).
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            kind: StoreKind::File,
        }
    }

    /// A file store used because the OS store is missing.
    pub fn fallback(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            kind: StoreKind::FileFallback,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.key"))
    }

    fn ensure_dir(&self) -> Result<(), KeyError> {
        let mut b = std::fs::DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            b.mode(0o700);
        }
        b.create(&self.dir).map_err(io_err)
    }
}

fn io_err(e: std::io::Error) -> KeyError {
    KeyError::Backend(e.kind().to_string())
}

impl KeyStore for FileKeyStore {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, KeyError> {
        check_name(name)?;
        match std::fs::read(self.path(name)) {
            Ok(v) => Ok(Some(Zeroizing::new(v))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_err(e)),
        }
    }

    fn set(&self, name: &str, secret: &[u8]) -> Result<(), KeyError> {
        use std::io::Write;
        check_name(name)?;
        check_secret(secret)?;
        self.ensure_dir()?;
        let tmp = self.dir.join(format!("{name}.key.tmp"));
        let _ = std::fs::remove_file(&tmp);
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        let mut f = o.open(&tmp).map_err(io_err)?;
        f.write_all(secret).map_err(io_err)?;
        f.sync_all().map_err(io_err)?;
        drop(f);
        std::fs::rename(&tmp, self.path(name)).map_err(io_err)
    }

    fn delete(&self, name: &str) -> Result<(), KeyError> {
        check_name(name)?;
        match std::fs::remove_file(self.path(name)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io_err(e)),
        }
    }

    fn kind(&self) -> StoreKind {
        self.kind
    }
}

/// The OS key store. The `profile` keeps a test or dev profile apart from the real one.
pub struct OsKeyStore {
    service: String,
}

impl OsKeyStore {
    /// `profile` is empty for the normal app, or a name like `dev`.
    pub fn new(profile: &str) -> Self {
        let service = if profile.is_empty() {
            SERVICE.to_string()
        } else {
            format!("{SERVICE}.{profile}")
        };
        Self { service }
    }

    pub fn service(&self) -> &str {
        &self.service
    }

    fn entry(&self, name: &str) -> Result<keyring::Entry, KeyError> {
        check_name(name)?;
        keyring::Entry::new(&self.service, name).map_err(kr_err)
    }

    /// True if the OS store answers. Used to pick the Linux file fallback.
    pub fn probe(&self) -> bool {
        match self.entry("probe") {
            Ok(e) => !matches!(
                e.get_secret(),
                Err(keyring::Error::NoStorageAccess(_)) | Err(keyring::Error::PlatformFailure(_))
            ),
            Err(_) => false,
        }
    }
}

fn kr_err(e: keyring::Error) -> KeyError {
    match e {
        keyring::Error::NoStorageAccess(_) => KeyError::Unavailable("no storage access".into()),
        keyring::Error::PlatformFailure(_) => KeyError::Unavailable("platform failure".into()),
        keyring::Error::BadEncoding(_) => KeyError::Corrupt,
        keyring::Error::TooLong(_, _) => KeyError::BadSize,
        keyring::Error::Invalid(_, _) => KeyError::BadName,
        other => KeyError::Backend(format!("{:?}", std::mem::discriminant(&other))),
    }
}

impl KeyStore for OsKeyStore {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, KeyError> {
        match self.entry(name)?.get_secret() {
            Ok(v) => Ok(Some(Zeroizing::new(v))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(kr_err(e)),
        }
    }

    fn set(&self, name: &str, secret: &[u8]) -> Result<(), KeyError> {
        check_secret(secret)?;
        self.entry(name)?.set_secret(secret).map_err(kr_err)
    }

    fn delete(&self, name: &str) -> Result<(), KeyError> {
        match self.entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(kr_err(e)),
        }
    }

    fn kind(&self) -> StoreKind {
        StoreKind::Os
    }
}

/// Picks the store for this run.
///
/// - `UCLIP_TEST_KEYSTORE=file` (only together with a non-empty profile): a [`FileKeyStore`] in
///   `state_dir/keys`.
/// - Linux, when the Secret Service does not answer: a fallback [`FileKeyStore`] in
///   `state_dir/keys`. The caller shows that in the diagnostic report ([`StoreKind`]).
/// - Else: the [`OsKeyStore`].
pub fn open_store(profile: &str, state_dir: &Path) -> Box<dyn KeyStore> {
    let test_file = std::env::var("UCLIP_TEST_KEYSTORE").is_ok_and(|v| v == "file");
    if test_file && !profile.is_empty() {
        return Box::new(FileKeyStore::new(state_dir.join("keys")));
    }
    let os = OsKeyStore::new(profile);
    if cfg!(target_os = "linux") && !os.probe() {
        return Box::new(FileKeyStore::fallback(state_dir.join("keys")));
    }
    Box::new(os)
}

/// The device's Ed25519 key pair. The secret seed is wiped on drop.
pub struct DeviceKey {
    signing: SigningKey,
}

impl DeviceKey {
    /// A new random key.
    pub fn generate() -> Self {
        Self {
            signing: SigningKey::generate(&mut OsRng),
        }
    }

    /// A key from a 32-byte seed (tests and vectors).
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        Self {
            signing: SigningKey::from_bytes(seed),
        }
    }

    /// The public key: the device ID.
    pub fn public(&self) -> [u8; 32] {
        self.signing.verifying_key().to_bytes()
    }

    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing.verifying_key()
    }

    pub fn signing_key(&self) -> &SigningKey {
        &self.signing
    }

    /// Loads the device key, or makes and stores a new one if there is none.
    pub fn load_or_create(store: &dyn KeyStore) -> Result<Self, KeyError> {
        if let Some(k) = Self::load(store)? {
            return Ok(k);
        }
        let k = Self::generate();
        k.save(store)?;
        Ok(k)
    }

    /// Loads the device key. `Ok(None)` if there is none.
    pub fn load(store: &dyn KeyStore) -> Result<Option<Self>, KeyError> {
        let Some(v) = store.get(DEVICE_KEY_NAME)? else {
            return Ok(None);
        };
        let mut seed: [u8; 32] = v.as_slice().try_into().map_err(|_| KeyError::Corrupt)?;
        let k = Self::from_seed(&seed);
        seed.zeroize();
        Ok(Some(k))
    }

    fn save(&self, store: &dyn KeyStore) -> Result<(), KeyError> {
        let seed = Zeroizing::new(self.signing.to_bytes());
        store.set(DEVICE_KEY_NAME, seed.as_slice())
    }

    /// Wipes the stored key and makes a new one (protocol 5.7: a removed device makes a new key).
    pub fn replace(store: &dyn KeyStore) -> Result<Self, KeyError> {
        store.delete(DEVICE_KEY_NAME)?;
        let k = Self::generate();
        k.save(store)?;
        Ok(k)
    }
}

impl fmt::Debug for DeviceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DeviceKey(public ")?;
        for b in self.public() {
            write!(f, "{b:02x}")?;
        }
        write!(f, ")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        let r: u64 = rand_core::RngCore::next_u64(&mut OsRng);
        p.push(format!("uc-keys-test-{tag}-{r:016x}"));
        p
    }

    fn roundtrip(s: &dyn KeyStore) {
        assert_eq!(s.get("a").unwrap(), None);
        s.set("a", b"one").unwrap();
        assert_eq!(s.get("a").unwrap().unwrap().as_slice(), b"one");
        s.set("a", b"two").unwrap();
        assert_eq!(s.get("a").unwrap().unwrap().as_slice(), b"two");
        s.delete("a").unwrap();
        s.delete("a").unwrap();
        assert_eq!(s.get("a").unwrap(), None);
        assert_eq!(s.set("A", b"x"), Err(KeyError::BadName));
        assert_eq!(s.set("../x", b"x"), Err(KeyError::BadName));
        assert_eq!(s.set("", b"x"), Err(KeyError::BadName));
        assert_eq!(s.set(".x", b"x"), Err(KeyError::BadName));
        assert_eq!(s.set("b", b""), Err(KeyError::BadSize));
        assert_eq!(s.set("b", &[0; MAX_SECRET_LEN + 1]), Err(KeyError::BadSize));
    }

    fn device_key_flow(s: &dyn KeyStore) {
        let k1 = DeviceKey::load_or_create(s).unwrap();
        let k2 = DeviceKey::load_or_create(s).unwrap();
        assert_eq!(k1.public(), k2.public());
        let k3 = DeviceKey::replace(s).unwrap();
        assert_ne!(k1.public(), k3.public());
        assert_eq!(DeviceKey::load(s).unwrap().unwrap().public(), k3.public());
        s.set(DEVICE_KEY_NAME, b"short").unwrap();
        assert_eq!(DeviceKey::load(s).unwrap_err(), KeyError::Corrupt);
        s.delete(DEVICE_KEY_NAME).unwrap();
    }

    #[test]
    fn memory_store() {
        let s = MemoryKeyStore::new();
        roundtrip(&s);
        device_key_flow(&s);
        assert!(s.is_empty());
        assert_eq!(s.kind(), StoreKind::Memory);
    }

    #[test]
    fn file_store() {
        let dir = scratch("file");
        let s = FileKeyStore::new(&dir);
        roundtrip(&s);
        device_key_flow(&s);
        assert_eq!(s.kind(), StoreKind::File);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            s.set("p", b"x").unwrap();
            let m = std::fs::metadata(dir.join("p.key"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(m & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_mode_picks_file_store() {
        // Only reads the variable when it is set by the test runner; does not set it, so the
        // test does not race other tests. With no profile the file store is never picked.
        let dir = scratch("open");
        if std::env::var("UCLIP_TEST_KEYSTORE").is_ok_and(|v| v == "file") {
            assert_eq!(open_store("dev", &dir).kind(), StoreKind::File);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn debug_shows_no_secret() {
        let k = DeviceKey::from_seed(&[7; 32]);
        let d = format!("{k:?}");
        assert!(d.starts_with("DeviceKey(public "));
        assert!(!d.contains(&"07".repeat(32)));
    }

    /// Round trip through the real OS store, under the profile `uctest`. Ignored by default: it
    /// writes to the user's credential store and may show a prompt on macOS. Run with
    /// `cargo test -p uc-keys -- --ignored os_store`.
    #[test]
    #[ignore]
    fn os_store() {
        let s = OsKeyStore::new("uctest");
        let name = format!("t{:08x}", rand_core::RngCore::next_u32(&mut OsRng));
        assert_eq!(s.get(&name).unwrap(), None);
        s.set(&name, b"secret").unwrap();
        assert_eq!(s.get(&name).unwrap().unwrap().as_slice(), b"secret");
        s.delete(&name).unwrap();
        assert_eq!(s.get(&name).unwrap(), None);
    }

    proptest::proptest! {
        #[test]
        fn names_never_escape(name in ".{0,80}") {
            let dir = scratch("pt");
            let s = FileKeyStore::new(&dir);
            if s.set(&name, b"x").is_ok() {
                let p = s.path(&name);
                proptest::prop_assert_eq!(p.parent().unwrap(), dir.as_path());
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
