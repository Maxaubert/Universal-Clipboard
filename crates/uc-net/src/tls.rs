//! TLS 1.3 with raw Ed25519 public keys (RFC 7250), for QUIC.
//!
//! Each side shows its device key as a raw public key. There are no certificates, no CA, no
//! server names and no SNI. The dialer checks that the server's key is the device it meant to
//! reach. The server accepts any key in the handshake; admission (is this key a member?) runs
//! right after the handshake, before any data (plan 3.7, protocol 5.9).
//!
//! 0-RTT and session tickets are off: they would let a replayed first flight carry data.

use std::sync::Arc;

use ed25519_dalek::{Signer as _, SigningKey, VerifyingKey};
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{WebPkiSupportedAlgorithms, verify_tls13_signature_with_raw_key};
use rustls::pki_types::{
    AlgorithmIdentifier, CertificateDer, InvalidSignature, ServerName,
    SignatureVerificationAlgorithm, SubjectPublicKeyInfoDer, UnixTime, alg_id,
};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{CertificateError, DigitallySignedStruct, DistinguishedName, SignatureScheme};
use uc_core::DeviceId;

/// The fixed TLS server name. It is never sent (SNI is off) and means nothing.
pub const SERVER_NAME: &str = "uclip";

static ED25519: Ed25519Verify = Ed25519Verify;
static SIG_ALGS: WebPkiSupportedAlgorithms = WebPkiSupportedAlgorithms {
    all: &[&ED25519],
    mapping: &[(SignatureScheme::ED25519, &[&ED25519])],
};

/// The SPKI encoding of an Ed25519 public key.
pub fn spki(key: &DeviceId) -> SubjectPublicKeyInfoDer<'static> {
    rustls::sign::public_key_to_spki(&alg_id::ED25519, key.0)
}

/// Read the device key out of a raw-public-key "certificate" (an SPKI).
pub fn key_from_spki(der: &[u8]) -> Option<DeviceId> {
    if der.len() != 44 {
        return None;
    }
    let mut k = [0u8; 32];
    k.copy_from_slice(&der[12..]);
    let id = DeviceId(k);
    (spki(&id).as_ref() == der).then_some(id)
}

/// The device key of the other side of an established connection.
pub fn peer_key(conn: &quinn::Connection) -> Option<DeviceId> {
    let ident = conn.peer_identity()?;
    let certs = ident.downcast::<Vec<CertificateDer<'static>>>().ok()?;
    match certs.as_slice() {
        [one] => key_from_spki(one.as_ref()),
        _ => None,
    }
}

/// The ALPN the two sides agreed on.
pub fn alpn(conn: &quinn::Connection) -> Option<Vec<u8>> {
    let hd = conn.handshake_data()?;
    let hd = hd.downcast::<quinn::crypto::rustls::HandshakeData>().ok()?;
    hd.protocol
}

#[derive(Debug)]
struct Ed25519Verify;

impl SignatureVerificationAlgorithm for Ed25519Verify {
    fn verify_signature(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), InvalidSignature> {
        let pk: [u8; 32] = public_key.try_into().map_err(|_| InvalidSignature)?;
        let pk = VerifyingKey::from_bytes(&pk).map_err(|_| InvalidSignature)?;
        let sig = ed25519_dalek::Signature::from_slice(signature).map_err(|_| InvalidSignature)?;
        pk.verify_strict(message, &sig)
            .map_err(|_| InvalidSignature)
    }

    fn public_key_alg_id(&self) -> AlgorithmIdentifier {
        alg_id::ED25519
    }

    fn signature_alg_id(&self) -> AlgorithmIdentifier {
        alg_id::ED25519
    }

    fn fips(&self) -> bool {
        false
    }
}

#[derive(Clone, Debug)]
struct Ed25519Signer(SigningKey);

impl rustls::sign::SigningKey for Ed25519Signer {
    fn choose_scheme(&self, offered: &[SignatureScheme]) -> Option<Box<dyn rustls::sign::Signer>> {
        offered
            .contains(&SignatureScheme::ED25519)
            .then(|| Box::new(self.clone()) as Box<dyn rustls::sign::Signer>)
    }

    fn algorithm(&self) -> rustls::SignatureAlgorithm {
        rustls::SignatureAlgorithm::ED25519
    }

    fn public_key(&self) -> Option<SubjectPublicKeyInfoDer<'_>> {
        Some(spki(&DeviceId(self.0.verifying_key().to_bytes())))
    }
}

impl rustls::sign::Signer for Ed25519Signer {
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, rustls::Error> {
        Ok(self.0.sign(message).to_bytes().to_vec())
    }

    fn scheme(&self) -> SignatureScheme {
        SignatureScheme::ED25519
    }
}

#[derive(Debug)]
struct RawKeyResolver(Arc<rustls::sign::CertifiedKey>);

impl RawKeyResolver {
    fn new(key: &SigningKey) -> Self {
        let id = DeviceId(key.verifying_key().to_bytes());
        let cert = CertificateDer::from(spki(&id).as_ref().to_vec());
        let signer = Arc::new(Ed25519Signer(key.clone()));
        Self(Arc::new(rustls::sign::CertifiedKey::new(
            vec![cert],
            signer,
        )))
    }
}

