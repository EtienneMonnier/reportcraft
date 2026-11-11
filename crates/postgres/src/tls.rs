//! TLS for PostgreSQL connections, in Rust (rustls with the ring provider), following libpq's
//! `sslmode`: `disable` sends nothing encrypted; `prefer` (the default) and `require` encrypt
//! without checking the server's certificate, as libpq does; `verify-full` (and `verify-ca`, held
//! to the same standard) also checks that the certificate comes from an authority the system
//! trusts and names the server.

use std::sync::Arc;

use reportcraft_core::{Error, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use tokio_postgres_rustls::MakeRustlsConnect;

/// Whether the server's certificate is checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verify {
    /// Encrypted, certificate not checked (libpq's `prefer` and `require`).
    No,
    /// Certificate checked against the system's authorities and the server's name.
    Full,
}

/// The connection string the driver understands (it knows `disable`, `prefer` and `require`), and
/// whether to verify: `verify-full` and `verify-ca` become `require` with verification.
pub(crate) fn prepare(conninfo: &str) -> (String, Verify) {
    let mut verify = Verify::No;
    let mut out = conninfo.to_owned();
    for strict in ["sslmode=verify-full", "sslmode=verify-ca"] {
        if out.contains(strict) {
            out = out.replace(strict, "sslmode=require");
            verify = Verify::Full;
        }
    }
    (out, verify)
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// The TLS connector for `verify`.
pub(crate) fn connector(verify: Verify) -> Result<MakeRustlsConnect> {
    let provider = provider();
    let builder = ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Invalid(format!("TLS: {e}")))?;
    let config = match verify {
        Verify::Full => {
            let mut roots = rustls::RootCertStore::empty();
            let found = rustls_native_certs::load_native_certs();
            let (added, _) = roots.add_parsable_certificates(found.certs);
            if added == 0 {
                return Err(Error::Io("sslmode=verify-full: no certificate authority found on this system".into()));
            }
            builder.with_root_certificates(roots).with_no_client_auth()
        }
        Verify::No => builder.dangerous().with_custom_certificate_verifier(Arc::new(AnyCertificate(provider))).with_no_client_auth(),
    };
    Ok(MakeRustlsConnect::new(config))
}

/// Accepts the server's certificate without checking who issued it (libpq's `require`), while
/// still checking the handshake's signatures, so the connection is encrypted with the server
/// that holds the certificate's key.
#[derive(Debug)]
struct AnyCertificate(Arc<CryptoProvider>);

impl ServerCertVerifier for AnyCertificate {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_modes_become_require_with_verification() {
        assert_eq!(prepare("host=db sslmode=verify-full"), ("host=db sslmode=require".into(), Verify::Full));
        assert_eq!(prepare("postgresql://u@db/x?sslmode=verify-ca"), ("postgresql://u@db/x?sslmode=require".into(), Verify::Full));
        assert_eq!(prepare("host=db sslmode=require"), ("host=db sslmode=require".into(), Verify::No));
        assert_eq!(prepare("host=db"), ("host=db".into(), Verify::No));
        assert!(connector(Verify::No).is_ok());
    }
}
