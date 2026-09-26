// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Fail-closed health checks for admission webhook serving certificates.
//!
//! This module deliberately performs every check locally: it never contacts a
//! remote service. A cert-health sidecar can therefore gate a pod's readiness
//! before the Service or kubelet attempts an apiserver-style TLS connection.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;

use rustls::ServerConfig;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use thiserror::Error;
use x509_parser::extensions::GeneralName;
use x509_parser::prelude::{FromDer, X509Certificate};

#[derive(Debug, Error)]
pub enum CertHealthError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("certificate file contains no certificates: {0}")]
    NoCertificate(String),
    #[error("private key file contains no supported private key: {0}")]
    NoPrivateKey(String),
    #[error("TLS certificate and private key do not form a valid server identity: {0}")]
    InvalidServerIdentity(String),
    #[error("failed to parse X.509 certificate: {0}")]
    Parse(String),
    #[error("serving certificate is outside its validity period")]
    Expired,
    #[error("CA certificate is outside its validity period")]
    CaExpired,
    #[error("CA certificate is not a CA")]
    NotCa,
    #[error("serving certificate is not signed by the configured CA: {0}")]
    Untrusted(String),
    #[error("serving certificate does not contain DNS SAN {0}")]
    MissingDnsName(String),
    #[error("serving certificate is not valid for TLS server authentication")]
    MissingServerAuth,
}

impl CertHealthError {
    fn io(path: impl AsRef<Path>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.as_ref().display().to_string(),
            source,
        }
    }
}

/// Values exported by the sidecar for Prometheus and readiness decisions.
#[derive(Debug)]
pub struct CertHealthState {
    valid: AtomicBool,
    not_before: AtomicI64,
    not_after: AtomicI64,
}

impl Default for CertHealthState {
    fn default() -> Self {
        Self {
            valid: AtomicBool::new(false),
            not_before: AtomicI64::new(0),
            not_after: AtomicI64::new(0),
        }
    }
}

impl CertHealthState {
    pub fn replace(&self, replacement: &Self) {
        self.not_before.store(
            replacement.not_before.load(Ordering::Acquire),
            Ordering::Release,
        );
        self.not_after.store(
            replacement.not_after.load(Ordering::Acquire),
            Ordering::Release,
        );
        self.valid
            .store(replacement.valid.load(Ordering::Acquire), Ordering::Release);
    }

    pub fn invalidate(&self) {
        self.valid.store(false, Ordering::Release);
    }

    pub fn is_valid(&self) -> bool {
        self.valid.load(Ordering::Acquire)
    }

    pub fn remaining_ratio(&self, now: i64) -> f64 {
        let not_before = self.not_before.load(Ordering::Acquire);
        let not_after = self.not_after.load(Ordering::Acquire);
        let lifetime = not_after.saturating_sub(not_before);
        if lifetime <= 0 {
            return 0.0;
        }
        (not_after.saturating_sub(now) as f64 / lifetime as f64).clamp(0.0, 1.0)
    }

    pub fn prometheus(&self, now: i64) -> String {
        let valid = u8::from(self.is_valid());
        format!(
            "# HELP stellar_webhook_certificate_valid Whether the serving certificate passed all checks.\n\
             # TYPE stellar_webhook_certificate_valid gauge\n\
             stellar_webhook_certificate_valid {valid}\n\
             # HELP stellar_webhook_certificate_remaining_ratio Fraction of the certificate lifetime remaining.\n\
             # TYPE stellar_webhook_certificate_remaining_ratio gauge\n\
             stellar_webhook_certificate_remaining_ratio {:.6}\n",
            self.remaining_ratio(now)
        )
    }
}

fn pem_blocks(pem: &[u8], wanted_label: &str) -> Vec<Vec<u8>> {
    let text = String::from_utf8_lossy(pem);
    let mut blocks = Vec::new();
    let mut remaining = text.as_ref();
    while let Some(start) = remaining.find("-----BEGIN ") {
        let after_begin = &remaining[start + 11..];
        let Some(label_end) = after_begin.find("-----") else {
            break;
        };
        let label = &after_begin[..label_end];
        let body_start = label_end + 5;
        let Some(end_marker) = after_begin[body_start..].find("-----END ") else {
            break;
        };
        let end_start = body_start + end_marker + 11;
        let Some(end_label) = after_begin[end_start..].find("-----") else {
            break;
        };
        if label == wanted_label {
            let body = &after_begin[body_start..end_start - 11];
            if let Ok(der) = base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                body.chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>(),
            ) {
                blocks.push(der);
            }
        }
        remaining = &after_begin[end_start + end_label..];
    }
    blocks
}