impl rustls::client::ResolvesClientCert for RawKeyResolver {
    fn resolve(
        &self,
        _hints: &[&[u8]],
        _schemes: &[SignatureScheme],
    ) -> Option<Arc<rustls::sign::CertifiedKey>> {
        Some(self.0.clone())
    }

    fn only_raw_public_keys(&self) -> bool {
        true
    }

    fn has_certs(&self) -> bool {
        true
    }
}

impl rustls::server::ResolvesServerCert for RawKeyResolver {
    fn resolve(
        &self,
        _hello: rustls::server::ClientHello<'_>,
    ) -> Option<Arc<rustls::sign::CertifiedKey>> {
        Some(self.0.clone())
    }

    fn only_raw_public_keys(&self) -> bool {
        true
    }
}

/// Client side: the server must show exactly the expected key.
#[derive(Debug)]
struct ExpectKey(DeviceId);

impl ServerCertVerifier for ExpectKey {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if !intermediates.is_empty() || key_from_spki(end_entity.as_ref()) != Some(self.0) {
            return Err(rustls::Error::InvalidCertificate(
                CertificateError::UnknownIssuer,
            ));
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _m: &[u8],
        _c: &CertificateDer<'_>,
        _d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::PeerIncompatible(
            rustls::PeerIncompatible::Tls12NotOffered,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature_with_raw_key(
            message,
            &SubjectPublicKeyInfoDer::from(cert.as_ref()),
            dss,
            &SIG_ALGS,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        SIG_ALGS.supported_schemes()
    }

    fn requires_raw_public_keys(&self) -> bool {
        true
    }
}

/// Server side: any well-formed Ed25519 raw key. Admission runs after the handshake.
#[derive(Debug)]
struct AnyKey;

impl ClientCertVerifier for AnyKey {
    fn offer_client_auth(&self) -> bool {
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        if !intermediates.is_empty() || key_from_spki(end_entity.as_ref()).is_none() {
            return Err(rustls::Error::InvalidCertificate(
                CertificateError::UnknownIssuer,
            ));
        }
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _m: &[u8],
        _c: &CertificateDer<'_>,
        _d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::PeerIncompatible(
            rustls::PeerIncompatible::Tls12NotOffered,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature_with_raw_key(
            message,
            &SubjectPublicKeyInfoDer::from(cert.as_ref()),
            dss,
            &SIG_ALGS,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        SIG_ALGS.supported_schemes()
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn requires_raw_public_keys(&self) -> bool {
        true
    }
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// A TLS setup error (should never happen with the fixed provider).
#[derive(Debug)]
pub struct TlsError(pub String);

impl std::fmt::Display for TlsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TLS setup failed: {}", self.0)
    }
}

impl std::error::Error for TlsError {}

fn err(e: impl std::fmt::Display) -> TlsError {
    TlsError(e.to_string())
}

/// QUIC server crypto for our key and these ALPNs.
pub fn server_crypto(key: &SigningKey, alpns: &[Vec<u8>]) -> Result<QuicServerConfig, TlsError> {
    let mut c = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(err)?
        .with_client_cert_verifier(Arc::new(AnyKey))
        .with_cert_resolver(Arc::new(RawKeyResolver::new(key)));
    c.alpn_protocols = alpns.to_vec();
    c.max_early_data_size = 0;
    c.send_tls13_tickets = 0;
    QuicServerConfig::try_from(c).map_err(err)
}

/// QUIC client crypto that accepts only `expect` as the server, offering one ALPN.
pub fn client_crypto(
    key: &SigningKey,
    expect: DeviceId,
    alpn: &[u8],
) -> Result<QuicClientConfig, TlsError> {
    let mut c = rustls::ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(err)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(ExpectKey(expect)))
        .with_client_cert_resolver(Arc::new(RawKeyResolver::new(key)));
    c.alpn_protocols = vec![alpn.to_vec()];
    c.enable_sni = false;
    c.enable_early_data = false;
    c.resumption = rustls::client::Resumption::disabled();
    QuicClientConfig::try_from(c).map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spki_round_trip() {
        let k = SigningKey::from_bytes(&[7; 32]);
        let id = DeviceId(k.verifying_key().to_bytes());
        let der = spki(&id);
        assert_eq!(der.as_ref().len(), 44);
        assert_eq!(key_from_spki(der.as_ref()), Some(id));
        let mut bad = der.as_ref().to_vec();
        bad[0] ^= 1;
        assert_eq!(key_from_spki(&bad), None);
        assert_eq!(key_from_spki(&der.as_ref()[1..]), None);
    }

    #[test]
    fn configs_build() {
        let k = SigningKey::from_bytes(&[1; 32]);
        assert!(server_crypto(&k, &[b"uclip/1".to_vec()]).is_ok());
        assert!(client_crypto(&k, DeviceId([2; 32]), b"uclip/1").is_ok());
    }
}
