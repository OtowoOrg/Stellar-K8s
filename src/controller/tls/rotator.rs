//! Automated mTLS certificate rotation engine for the internal node mesh.
//!
//! The rotator decides when a peer certificate is close enough to expiry to be
//! replaced and issues a fresh certificate signed by the mesh CA. It is a pure,
//! side-effect-free engine: persisting the result to a `Secret` and signalling
//! pods to reload is handled by [`crate::controller::mtls_rotation`].

use crate::controller::tls::ca::{remaining_validity, CaCertificate};
use crate::error::{Error, Result};
use chrono::Datelike;
use rcgen::{
    CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, Ia5String, KeyPair,
    KeyUsagePurpose, SanType,
};
use std::time::{Duration, SystemTime};

/// Policy controlling when and how mesh certificates are rotated.
#[derive(Debug, Clone)]
pub struct RotationPolicy {
    /// Rotate once the remaining validity drops to or below this window.
    pub rotation_window: Duration,
    /// Requested lifetime for newly issued certificates.
    pub ttl: Duration,
    /// How often mesh nodes should reload their certificates from disk.
    pub reload_interval: Duration,
}

impl Default for RotationPolicy {
    fn default() -> Self {
        Self {
            rotation_window: Duration::from_secs(7 * 24 * 60 * 60),
            ttl: Duration::from_secs(90 * 24 * 60 * 60),
            reload_interval: Duration::from_secs(5 * 60),
        }
    }
}

/// A freshly issued certificate bundle ready to be written to a `Secret`.
#[derive(Debug, Clone)]
pub struct RotatedCertificate {
    /// PEM-encoded leaf certificate.
    pub cert_pem: String,
    /// PEM-encoded leaf private key.
    pub key_pem: String,
    /// PEM-encoded issuing CA certificate.
    pub ca_cert_pem: String,
    /// Absolute expiry time of the issued leaf certificate.
    pub expires_at: SystemTime,
}

impl Default for RotatedCertificate {
    fn default() -> Self {
        Self {
            cert_pem: String::new(),
            key_pem: String::new(),
            ca_cert_pem: String::new(),
            expires_at: SystemTime::UNIX_EPOCH,
        }
    }
}

/// Stateless certificate rotation engine.
#[derive(Debug, Clone, Default)]
pub struct Rotator {
    /// Rotation policy applied to every decision made by this engine.
    pub policy: RotationPolicy,
}

impl Rotator {
    /// Create a rotator that applies `policy`.
    pub fn new(policy: RotationPolicy) -> Self {
        Self { policy }
    }

    /// Returns `true` when `cert_pem` is within the rotation window (or expired).
    pub fn should_rotate(&self, cert_pem: &str) -> Result<bool> {
        Ok(remaining_validity(cert_pem)? <= self.policy.rotation_window)
    }

    /// Issue a new certificate for `dns_names`, signed by the supplied CA.
    pub fn issue_signed_certificate(
        &self,
        ca_cert_pem: &str,
        ca_key_pem: &str,
        dns_names: &[String],
    ) -> Result<RotatedCertificate> {
        let ca_key = KeyPair::from_pem(ca_key_pem).map_err(Error::CertificateError)?;
        let ca_params =
            CertificateParams::from_ca_cert_pem(ca_cert_pem).map_err(Error::CertificateError)?;
        let ca_cert = ca_params
            .self_signed(&ca_key)
            .map_err(Error::CertificateError)?;

        let mut params = CertificateParams::default();
        params.distinguished_name = DistinguishedName::new();
        params
            .distinguished_name
            .push(DnType::CommonName, "stellar-internal-node");
        params.key_usages.push(KeyUsagePurpose::DigitalSignature);
        params.key_usages.push(KeyUsagePurpose::KeyEncipherment);
        params
            .extended_key_usages
            .push(ExtendedKeyUsagePurpose::ServerAuth);
        params
            .extended_key_usages
            .push(ExtendedKeyUsagePurpose::ClientAuth);
        for dns in dns_names {
            params.subject_alt_names.push(SanType::DnsName(
                Ia5String::try_from(dns.clone()).map_err(|e| Error::ConfigError(e.to_string()))?,
            ));
        }
        // `rcgen` works in whole-day validity granularity, which is sufficient
        // for a ~90-day mesh certificate.
        let ttl = chrono::Duration::from_std(self.policy.ttl)
            .unwrap_or_else(|_| chrono::Duration::days(90));
        let expiry = chrono::Utc::now() + ttl;
        params.not_after =
            rcgen::date_time_ymd(expiry.year(), expiry.month() as u8, expiry.day() as u8);

        let key_pair = KeyPair::generate().map_err(Error::CertificateError)?;
        let cert = params
            .signed_by(&key_pair, &ca_cert, &ca_key)
            .map_err(Error::CertificateError)?;
        let cert_pem = cert.pem();

        let expires_at = SystemTime::now() + remaining_validity(&cert_pem)?;
        Ok(RotatedCertificate {
            cert_pem,
            key_pem: key_pair.serialize_pem(),
            ca_cert_pem: ca_cert_pem.to_string(),
            expires_at,
        })
    }