/// Build a rustls server configuration from a PEM certificate chain and key.
///
/// This is shared by the webhook server and cert-health sidecar. rustls checks
/// that the public key in the leaf certificate matches the private key.
pub fn load_server_config(
    cert_path: impl AsRef<Path>,
    key_path: impl AsRef<Path>,
) -> Result<ServerConfig, CertHealthError> {
    let cert_path = cert_path.as_ref();
    let key_path = key_path.as_ref();
    let cert_pem = std::fs::read(cert_path).map_err(|e| CertHealthError::io(cert_path, e))?;
    let certs: Vec<CertificateDer<'static>> = pem_blocks(&cert_pem, "CERTIFICATE")
        .into_iter()
        .map(CertificateDer::from)
        .collect();
    if certs.is_empty() {
        return Err(CertHealthError::NoCertificate(
            cert_path.display().to_string(),
        ));
    }

    let key_pem = std::fs::read(key_path).map_err(|e| CertHealthError::io(key_path, e))?;
    let key_text = String::from_utf8_lossy(&key_pem);
    let key_label = key_text
        .find("-----BEGIN ")
        .and_then(|start| key_text.get(start + 11..))
        .and_then(|value| value.find("-----"))
        .map(|end| {
            key_text
                .split("-----BEGIN ")
                .nth(1)
                .unwrap_or("")
                .get(..end)
                .unwrap_or("")
        });
    let key = pem_blocks(&key_pem, "PRIVATE KEY")
        .into_iter()
        .chain(pem_blocks(&key_pem, "RSA PRIVATE KEY"))
        .chain(pem_blocks(&key_pem, "EC PRIVATE KEY"))
        .next()
        .map(|der| match key_label {
            Some("RSA PRIVATE KEY") => PrivateKeyDer::Pkcs1(der.into()),
            Some("EC PRIVATE KEY") => PrivateKeyDer::Sec1(der.into()),
            _ => PrivateKeyDer::Pkcs8(der.into()),
        })
        .ok_or_else(|| CertHealthError::NoPrivateKey(key_path.display().to_string()))?;

    ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| CertHealthError::InvalidServerIdentity(e.to_string()))?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| CertHealthError::InvalidServerIdentity(e.to_string()))
}

/// Decode the first certificate in a PEM buffer into a DER slice.
///
/// `x509_parser` returns an owned PEM block, so the DER bytes are copied out to
/// keep the caller independent of that temporary.
fn first_certificate_der(
    pem: &[u8],
    description: &str,
) -> Result<Vec<u8>, CertHealthError> {
    let (_, block) = x509_parser::pem::parse_x509_pem(pem)
        .map_err(|e| CertHealthError::Parse(format!("{description}: {e}")))?;
    Ok(block.contents)
}

/// Validate a leaf/CA pair against the exact service DNS name used by the API
/// server. Every error is actionable and causes readiness to fail closed.
pub fn validate_serving_certificate(
    cert_pem: &[u8],
    ca_pem: &[u8],
    expected_dns_name: &str,
) -> Result<CertHealthState, CertHealthError> {
    // Validate key/certificate consistency before inspecting the chain.
    // Callers using files do this through `load_server_config`; this protects
    // byte-oriented callers and makes the trust checks independently testable.
    let leaf_der = first_certificate_der(cert_pem, "serving certificate")?;
    let ca_der = first_certificate_der(ca_pem, "CA certificate")?;
    let (_, leaf) = X509Certificate::from_der(&leaf_der)
        .map_err(|e| CertHealthError::Parse(format!("serving certificate: {e}")))?;
    let (_, ca) = X509Certificate::from_der(&ca_der)
        .map_err(|e| CertHealthError::Parse(format!("CA certificate: {e}")))?;

    if !leaf.validity().is_valid() {
        return Err(CertHealthError::Expired);
    }
    if !ca.validity().is_valid() {
        return Err(CertHealthError::CaExpired);
    }
    if !ca.is_ca() {
        return Err(CertHealthError::NotCa);
    }

    leaf.verify_signature(Some(ca.public_key()))
        .map_err(|e| CertHealthError::Untrusted(e.to_string()))?;

    let san = leaf
        .subject_alternative_name()
        .map_err(|e| CertHealthError::Parse(format!("serving certificate SAN: {e}")))?
        .ok_or_else(|| CertHealthError::MissingDnsName(expected_dns_name.to_string()))?;
    let dns_name_matches = san.value.general_names.iter().any(|name| {
        matches!(name, GeneralName::DNSName(dns) if dns.eq_ignore_ascii_case(expected_dns_name))
    });
    if !dns_name_matches {
        return Err(CertHealthError::MissingDnsName(
            expected_dns_name.to_string(),
        ));
    }

    let server_auth = leaf
        .extended_key_usage()
        .map_err(|e| CertHealthError::Parse(format!("serving certificate EKU: {e}")))?
        .is_some_and(|eku| eku.value.server_auth);
    if !server_auth {
        return Err(CertHealthError::MissingServerAuth);
    }

    Ok(CertHealthState {
        valid: AtomicBool::new(true),
        not_before: AtomicI64::new(leaf.validity().not_before.timestamp()),
        not_after: AtomicI64::new(leaf.validity().not_after.timestamp()),
    })
}

