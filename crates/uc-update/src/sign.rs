//! Signatures in the minisign format (plan 8.4: "the same minisign signature format").
//!
//! We sign in the pre-hashed mode (`ED`: Ed25519 over the BLAKE2b-512 hash of the data). The
//! check uses the `minisign-verify` crate, so a file signed here can also be checked with the
//! `minisign` tool: `minisign -Vm FILE -P PUBLIC_KEY`.
//!
//! The **trusted comment** is part of what is signed. We put the file name and the version in
//! it (`uclip <name> <version>`), and the check compares it. So a valid signature of one file
//! cannot be reused for another file or another version.
//!
//! The secret key file is our own simple format (not the encrypted minisign key file):
//! `uclip-updater-secret-v1:` and base64 of key ID (8 bytes) and seed (32 bytes). It lives
//! outside the repo, and in the GitHub environment `release` as a secret (plan 8.4).

use blake2::{Blake2b512, Digest};
use data_encoding::BASE64;
use ed25519_dalek::{Signer, SigningKey};
use rand_core::{OsRng, RngCore};
use zeroize::Zeroizing;

use crate::UpdateError;

const SECRET_PREFIX: &str = "uclip-updater-secret-v1:";

/// An updater secret key.
pub struct SecretKey {
    key_id: [u8; 8],
    sk: SigningKey,
}

impl SecretKey {
    /// A new random key.
    pub fn generate() -> SecretKey {
        let mut key_id = [0u8; 8];
        OsRng.fill_bytes(&mut key_id);
        SecretKey {
            key_id,
            sk: SigningKey::generate(&mut OsRng),
        }
    }

    /// The key as one line of text (keep it secret).
    pub fn to_text(&self) -> Zeroizing<String> {
        let mut b = Zeroizing::new(Vec::with_capacity(40));
        b.extend_from_slice(&self.key_id);
        b.extend_from_slice(self.sk.as_bytes());
        Zeroizing::new(format!("{SECRET_PREFIX}{}", BASE64.encode(&b)))
    }

    /// Read a key written by [`SecretKey::to_text`].
    pub fn from_text(s: &str) -> Result<SecretKey, UpdateError> {
        let bad = || UpdateError::BadKey("not an updater secret key".into());
        let rest = s.trim().strip_prefix(SECRET_PREFIX).ok_or_else(bad)?;
        let b = Zeroizing::new(BASE64.decode(rest.as_bytes()).map_err(|_| bad())?);
        if b.len() != 40 {
            return Err(bad());
        }
        let mut key_id = [0u8; 8];
        key_id.copy_from_slice(&b[..8]);
        let mut seed = Zeroizing::new([0u8; 32]);
        seed.copy_from_slice(&b[8..]);
        Ok(SecretKey {
            key_id,
            sk: SigningKey::from_bytes(&seed),
        })
    }

    /// The public key in minisign form (base64 of `Ed`, key ID, key).
    pub fn public_b64(&self) -> String {
        let mut b = Vec::with_capacity(42);
        b.extend_from_slice(b"Ed");
        b.extend_from_slice(&self.key_id);
        b.extend_from_slice(self.sk.verifying_key().as_bytes());
        BASE64.encode(&b)
    }

    /// Sign `data`. `trusted_comment` must be one line. Returns the `.minisig` text.
    pub fn sign(&self, data: &[u8], trusted_comment: &str) -> Result<String, UpdateError> {
        if trusted_comment.contains(['\n', '\r']) {
            return Err(UpdateError::BadKey("the comment must be one line".into()));
        }
        let hash = Blake2b512::digest(data);
        let sig = self.sk.sign(&hash).to_bytes();
        let mut bin1 = Vec::with_capacity(74);
        bin1.extend_from_slice(b"ED");
        bin1.extend_from_slice(&self.key_id);
        bin1.extend_from_slice(&sig);
        let mut global = Vec::with_capacity(64 + trusted_comment.len());
        global.extend_from_slice(&sig);
        global.extend_from_slice(trusted_comment.as_bytes());
        let gsig = self.sk.sign(&global).to_bytes();
        let id = data_encoding::HEXUPPER.encode(&self.key_id);
        Ok(format!(
            "untrusted comment: signature from uclip updater key {id}\n{}\ntrusted comment: {trusted_comment}\n{}\n",
            BASE64.encode(&bin1),
            BASE64.encode(&gsig)
        ))
    }
}

/// Check `sig_text` over `data` with the public key `public_b64`, and check that the trusted
/// comment is exactly `expect_comment`. Legacy (non pre-hashed) signatures are refused.
pub fn verify(
    public_b64: &str,
    data: &[u8],
    sig_text: &str,
    expect_comment: &str,
) -> Result<(), UpdateError> {
    let pk = minisign_verify::PublicKey::from_base64(public_b64.trim())
        .map_err(|e| UpdateError::BadKey(e.to_string()))?;
    let sig = minisign_verify::Signature::decode(sig_text)
        .map_err(|e| UpdateError::BadSignature(e.to_string()))?;
    pk.verify(data, &sig, false)
        .map_err(|e| UpdateError::BadSignature(e.to_string()))?;
    if sig.trusted_comment() != expect_comment {
        return Err(UpdateError::BadSignature(format!(
            "signed for \"{}\", expected \"{expect_comment}\"",
            sig.trusted_comment()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_round_trip() {
        let k = SecretKey::generate();
        let pk = k.public_b64();
        let s = k.sign(b"hello", "uclip a.exe 1.0.0").unwrap();
        verify(&pk, b"hello", &s, "uclip a.exe 1.0.0").unwrap();
        // Other data, other comment, other key: refused.
        assert!(verify(&pk, b"hellp", &s, "uclip a.exe 1.0.0").is_err());
        assert!(verify(&pk, b"hello", &s, "uclip b.exe 1.0.0").is_err());
        let other = SecretKey::generate().public_b64();
        assert!(verify(&other, b"hello", &s, "uclip a.exe 1.0.0").is_err());
    }

    #[test]
    fn a_changed_trusted_comment_breaks_the_global_signature() {
        let k = SecretKey::generate();
        let s = k.sign(b"x", "uclip a 1.0.0").unwrap();
        let forged = s.replace("uclip a 1.0.0", "uclip a 9.0.0");
        assert!(verify(&k.public_b64(), b"x", &forged, "uclip a 9.0.0").is_err());
    }

    #[test]
    fn secret_key_text_round_trip() {
        let k = SecretKey::generate();
        let t = k.to_text();
        let k2 = SecretKey::from_text(&t).unwrap();
        assert_eq!(k.public_b64(), k2.public_b64());
        assert!(SecretKey::from_text("nope").is_err());
        assert!(k.sign(b"x", "two\nlines").is_err());
    }
}
