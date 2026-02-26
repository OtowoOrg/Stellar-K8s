//! mTLS Certificate Management for internal communication
//!
//! Handles CA creation and certificate issuance for the Operator REST API
//! and Stellar nodes. Supports automated rotation of server certificates
//! before expiration.

use crate::crd::StellarNode;
use crate::error::{Error, Result};
use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::{
    api::{Api, Patch, PatchParams},
    Client, Resource, ResourceExt,
};
use rcgen::{
    CertificateParams, DistinguishedName, ExtendedKeyUsagePurpose, Ia5String, IsCa, KeyPair,
    KeyUsagePurpose, SanType,
};
use std::collections::BTreeMap;
use std::time::Duration;
use tracing::{debug, info};
use x509_parser::certificate::X509Certificate;
use x509_parser::pem::parse_x509_pem;
use x509_parser::prelude::FromDer;

pub const CA_SECRET_NAME: &str = "stellar-operator-ca";
pub const SERVER_CERT_SECRET_NAME: &str = "stellar-operator-server-cert";

/// Default number of days before certificate expiration at which to trigger rotation.
pub const DEFAULT_CERT_ROTATION_THRESHOLD_DAYS: u32 = 30;

/// Ensure the CA exists in the cluster
pub async fn ensure_ca(client: &Client, namespace: &str) -> Result<()> {
    let secrets: Api<Secret> = Api::namespaced(client.clone(), namespace);

    if secrets.get(CA_SECRET_NAME).await.is_ok() {
        return Ok(());
    }

    // Generate new CA
    let mut params = CertificateParams::default();
    params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "stellar-operator-ca");
    params.key_usages.push(KeyUsagePurpose::DigitalSignature);
    params.key_usages.push(KeyUsagePurpose::KeyCertSign);
    params.key_usages.push(KeyUsagePurpose::CrlSign);

    let key_pair = KeyPair::generate().map_err(|e| Error::ConfigError(e.to_string()))?;
    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| Error::ConfigError(e.to_string()))?;

    let mut data = BTreeMap::new();
    data.insert("tls.crt".to_string(), cert.pem().into_bytes());
    data.insert("tls.key".to_string(), key_pair.serialize_pem().into_bytes());

    let secret = Secret {
        metadata: ObjectMeta {
            name: Some(CA_SECRET_NAME.to_string()),
            namespace: Some(namespace.to_string()),
            ..Default::default()
        },
        data: Some(
            data.into_iter()
                .map(|(k, v)| (k, k8s_openapi::ByteString(v)))
                .collect(),
        ),
        ..Default::default()
    };

    secrets
        .patch(
            CA_SECRET_NAME,
            &PatchParams::apply("stellar-operator").force(),
            &Patch::Apply(&secret),
        )
        .await
        .map_err(Error::KubeError)?;

    Ok(())
}

/// Ensure server certificate exists for the operator (creates only if missing).
pub async fn ensure_server_cert(
    client: &Client,
    namespace: &str,
    dns_names: Vec<String>,
) -> Result<()> {
    let secrets: Api<Secret> = Api::namespaced(client.clone(), namespace);

    if secrets.get(SERVER_CERT_SECRET_NAME).await.is_ok() {
        return Ok(());
    }

    generate_and_patch_server_cert_inner(&secrets, namespace, dns_names).await
}

/// Returns the time until the certificate expires, or `None` if already expired/invalid.
/// Uses the first certificate in the PEM if multiple are present.
pub fn cert_time_to_expiration(cert_pem: &[u8]) -> Result<Option<Duration>> {
    let (_, pem) = parse_x509_pem(cert_pem)
        .map_err(|e| Error::ConfigError(format!("Failed to parse PEM: {e}")))?;
    let (_, cert) = X509Certificate::from_der(&pem.contents)
        .map_err(|e| Error::ConfigError(format!("Failed to parse X.509 certificate: {e}")))?;
    let validity = cert.validity();
    let duration = validity.time_to_expiration();
    // x509-parser uses time::Duration; convert to std::time::Duration
    Ok(duration.map(|d| {
        let secs = d.whole_seconds().try_into().unwrap_or(0u64);
        Duration::from_secs(secs)
    }))
}

