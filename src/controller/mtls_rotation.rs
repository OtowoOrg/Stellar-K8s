//! Dual-Key mTLS Rotation Strategy for Zero-Downtime Key Rotation
//!
//! Implements automated mTLS key rotation with support for multiple valid keys
//! during the rotation window, ensuring zero-downtime transitions.
//!
//! The rotation strategy:
//! 1. Generate new key pair and certificate
//! 2. Store both old and new certificates in the Secret (dual-key phase)
//! 3. Signal pods to reload configuration without restart
//! 4. Verify certificate validity before decommissioning old keys
//! 5. Remove old certificate after grace period

use crate::error::{Error, Result};
use k8s_openapi::api::core::v1::{Pod, Secret};
use kube::{
    api::{Api, ListParams, Patch, PatchParams},
    Client, ResourceExt,
};
use rcgen::{
    CertificateParams, DistinguishedName, ExtendedKeyUsagePurpose, Ia5String, IsCa, KeyPair,
    KeyUsagePurpose, SanType,
};
use std::collections::BTreeMap;
use tracing::{debug, info, warn};
use x509_parser::certificate::X509Certificate;
use x509_parser::pem::parse_x509_pem;
use x509_parser::prelude::FromDer;

/// Configuration for dual-key rotation strategy
#[derive(Debug, Clone)]
pub struct DualKeyRotationConfig {
    /// Grace period (in seconds) to keep old certificate active during rotation
    pub grace_period_secs: u64,
    /// Maximum time to wait for pods to reload configuration
    pub reload_timeout_secs: u64,
    /// Number of retries for pod reload signal
    pub reload_retries: u32,
}

impl Default for DualKeyRotationConfig {
    fn default() -> Self {
        Self {
            grace_period_secs: 300,  // 5 minutes
            reload_timeout_secs: 60, // 1 minute
            reload_retries: 3,
        }
    }
}

/// Represents a certificate in the rotation process
#[derive(Debug, Clone)]
pub struct CertificateEntry {
    pub cert_pem: Vec<u8>,
    pub key_pem: Vec<u8>,
    pub is_active: bool,
    pub created_at: i64,
}

/// Dual-key rotation state stored in Secret annotations
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct RotationState {
    pub current_cert_index: u32,
    pub previous_cert_index: Option<u32>,
    pub rotation_in_progress: bool,
    pub last_rotation_time: Option<i64>,
    pub grace_period_end: Option<i64>,
}

