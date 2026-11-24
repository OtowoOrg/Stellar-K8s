//! Internal mesh certificate authority generation and expiry inspection.
//!
//! Provides a self-signed certificate authority used to sign short-lived
//! certificates for the internal node mesh. Expiry inspection is delegated to
//! `x509-parser` because `rcgen` does not expose validity data on the
//! certificates it generates.

use crate::error::{Error, Result};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair, KeyUsagePurpose,
};
use std::time::Duration;
use x509_parser::certificate::X509Certificate;
use x509_parser::pem::parse_x509_pem;
use x509_parser::prelude::FromDer;

/// A self-signed certificate authority for the internal node mesh.
#[derive(Debug, Clone)]
pub struct CaCertificate {
    /// PEM-encoded CA certificate.
    pub cert_pem: String,
    /// PEM-encoded CA private key.
    pub key_pem: String,
}

impl CaCertificate {
    /// Generate a new self-signed CA certificate with the given common name.
    pub fn generate(common_name: &str) -> Result<Self> {
        let mut params = CertificateParams::default();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.distinguished_name = DistinguishedName::new();
        params
            .distinguished_name
            .push(DnType::CommonName, common_name);
        params.key_usages.push(KeyUsagePurpose::DigitalSignature);
        params.key_usages.push(KeyUsagePurpose::KeyCertSign);
        params.key_usages.push(KeyUsagePurpose::CrlSign);

        let key_pair = KeyPair::generate().map_err(Error::CertificateError)?;
        let cert = params
            .self_signed(&key_pair)
            .map_err(Error::CertificateError)?;

        Ok(Self {
            cert_pem: cert.pem(),
            key_pem: key_pair.serialize_pem(),
        })
    }

    /// Returns `true` when the CA certificate expires within `threshold`.
    pub fn is_expiring_within(&self, threshold: Duration) -> Result<bool> {
        Ok(remaining_validity(&self.cert_pem)? <= threshold)
    }
}

/// Parse a PEM-encoded certificate and return the time left until it expires.
///
/// Returns `Duration::ZERO` when the certificate is already expired.
pub(crate) fn remaining_validity(cert_pem: &str) -> Result<Duration> {
    let (_, pem) = parse_x509_pem(cert_pem.as_bytes())
        .map_err(|e| Error::ConfigError(format!("failed to parse certificate PEM: {e}")))?;
    let (_, cert) = X509Certificate::from_der(&pem.contents)
        .map_err(|e| Error::ConfigError(format!("failed to parse certificate: {e}")))?;

    Ok(cert
        .validity()
        .time_to_expiration()
        .map(|d| Duration::from_secs(d.whole_seconds().max(0) as u64))
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_ca_certificate() {
        let ca = CaCertificate::generate("stellar-internal-ca").unwrap();
        assert!(ca.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(ca.key_pem.contains("BEGIN PRIVATE KEY"));
    }

    #[test]
    fn fresh_ca_is_not_expiring_immediately() {
        let ca = CaCertificate::generate("stellar-internal-ca").unwrap();
        assert!(!ca.is_expiring_within(Duration::from_secs(60)).unwrap());
    }
}
