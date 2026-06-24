//! Opt-in TLS for the *direct* (non-tunneled) database connections.
//!
//! The encouraged path for the DB panels is an SSH tunnel, which already
//! encrypts the transport — so TLS defaults to [`TlsMode::Off`] and every
//! existing connection keeps its exact prior behavior. When a user points a
//! connection straight at a non-loopback host (no tunnel), `Require` /
//! `VerifyFull` stop the password handshake and query traffic from crossing
//! the network in cleartext.

use serde::{Deserialize, Serialize};

/// How a direct DB connection negotiates TLS.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TlsMode {
    /// No TLS — cleartext. The transport is expected to be an SSH tunnel.
    #[default]
    Off,
    /// Encrypt, but accept any server certificate. Stops passive
    /// eavesdropping (not an active MITM); works with self-signed servers.
    Require,
    /// Encrypt and verify the server certificate against the Mozilla root
    /// store + hostname — full protection.
    #[serde(rename = "verify-full")]
    VerifyFull,
}

impl TlsMode {
    /// Parse the wire string (`off` / `require` / `verify-full`). Unknown or
    /// empty → [`TlsMode::Off`] (fail safe toward the historical behavior).
    pub fn from_wire(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "require" => TlsMode::Require,
            "verify-full" | "verify_full" | "verifyfull" => TlsMode::VerifyFull,
            _ => TlsMode::Off,
        }
    }

    /// True for [`TlsMode::Off`].
    pub fn is_off(self) -> bool {
        matches!(self, TlsMode::Off)
    }
}

/// Flatten an error and its `source()` chain into a single `": "`-joined
/// string. Several driver errors — notably [`tokio_postgres::Error`] —
/// print only their own kind in `Display` (`error performing TLS
/// handshake`) and hide the useful detail (the rustls certificate
/// diagnostic) behind `source()`. Walking the chain surfaces it so the
/// frontend can localize "cert not trusted" / "hostname mismatch".
/// Adjacent duplicate segments are collapsed — an `io::Error` that just
/// re-displays its inner error would otherwise repeat it verbatim.
pub fn error_chain(err: &dyn std::error::Error) -> String {
    let mut parts: Vec<String> = vec![err.to_string()];
    let mut source = err.source();
    while let Some(e) = source {
        let s = e.to_string();
        if parts.last().map(|p| *p != s).unwrap_or(true) {
            parts.push(s);
        }
        source = e.source();
    }
    parts.join(": ")
}

/// Build a rustls [`ClientConfig`](rustls::ClientConfig) for the PostgreSQL
/// connector. Only meaningful for `Require` / `VerifyFull`; `Off` is handled
/// by the caller (plain `NoTls`). An explicit `ring` provider is named
/// because both `ring` and `aws-lc-rs` are present transitively, which makes
/// the process-default ambiguous (and would otherwise panic at runtime).
pub fn pg_rustls_config(mode: TlsMode) -> Result<rustls::ClientConfig, String> {
    use std::sync::Arc;

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("rustls client config: {e}"))?;

    let config = match mode {
        TlsMode::VerifyFull => {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            builder.with_root_certificates(roots).with_no_client_auth()
        }
        // `Require` (and the `Off` fallback, never reached here) encrypts but
        // does not verify the certificate.
        TlsMode::Require | TlsMode::Off => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyServerCert(provider)))
            .with_no_client_auth(),
    };
    Ok(config)
}

/// A rustls verifier that encrypts the channel but accepts any server
/// certificate. Used only for [`TlsMode::Require`]; signature checks still
/// run through the crypto provider so the handshake itself is sound.
#[derive(Debug)]
struct AcceptAnyServerCert(std::sync::Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for AcceptAnyServerCert {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wire_forms() {
        assert_eq!(TlsMode::from_wire("off"), TlsMode::Off);
        assert_eq!(TlsMode::from_wire(""), TlsMode::Off);
        assert_eq!(TlsMode::from_wire("bogus"), TlsMode::Off);
        assert_eq!(TlsMode::from_wire("require"), TlsMode::Require);
        assert_eq!(TlsMode::from_wire("verify-full"), TlsMode::VerifyFull);
        assert_eq!(TlsMode::from_wire("VERIFY-FULL"), TlsMode::VerifyFull);
        assert!(TlsMode::Off.is_off());
        assert!(!TlsMode::Require.is_off());
    }

    #[test]
    fn error_chain_surfaces_hidden_cause_and_dedups() {
        use std::error::Error;
        use std::fmt;

        // Innermost: the rustls cert verdict.
        #[derive(Debug)]
        struct Cert;
        impl fmt::Display for Cert {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("invalid peer certificate: NotValidForName")
            }
        }
        impl Error for Cert {}

        // Middle: mirrors `io::Error` wrapping — its Display equals the
        // inner's, so error_chain must collapse the repeat.
        #[derive(Debug)]
        struct Io(Cert);
        impl fmt::Display for Io {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
        impl Error for Io {
            fn source(&self) -> Option<&(dyn Error + 'static)> {
                Some(&self.0)
            }
        }

        // Top: mirrors `tokio_postgres::Error` — Display prints only its
        // own kind and hides the cause behind source().
        #[derive(Debug)]
        struct Top(Io);
        impl fmt::Display for Top {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("error performing TLS handshake")
            }
        }
        impl Error for Top {
            fn source(&self) -> Option<&(dyn Error + 'static)> {
                Some(&self.0)
            }
        }

        assert_eq!(
            error_chain(&Top(Io(Cert))),
            "error performing TLS handshake: invalid peer certificate: NotValidForName"
        );
    }

    #[test]
    fn pg_config_builds_without_panicking() {
        // Catches the dual-provider ambiguity / missing-provider panic at
        // test time, without needing a live server.
        assert!(pg_rustls_config(TlsMode::Require).is_ok());
        assert!(pg_rustls_config(TlsMode::VerifyFull).is_ok());
    }
}