/// Start a dual-key rotation: generate new certificate and store both old and new
pub async fn start_dual_key_rotation(
    client: &Client,
    namespace: &str,
    secret_name: &str,
    ca_secret_name: &str,
    dns_names: Vec<String>,
    config: &DualKeyRotationConfig,
) -> Result<()> {
    let secrets: Api<Secret> = Api::namespaced(client.clone(), namespace);

    // Get current secret
    let current_secret = secrets.get(secret_name).await.map_err(Error::KubeError)?;
    let mut rotation_state: RotationState = current_secret
        .annotations()
        .get("stellar.org/rotation-state")
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();

    if rotation_state.rotation_in_progress {
        debug!("Rotation already in progress, skipping");
        return Ok(());
    }

    // Get CA certificate
    let ca_secret = secrets
        .get(ca_secret_name)
        .await
        .map_err(Error::KubeError)?;
    let ca_cert_pem = String::from_utf8(
        ca_secret
            .data
            .as_ref()
            .unwrap()
            .get("tls.crt")
            .unwrap()
            .0
            .clone(),
    )
    .unwrap();
    let ca_key_pem = String::from_utf8(
        ca_secret
            .data
            .as_ref()
            .unwrap()
            .get("tls.key")
            .unwrap()
            .0
            .clone(),
    )
    .unwrap();

    // Generate new certificate
    let ca_key_pair =
        KeyPair::from_pem(&ca_key_pem).map_err(|e| Error::ConfigError(e.to_string()))?;
    let mut ca_params = CertificateParams::new(vec!["stellar-operator-ca".to_string()])?;
    ca_params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_cert = ca_params
        .self_signed(&ca_key_pair)
        .map_err(|e| Error::ConfigError(e.to_string()))?;

    let mut params = CertificateParams::default();
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "stellar-operator");
    for dns in dns_names {
        params.subject_alt_names.push(SanType::DnsName(
            Ia5String::try_from(dns).map_err(|e| Error::ConfigError(e.to_string()))?,
        ));
    }
    params.key_usages.push(KeyUsagePurpose::DigitalSignature);
    params
        .extended_key_usages
        .push(ExtendedKeyUsagePurpose::ServerAuth);
    params
        .extended_key_usages
        .push(ExtendedKeyUsagePurpose::ClientAuth);

    let key_pair = KeyPair::generate().map_err(|e| Error::ConfigError(e.to_string()))?;
    let cert = params
        .signed_by(&key_pair, &ca_cert, &ca_key_pair)
        .map_err(|e| Error::ConfigError(e.to_string()))?;

    let new_cert_pem = cert.pem().into_bytes();
    let new_key_pem = key_pair.serialize_pem().into_bytes();

    // Store both old and new certificates (dual-key phase)
    let mut data = BTreeMap::new();
    data.insert("tls.crt".to_string(), new_cert_pem.clone());
    data.insert("tls.key".to_string(), new_key_pem.clone());
    data.insert("ca.crt".to_string(), ca_cert_pem.into_bytes());

    // Keep old certificate for grace period
    if let Some(old_cert) = current_secret.data.as_ref().and_then(|d| d.get("tls.crt")) {
        data.insert("tls.crt.old".to_string(), old_cert.0.clone());
    }
    if let Some(old_key) = current_secret.data.as_ref().and_then(|d| d.get("tls.key")) {
        data.insert("tls.key.old".to_string(), old_key.0.clone());
    }

    // Update rotation state
    rotation_state.previous_cert_index = Some(rotation_state.current_cert_index);
    rotation_state.current_cert_index = rotation_state.current_cert_index.wrapping_add(1);
    rotation_state.rotation_in_progress = true;
    rotation_state.last_rotation_time = Some(chrono::Utc::now().timestamp());
    rotation_state.grace_period_end =
        Some(chrono::Utc::now().timestamp() + config.grace_period_secs as i64);

    let mut secret = current_secret.clone();
    secret.data = Some(
        data.into_iter()
            .map(|(k, v)| (k, k8s_openapi::ByteString(v)))
            .collect(),
    );

    // Add rotation state annotation
    let mut annotations = secret.annotations().clone();
    annotations.insert(
        "stellar.org/rotation-state".to_string(),
        serde_json::to_string(&rotation_state).unwrap(),
    );
    secret.metadata.annotations = Some(annotations);

    secrets
        .patch(
            secret_name,
            &PatchParams::apply("stellar-operator").force(),
            &Patch::Apply(&secret),
        )
        .await
        .map_err(Error::KubeError)?;

    info!("Started dual-key rotation for {}", secret_name);
    Ok(())
}

/// Signal pods to reload configuration without restart
pub async fn signal_pod_reload(
    client: &Client,
    namespace: &str,
    label_selector: &str,
) -> Result<()> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let lp = ListParams::default().labels(label_selector);
    let pod_list = pods.list(&lp).await.map_err(Error::KubeError)?;

    let pod_items: Vec<_> = pod_list.items.into_iter().collect();
    for pod in pod_items {
        let pod_name = pod.metadata.name.clone().unwrap_or_default();
        debug!("Signaling pod {} to reload configuration", pod_name);

        // Add annotation to trigger reload
        let mut pod_patch = pod.clone();
        let mut annotations = pod_patch.metadata.annotations.clone().unwrap_or_default();
        annotations.insert(
            "stellar.org/config-reload".to_string(),
            chrono::Utc::now().timestamp().to_string(),
        );
        pod_patch.metadata.annotations = Some(annotations);

        pods.patch(
            &pod_name,
            &PatchParams::apply("stellar-operator").force(),
            &Patch::Apply(&pod_patch),
        )
        .await
        .map_err(Error::KubeError)?;
    }

    info!("Signaled pods to reload configuration");
    Ok(())
}