/// Check whether the current server certificate in the cluster is within the rotation threshold
/// (i.e. expires within `rotation_threshold_days` days). Returns true if rotation should be performed.
pub async fn server_cert_needs_rotation(
    client: &Client,
    namespace: &str,
    rotation_threshold_days: u32,
) -> Result<bool> {
    let secrets: Api<Secret> = Api::namespaced(client.clone(), namespace);
    let secret = match secrets.get(SERVER_CERT_SECRET_NAME).await {
        Ok(s) => s,
        Err(_) => return Ok(true), // No cert yet, needs creation (handled by ensure_server_cert)
    };
    let data = secret
        .data
        .as_ref()
        .ok_or_else(|| Error::ConfigError("Server cert secret has no data".to_string()))?;
    let cert_pem = data
        .get("tls.crt")
        .ok_or_else(|| Error::ConfigError("Server cert secret missing tls.crt".to_string()))?;
    let time_to_exp = cert_time_to_expiration(&cert_pem.0)?;
    let threshold = Duration::from_secs(rotation_threshold_days as u64 * 24 * 3600);
    match time_to_exp {
        None => Ok(true), // Expired or invalid, rotate
        Some(d) if d <= threshold => Ok(true),
        Some(_) => Ok(false),
    }
}

/// Generate a new server certificate and update the Secret (overwrites existing).
/// Used for rotation; for initial creation use `ensure_server_cert`.
pub async fn rotate_server_cert(
    client: &Client,
    namespace: &str,
    dns_names: Vec<String>,
) -> Result<()> {
    let secrets: Api<Secret> = Api::namespaced(client.clone(), namespace);
    generate_and_patch_server_cert_inner(&secrets, namespace, dns_names).await
}

async fn generate_and_patch_server_cert_inner(
    secrets: &Api<Secret>,
    namespace: &str,
    dns_names: Vec<String>,
) -> Result<()> {
    let ca_secret = secrets
        .get(CA_SECRET_NAME)
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

    let mut data = BTreeMap::new();
    data.insert("tls.crt".to_string(), cert.pem().into_bytes());
    data.insert("tls.key".to_string(), key_pair.serialize_pem().into_bytes());
    data.insert("ca.crt".to_string(), ca_cert_pem.into_bytes());

    let secret = Secret {
        metadata: ObjectMeta {
            name: Some(SERVER_CERT_SECRET_NAME.to_string()),
            namespace: Some(namespace.to_string()),
            ..Default::default()
        },
        data: Some(
            data.into_iter()
                .map(|(k, v)| (k, k8s_openapi::ByteString(v)))
                .collect(),
        ),
        ..Default::default()
    };

    secrets
        .patch(
            SERVER_CERT_SECRET_NAME,
            &PatchParams::apply("stellar-operator").force(),
            &Patch::Apply(&secret),
        )
        .await
        .map_err(Error::KubeError)?;

    Ok(())
}

/// If the server certificate is within the rotation threshold, generate a new one and update the Secret.
/// Returns `true` if rotation was performed, `false` otherwise.
pub async fn maybe_rotate_server_cert(
    client: &Client,
    namespace: &str,
    dns_names: Vec<String>,
    rotation_threshold_days: u32,
) -> Result<bool> {
    if !server_cert_needs_rotation(client, namespace, rotation_threshold_days).await? {
        debug!("Server certificate is still valid beyond threshold, skipping rotation");
        return Ok(false);
    }
    info!(
        "Server certificate within {} days of expiration or missing, rotating",
        rotation_threshold_days
    );
    rotate_server_cert(client, namespace, dns_names).await?;
    Ok(true)
}