    /// Run a full rotation against an ephemeral CA.
    ///
    /// Intended for local simulation and tests where no CA `Secret` exists yet;
    /// production callers pass a real CA to [`Self::issue_signed_certificate`].
    pub fn simulate_rotation_cycle(
        &self,
        cert_pem: &str,
        dns_names: &[String],
    ) -> Result<RotatedCertificate> {
        if !self.should_rotate(cert_pem)? {
            return Err(Error::ConfigError(
                "certificate is not due for rotation".to_string(),
            ));
        }

        let ca = CaCertificate::generate("stellar-internal-ca")?;
        self.issue_signed_certificate(&ca.cert_pem, &ca.key_pem, dns_names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn self_signed_cert(valid_for: chrono::Duration) -> String {
        let mut params = CertificateParams::default();
        params.distinguished_name = DistinguishedName::new();
        params
            .distinguished_name
            .push(DnType::CommonName, "peer.internal");
        let expiry = chrono::Utc::now() + valid_for;
        params.not_after =
            rcgen::date_time_ymd(expiry.year(), expiry.month() as u8, expiry.day() as u8);
        let key_pair = KeyPair::generate().unwrap();
        params.self_signed(&key_pair).unwrap().pem()
    }

    #[test]
    fn does_not_rotate_a_long_lived_certificate() {
        let cert = self_signed_cert(chrono::Duration::days(60));
        let rotator = Rotator::new(RotationPolicy::default());
        assert!(!rotator.should_rotate(&cert).unwrap());
    }

    #[test]
    fn rotates_an_expired_certificate() {
        let cert = self_signed_cert(chrono::Duration::days(-1));
        let rotator = Rotator::new(RotationPolicy::default());
        assert!(rotator.should_rotate(&cert).unwrap());
    }

    #[test]
    fn issues_certificate_for_internal_service() {
        let ca = CaCertificate::generate("ca.internal").unwrap();
        let rotator = Rotator::new(RotationPolicy::default());
        let issued = rotator
            .issue_signed_certificate(&ca.cert_pem, &ca.key_pem, &["node.internal".to_string()])
            .unwrap();
        assert!(issued.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(issued.key_pem.contains("BEGIN PRIVATE KEY"));
        assert_eq!(issued.ca_cert_pem, ca.cert_pem);
        assert!(issued.expires_at > SystemTime::now());
    }

    #[test]
    fn simulate_rotation_cycle_issues_fresh_certificate() {
        let expiring = self_signed_cert(chrono::Duration::days(-1));
        let rotator = Rotator::new(RotationPolicy::default());
        let rotated = rotator
            .simulate_rotation_cycle(&expiring, &["node.internal".to_string()])
            .unwrap();
        assert!(rotated.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(rotated.expires_at > SystemTime::now());
    }

    #[test]
    fn simulate_rotation_cycle_rejects_healthy_certificate() {
        let healthy = self_signed_cert(chrono::Duration::days(60));
        let rotator = Rotator::new(RotationPolicy::default());
        assert!(rotator
            .simulate_rotation_cycle(&healthy, &["node.internal".to_string()])
            .is_err());
    }
}