/// Verify certificate validity before decommissioning old keys
pub async fn verify_certificate_validity(
    client: &Client,
    namespace: &str,
    secret_name: &str,
) -> Result<bool> {
    let secrets: Api<Secret> = Api::namespaced(client.clone(), namespace);
    let secret = secrets.get(secret_name).await.map_err(Error::KubeError)?;

    let data = secret
        .data
        .as_ref()
        .ok_or_else(|| Error::ConfigError("Secret has no data".to_string()))?;

    let cert_pem = data
        .get("tls.crt")
        .ok_or_else(|| Error::ConfigError("Secret missing tls.crt".to_string()))?;

    // Parse and validate certificate
    match parse_x509_pem(&cert_pem.0) {
        Ok((_, pem)) => match X509Certificate::from_der(&pem.contents) {
            Ok((_, cert)) => {
                let validity = cert.validity();
                match validity.time_to_expiration() {
                    Some(duration) if duration.whole_seconds() > 0 => {
                        debug!("Certificate is valid");
                        Ok(true)
                    }
                    _ => {
                        warn!("Certificate is expired or invalid");
                        Ok(false)
                    }
                }
            }
            Err(e) => {
                warn!("Failed to parse certificate: {}", e);
                Ok(false)
            }
        },
        Err(e) => {
            warn!("Failed to parse PEM: {}", e);
            Ok(false)
        }
    }
}

/// Complete rotation by removing old certificate after grace period
pub async fn complete_rotation(client: &Client, namespace: &str, secret_name: &str) -> Result<()> {
    let secrets: Api<Secret> = Api::namespaced(client.clone(), namespace);
    let mut secret = secrets.get(secret_name).await.map_err(Error::KubeError)?;

    // Remove old certificate entries
    if let Some(ref mut data) = secret.data {
        data.remove("tls.crt.old");
        data.remove("tls.key.old");
    }

    // Update rotation state
    let mut rotation_state: RotationState = secret
        .annotations()
        .get("stellar.org/rotation-state")
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();

    rotation_state.rotation_in_progress = false;
    rotation_state.previous_cert_index = None;
    rotation_state.grace_period_end = None;

    let mut annotations = secret.annotations().clone();
    annotations.insert(
        "stellar.org/rotation-state".to_string(),
        serde_json::to_string(&rotation_state).unwrap(),
    );
    secret.metadata.annotations = Some(annotations);

    secrets
        .patch(
            secret_name,
            &PatchParams::apply("stellar-operator").force(),
            &Patch::Apply(&secret),
        )
        .await
        .map_err(Error::KubeError)?;

    info!("Completed dual-key rotation for {}", secret_name);
    Ok(())
}

/// Check if rotation grace period has expired
pub fn is_grace_period_expired(rotation_state: &RotationState) -> bool {
    if let Some(grace_end) = rotation_state.grace_period_end {
        chrono::Utc::now().timestamp() > grace_end
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rotation_state_default() {
        let state = RotationState::default();
        assert_eq!(state.current_cert_index, 0);
        assert_eq!(state.previous_cert_index, None);
        assert!(!state.rotation_in_progress);
    }

    #[test]
    fn test_grace_period_not_expired() {
        let state = RotationState {
            grace_period_end: Some(chrono::Utc::now().timestamp() + 300),
            ..Default::default()
        };
        assert!(!is_grace_period_expired(&state));
    }

    #[test]
    fn test_grace_period_expired() {
        let state = RotationState {
            grace_period_end: Some(chrono::Utc::now().timestamp() - 1),
            ..Default::default()
        };
        assert!(is_grace_period_expired(&state));
    }

    #[test]
    fn test_dual_key_rotation_config_default() {
        let config = DualKeyRotationConfig::default();
        assert_eq!(config.grace_period_secs, 300);
        assert_eq!(config.reload_timeout_secs, 60);
        assert_eq!(config.reload_retries, 3);
    }
}
