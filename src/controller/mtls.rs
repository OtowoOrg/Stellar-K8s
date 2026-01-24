//! mTLS Certificate Management for internal communication
//!
//! Handles CA creation and certificate issuance for the Operator REST API
//! and Stellar nodes.

use crate::crd::StellarNode;
use crate::error::{Error, Result};
use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::{
    api::{Api, Patch, PatchParams},
    Client, Resource, ResourceExt,
};
use rcgen::{
    Certificate, CertificateParams, DistinguishedName, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose, SanType,
};
use std::collections::BTreeMap;

pub const CA_SECRET_NAME: &str = "stellar-operator-ca";
pub const SERVER_CERT_SECRET_NAME: &str = "stellar-operator-server-cert";

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

    let cert = Certificate::from_params(params).map_err(|e| Error::ConfigError(e.to_string()))?;

    let mut data = BTreeMap::new();
    data.insert(
        "tls.crt".to_string(),
        cert.serialize_pem()
            .map_err(|e| Error::ConfigError(e.to_string()))?
            .into_bytes(),
    );
    data.insert(
        "tls.key".to_string(),
        cert.serialize_private_key_pem().into_bytes(),
    );

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

/// Ensure server certificate exists for the operator
pub async fn ensure_server_cert(
    client: &Client,
    namespace: &str,
    dns_names: Vec<String>,
) -> Result<()> {
    let secrets: Api<Secret> = Api::namespaced(client.clone(), namespace);

    if secrets.get(SERVER_CERT_SECRET_NAME).await.is_ok() {
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

    // In rcgen 0.11, loading CA from PEM is tricky.
    // We'll reconstruct the CA Certificate object using the known params and the loaded key.
    let mut ca_params = CertificateParams::default();
    ca_params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    ca_params.distinguished_name = DistinguishedName::new();
    ca_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "stellar-operator-ca");
    ca_params.key_pair = Some(ca_key_pair);

    let ca_cert =
        Certificate::from_params(ca_params).map_err(|e| Error::ConfigError(e.to_string()))?;

    let mut params = CertificateParams::default();
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "stellar-operator");
    for dns in dns_names {
        params.subject_alt_names.push(SanType::DnsName(dns));
    }
    params.key_usages.push(KeyUsagePurpose::DigitalSignature);
    params
        .extended_key_usages
        .push(ExtendedKeyUsagePurpose::ServerAuth);
    params
        .extended_key_usages
        .push(ExtendedKeyUsagePurpose::ClientAuth);

    let cert = Certificate::from_params(params).map_err(|e| Error::ConfigError(e.to_string()))?;
    let signed_cert = cert
        .serialize_pem_with_signer(&ca_cert)
        .map_err(|e| Error::ConfigError(e.to_string()))?;

    let mut data = BTreeMap::new();
    data.insert("tls.crt".to_string(), signed_cert.into_bytes());
    data.insert(
        "tls.key".to_string(),
        cert.serialize_private_key_pem().into_bytes(),
    );
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

/// Ensure client certificate exists for a specific node
pub async fn ensure_node_cert(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let node_name = node.name_any();
    let secret_name = format!("{}-client-cert", node_name);
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
    let mut ca_params = CertificateParams::default();
    ca_params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    ca_params.distinguished_name = DistinguishedName::new();
    ca_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "stellar-operator-ca");
    ca_params.key_pair = Some(ca_key_pair);

    let ca_cert =
        Certificate::from_params(ca_params).map_err(|e| Error::ConfigError(e.to_string()))?;

    let mut params = CertificateParams::default();
    params.distinguished_name = DistinguishedName::new();
    params.distinguished_name.push(
        rcgen::DnType::CommonName,
        format!("stellar-node-{}", node_name),
    );
    params.key_usages.push(KeyUsagePurpose::DigitalSignature);
    params
        .extended_key_usages
        .push(ExtendedKeyUsagePurpose::ClientAuth);
    params
        .extended_key_usages
        .push(ExtendedKeyUsagePurpose::ServerAuth);

    let cert = Certificate::from_params(params).map_err(|e| Error::ConfigError(e.to_string()))?;
    let signed_cert = cert
        .serialize_pem_with_signer(&ca_cert)
        .map_err(|e| Error::ConfigError(e.to_string()))?;

    let mut data = BTreeMap::new();
    data.insert("tls.crt".to_string(), signed_cert.into_bytes());
    data.insert(
        "tls.key".to_string(),
        cert.serialize_private_key_pem().into_bytes(),
    );
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
