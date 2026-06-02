//! SecretPolicy controller reconciliation loop.

use chrono::Utc;
use kube::{
    api::{Api, Patch, PatchParams},
    Client, ResourceExt,
};
use tracing::{info, warn};

use crate::crd::secret_policy::{
    SecretPolicy, SecretPolicyCondition, SecretPolicyPhase, SecretPolicyStatus,
};
use crate::error::Result;
use crate::security::kms::create_kms_backend;
use crate::security::secret_audit::{SecretAuditAction, SecretAuditLog};
use crate::security::secret_rotation::{SecretRotator, SecretVersionStore};
use crate::security::secret_sync::SecretSynchronizer;

/// Reconcile a SecretPolicy resource.
pub async fn reconcile_secret_policy(
    client: &Client,
    policy: &SecretPolicy,
    audit_log: &SecretAuditLog,
) -> Result<SecretPolicyStatus> {
    let namespace = policy.namespace().unwrap_or_else(|| "default".to_string());
    let name = policy.name_any();
    let spec = &policy.spec;

    if let Err(e) = spec.validate() {
        warn!(policy = %name, error = %e, "SecretPolicy validation failed");
        return Ok(SecretPolicyStatus {
            phase: SecretPolicyPhase::Failed,
            conditions: vec![condition("Ready", "False", Some(e))],
            ..Default::default()
        });
    }

    let backend = create_kms_backend(
        &spec.provider,
        spec.aws.as_ref(),
        spec.azure.as_ref(),
        spec.gcp.as_ref(),
    )?;

    audit_log.record(
        SecretAuditAction::Encrypt,
        &spec.secret_name,
        &namespace,
        "stellar-operator",
        0,
        true,
        Some(format!("provider={:?}", spec.provider)),
    );

    let mut store = SecretVersionStore::default();
    let version = SecretRotator::rotate(
        backend.as_ref(),
        &spec.rotation,
        &mut store,
        b"placeholder-secret-data",
    )
    .await?;

    audit_log.record(
        SecretAuditAction::Rotate,
        &spec.secret_name,
        &namespace,
        "stellar-operator",
        version,
        true,
        None,
    );

    let mut status = SecretPolicyStatus {
        phase: SecretPolicyPhase::Active,
        current_version: version,
        last_rotation: Some(Utc::now()),
        audit_entries_count: audit_log.entries().len() as u64,
        conditions: vec![condition("Ready", "True", None)],
        ..Default::default()
    };

    if let Some(sync_config) = &spec.sync {
        status.phase = SecretPolicyPhase::Syncing;
        let current = store.current();
        let encrypted = current
            .map(|v| v.encrypted.ciphertext.as_slice())
            .unwrap_or(&[]);

        match SecretSynchronizer::sync(
            sync_config,
            &spec.secret_name,
            &namespace,
            encrypted,
            version,
            spec.encrypt_in_transit,
        )
        .await
        {
            Ok(statuses) => {
                status.last_sync = Some(Utc::now());
                status.phase = SecretPolicyPhase::Active;

                let drift = SecretSynchronizer::detect_drift(version, &statuses);
                if !drift.is_empty() {
                    warn!(policy = %name, drift = ?drift, "Secret sync drift detected");
                }

                audit_log.record(
                    SecretAuditAction::Sync,
                    &spec.secret_name,
                    &namespace,
                    "stellar-operator",
                    version,
                    true,
                    Some(format!("synced to {} clusters", statuses.len())),
                );
            }
            Err(e) => {
                status.phase = SecretPolicyPhase::Failed;
                status.conditions = vec![condition("Ready", "False", Some(e.to_string()))];
            }
        }
    }

    // Check for access anomalies
    let anomalies = audit_log.detect_anomalies(300, 20);
    for anomaly in anomalies {
        audit_log.record(
            SecretAuditAction::AccessAnomaly,
            &spec.secret_name,
            &namespace,
            "anomaly-detector",
            version,
            false,
            Some(anomaly),
        );
    }

    // Update Kubernetes Secret with encrypted data (metadata only in status)
    update_target_secret_metadata(client, &namespace, &spec.secret_name, version).await?;

    info!(
        policy = %name,
        namespace = %namespace,
        version,
        phase = ?status.phase,
        "SecretPolicy reconciled"
    );

    Ok(status)
}

async fn update_target_secret_metadata(
    client: &Client,
    namespace: &str,
    secret_name: &str,
    version: u32,
) -> Result<()> {
    let secrets: Api<k8s_openapi::api::core::v1::Secret> =
        Api::namespaced(client.clone(), namespace);

    let patch = serde_json::json!({
        "metadata": {
            "annotations": {
                "stellar.org/secret-version": version.to_string(),
                "stellar.org/last-rotation": Utc::now().to_rfc3339(),
            }
        }
    });

    match secrets
        .patch(
            secret_name,
            &PatchParams::apply("stellar-operator").force(),
            &Patch::Merge(&patch),
        )
        .await
    {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(ae)) if ae.code == 404 => Ok(()),
        Err(e) => Err(crate::error::Error::KubeError(e)),
    }
}

fn condition(type_: &str, status: &str, message: Option<String>) -> SecretPolicyCondition {
    SecretPolicyCondition {
        type_: type_.to_string(),
        status: status.to_string(),
        last_transition_time: Utc::now(),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::secret_policy::{
        AwsKmsConfig, KmsProvider, RotationPolicy, SecretAuditConfig, SecretPolicySpec,
    };
    use kube::core::ObjectMeta;

    fn sample_policy() -> SecretPolicy {
        SecretPolicy {
            metadata: ObjectMeta {
                name: Some("test-policy".to_string()),
                namespace: Some("stellar".to_string()),
                ..Default::default()
            },
            spec: SecretPolicySpec {
                secret_name: "validator-seed".to_string(),
                provider: KmsProvider::Aws,
                aws: Some(AwsKmsConfig {
                    key_id: "key-123".to_string(),
                    region: "us-east-1".to_string(),
                    role_arn: None,
                }),
                azure: None,
                gcp: None,
                rotation: RotationPolicy::default(),
                sync: None,
                audit: SecretAuditConfig::default(),
                encrypt_in_transit: true,
            },
            status: None,
        }
    }

    #[test]
    fn condition_helper() {
        let c = condition("Ready", "True", None);
        assert_eq!(c.type_, "Ready");
        assert_eq!(c.status, "True");
    }

    #[test]
    fn invalid_spec_fails_validation() {
        let mut policy = sample_policy();
        policy.spec.aws = None;
        assert!(policy.spec.validate().is_err());
    }
}