/// Convenience validation for on-disk cert-manager Secrets.
pub fn validate_files(
    cert_path: impl AsRef<Path>,
    key_path: impl AsRef<Path>,
    ca_path: impl AsRef<Path>,
    expected_dns_name: &str,
) -> Result<CertHealthState, CertHealthError> {
    let cert_path = cert_path.as_ref();
    let key_path = key_path.as_ref();
    let ca_path = ca_path.as_ref();
    load_server_config(cert_path, key_path)?;
    let cert_pem = std::fs::read(cert_path).map_err(|e| CertHealthError::io(cert_path, e))?;
    let ca_pem = std::fs::read(ca_path).map_err(|e| CertHealthError::io(ca_path, e))?;
    validate_serving_certificate(&cert_pem, &ca_pem, expected_dns_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{
        BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose,
        IsCa, KeyPair, KeyUsagePurpose, SanType,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    fn issued_certificates() -> (String, String, String) {
        let mut ca_params = CertificateParams::default();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.distinguished_name = DistinguishedName::new();
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "stellar-webhook-ca");
        ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
        let ca_key = KeyPair::generate().unwrap();
        let ca = ca_params.self_signed(&ca_key).unwrap();

        let mut leaf_params =
            CertificateParams::new(vec!["stellar-webhook.stellar-webhook.svc".to_string()])
                .unwrap();
        leaf_params.distinguished_name = DistinguishedName::new();
        leaf_params
            .distinguished_name
            .push(DnType::CommonName, "stellar-webhook");
        leaf_params.subject_alt_names = vec![SanType::DnsName(
            "stellar-webhook.stellar-webhook.svc".try_into().unwrap(),
        )];
        leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        leaf_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        let leaf_key = KeyPair::generate().unwrap();
        let issuer = rcgen::Issuer::from_params(&ca_params, &ca_key);
        let leaf = leaf_params.signed_by(&leaf_key, &issuer).unwrap();

        (leaf.pem(), leaf_key.serialize_pem(), ca.pem())
    }

    #[test]
    fn validates_trusted_service_certificate() {
        let (cert, _key, ca) = issued_certificates();
        let state = validate_serving_certificate(
            cert.as_bytes(),
            ca.as_bytes(),
            "stellar-webhook.stellar-webhook.svc",
        )
        .expect("certificate should be valid");
        assert!(state.is_valid());
        assert!(state.remaining_ratio(now()) > 0.99);
    }

    #[test]
    fn rejects_untrusted_certificate() {
        let (cert, _key, _ca) = issued_certificates();
        let (_other_cert, _other_key, other_ca) = issued_certificates();
        let error = validate_serving_certificate(
            cert.as_bytes(),
            other_ca.as_bytes(),
            "stellar-webhook.stellar-webhook.svc",
        )
        .unwrap_err();
        assert!(matches!(error, CertHealthError::Untrusted(_)));
    }

    #[test]
    fn rejects_wrong_dns_name() {
        let (cert, _key, ca) = issued_certificates();
        let error = validate_serving_certificate(
            cert.as_bytes(),
            ca.as_bytes(),
            "different.stellar-webhook.svc",
        )
        .unwrap_err();
        assert!(matches!(error, CertHealthError::MissingDnsName(_)));
    }

    #[test]
    fn rejects_leaf_certificate_as_ca() {
        let (cert, _key, ca) = issued_certificates();
        let error = validate_serving_certificate(
            cert.as_bytes(),
            cert.as_bytes(),
            "stellar-webhook.stellar-webhook.svc",
        )
        .unwrap_err();
        assert!(matches!(error, CertHealthError::NotCa));
        assert!(!ca.is_empty());
    }

    #[test]
    fn detects_mismatched_private_key() {
        let (_cert, _key, _ca) = issued_certificates();
        let (_cert2, other_key, _ca2) = issued_certificates();
        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("tls.crt");
        let key_path = dir.path().join("tls.key");
        let ca_path = dir.path().join("ca.crt");
        let (cert, _key, ca) = issued_certificates();
        std::fs::write(&cert_path, cert).unwrap();
        std::fs::write(&key_path, other_key).unwrap();
        std::fs::write(&ca_path, ca).unwrap();
        let error = validate_files(
            &cert_path,
            &key_path,
            &ca_path,
            "stellar-webhook.stellar-webhook.svc",
        )
        .unwrap_err();
        assert!(matches!(error, CertHealthError::InvalidServerIdentity(_)));
    }

    #[test]
    fn remaining_ratio_tracks_expiry_horizons() {
        let mut state = CertHealthState::default();
        state.not_before.store(1_000, Ordering::Release);
        state.not_after.store(1_100, Ordering::Release);
        assert!((state.remaining_ratio(1_025) - 0.75).abs() < f64::EPSILON);
        assert!((state.remaining_ratio(1_075) - 0.25).abs() < f64::EPSILON);
        assert!((state.remaining_ratio(1_090) - 0.10).abs() < f64::EPSILON);
        assert_eq!(state.remaining_ratio(1_101), 0.0);
    }

    fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }
}