/// Ensure client certificate exists for a specific node
pub async fn ensure_node_cert(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let node_name = node.name_any();
    let secret_name = format!("{node_name}-client-cert");
    let secrets: Api<Secret> = Api::namespaced(client.clone(), &namespace);

    if secrets.get(&secret_name).await.is_ok() {
        return Ok(());
    }

    let ca_secret = secrets
        .get(CA_SECRET_NAME)
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

    let ca_key_pair =
        KeyPair::from_pem(&ca_key_pem).map_err(|e| Error::ConfigError(e.to_string()))?;
    let mut ca_params = CertificateParams::new(vec!["stellar-operator-ca".to_string()])?;
    ca_params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_cert = ca_params
        .self_signed(&ca_key_pair)
        .map_err(|e| Error::ConfigError(e.to_string()))?;

    let mut params = CertificateParams::default();
    params.distinguished_name = DistinguishedName::new();
    params.distinguished_name.push(
        rcgen::DnType::CommonName,
        format!("stellar-node-{node_name}"),
    );
    params.key_usages.push(KeyUsagePurpose::DigitalSignature);
    params
        .extended_key_usages
        .push(ExtendedKeyUsagePurpose::ClientAuth);
    params
        .extended_key_usages
        .push(ExtendedKeyUsagePurpose::ServerAuth);

    let key_pair = KeyPair::generate().map_err(|e| Error::ConfigError(e.to_string()))?;
    let cert = params
        .signed_by(&key_pair, &ca_cert, &ca_key_pair)
        .map_err(|e| Error::ConfigError(e.to_string()))?;

    let mut data = BTreeMap::new();
    data.insert("tls.crt".to_string(), cert.pem().into_bytes());
    data.insert("tls.key".to_string(), key_pair.serialize_pem().into_bytes());
    data.insert("ca.crt".to_string(), ca_cert_pem.into_bytes());

    let secret = Secret {
        metadata: ObjectMeta {
            name: Some(secret_name.clone()),
            namespace: Some(namespace.to_string()),
            owner_references: Some(vec![
                k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference {
                    api_version: StellarNode::api_version(&()).to_string(),
                    kind: StellarNode::kind(&()).to_string(),
                    name: node_name.clone(),
                    uid: node.uid().unwrap_or_default(),
                    controller: Some(true),
                    block_owner_deletion: Some(true),
                },
            ]),
            ..Default::default()
        },
        data: Some(
            data.into_iter()
                .map(|(k, v)| (k, k8s_openapi::ByteString(v)))
                .collect(),
        ),
        ..Default::default()
    };

    secrets
        .patch(
            &secret_name,
            &PatchParams::apply("stellar-operator").force(),
            &Patch::Apply(&secret),
        )
        .await
        .map_err(Error::KubeError)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_self_signed_cert(not_before: (i32, u8, u8), not_after: (i32, u8, u8)) -> Vec<u8> {
        let mut params = CertificateParams::default();
        params.distinguished_name = DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "test");
        params.not_before = rcgen::date_time_ymd(not_before.0, not_before.1, not_before.2);
        params.not_after = rcgen::date_time_ymd(not_after.0, not_after.1, not_after.2);
        let cert = params.self_signed(&KeyPair::generate().unwrap()).unwrap();
        cert.pem().into_bytes()
    }

    #[test]
    fn cert_time_to_expiration_healthy_cert_beyond_threshold() {
        // Certificate valid from 2020 to 2030: rotation should be ignored when healthy
        let pem = make_self_signed_cert((2020, 1, 1), (2030, 1, 1));
        let time_to_exp = cert_time_to_expiration(&pem).unwrap();
        let thirty_days =
            Duration::from_secs(DEFAULT_CERT_ROTATION_THRESHOLD_DAYS as u64 * 24 * 3600);
        assert!(
            time_to_exp.is_some(),
            "healthy cert should have some time to expiration (got None - cert may be considered invalid by parser)"
        );
        assert!(
            time_to_exp.unwrap() > thirty_days,
            "cert with long validity should be beyond rotation threshold (ignored when healthy)"
        );
    }

    #[test]
    fn cert_time_to_expiration_expired_returns_none() {
        // Certificate already expired: rotation should be triggered (threshold "met" for rotation)
        let pem = make_self_signed_cert((2020, 1, 1), (2020, 6, 1));
        let time_to_exp = cert_time_to_expiration(&pem).unwrap();
        assert!(
            time_to_exp.is_none(),
            "expired cert should return None so rotation is performed"
        );
    }

    #[test]
    fn cert_time_to_expiration_near_expiry_within_threshold() {
        // Certificate expiring soon (not_after in the past from "now"): same as expired
        let pem = make_self_signed_cert((2020, 1, 1), (2020, 1, 2));
        let time_to_exp = cert_time_to_expiration(&pem).unwrap();
        assert!(
            time_to_exp.is_none(),
            "expired cert triggers rotation when threshold is met (any expired cert)"
        );
    }

    #[test]
    fn rotation_threshold_constant() {
        assert_eq!(DEFAULT_CERT_ROTATION_THRESHOLD_DAYS, 30);
    }
}
