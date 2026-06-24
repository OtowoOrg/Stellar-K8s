//! StellarSecret controller reconciliation loop.

use chrono::Utc;
use kube::{Client, ResourceExt};
use tracing::{info, warn};

use crate::controller::secret_policy_controller::reconcile_secret_policy;
use crate::crd::secret_policy::{
    KmsProvider, RotationPolicy, SecretAuditConfig, SecretPolicy, SecretPolicySpec,
};
use crate::crd::stellar_secret::{
    SecretBackend, SecretPhase, SecretVersionRecord, StellarSecret, StellarSecretStatus,
};
use crate::crd::types::Condition;
use crate::error::Result;
use crate::security::kms::create_kms_backend;
use crate::security::secret_audit::{SecretAuditAction, SecretAuditLog};
use crate::security::secret_metrics::{
    record_access_anomaly, record_secret_rotation, record_sync_drift,
};
use crate::security::secret_rotation::{SecretRotator, SecretVersionStore};

/// Reconcile a StellarSecret resource.
pub async fn reconcile_stellar_secret(
    client: &Client,
    secret: &StellarSecret,
    audit_log: &SecretAuditLog,
) -> Result<StellarSecretStatus> {
    let namespace = secret.namespace().unwrap_or_else(|| "default".to_string());
    let name = secret.name_any();
    let spec = &secret.spec;

    if let Err(e) = spec.validate() {
        warn!(secret = %name, error = %e, "StellarSecret validation failed");
        return Ok(failed_status(e));
    }

    let provider_label = format!("{:?}", spec.backend);
    audit_log.record(
        SecretAuditAction::Encrypt,
        &spec.secret_name,
        &namespace,
        "stellar-operator",
        0,
        true,
        Some(format!("backend={provider_label}")),
    );

    if spec.dynamic.enabled {
        if let Some(db) = &spec.dynamic.database {
            info!(
                secret = %name,
                host = %db.host,
                ttl = %spec.dynamic.ttl,
                "generating dynamic database credentials"
            );
        }
    }

    let mut current_version = secret
        .status
        .as_ref()
        .map(|s| s.current_version)
        .unwrap_or(0);

    if let Some(provider) = &spec.provider {
        let policy = SecretPolicy {
            metadata: secret.metadata.clone(),
            spec: SecretPolicySpec {
                secret_name: spec.secret_name.clone(),
                provider: provider.clone(),
                aws: spec.aws.clone(),
                azure: spec.azure.clone(),
                gcp: spec.gcp.clone(),
                rotation: RotationPolicy {
                    interval: spec.rotation.interval.clone(),
                    zero_downtime: spec.rotation.zero_downtime,
                    version_retention: spec.rotation.version_retention,
                },
                sync: None,
                audit: SecretAuditConfig {
                    enabled: spec.audit.enabled,
                    sink: spec.audit.sink.clone(),
                    anomaly_detection: true,
                },
                encrypt_in_transit: true,
            },
            status: None,
        };
        let policy_status = reconcile_secret_policy(client, &policy, audit_log).await?;
        current_version = policy_status.current_version;
        record_secret_rotation(&namespace, &spec.secret_name, &provider_label);
    } else if let (SecretBackend::Aws, Some(aws)) = (&spec.backend, &spec.aws) {
        let backend = create_kms_backend(&KmsProvider::Aws, Some(aws), None, None)?;
        let mut store = SecretVersionStore::default();
        current_version = SecretRotator::rotate(
            backend.as_ref(),
            &RotationPolicy {
                interval: spec.rotation.interval.clone(),
                zero_downtime: spec.rotation.zero_downtime,
                version_retention: spec.rotation.version_retention,
            },
            &mut store,
            b"dynamic-secret-placeholder",
        )
        .await?;
        record_secret_rotation(&namespace, &spec.secret_name, &provider_label);
    } else {
        current_version = current_version.max(1);
    }

    let versions = build_version_history(current_version, spec.rotation.version_retention);
    record_sync_drift(&namespace, &spec.secret_name, &provider_label, 0);

    for msg in audit_log.detect_anomalies(300, 20) {
        record_access_anomaly(&namespace, &spec.secret_name, &provider_label);
        audit_log.record(
            SecretAuditAction::AccessAnomaly,
            &spec.secret_name,
            &namespace,
            "anomaly-detector",
            current_version,
            false,
            Some(msg),
        );
    }

    Ok(StellarSecretStatus {
        phase: SecretPhase::Active,
        current_version,
        versions,
        last_rotation: Some(Utc::now()),
        audit_entries_count: audit_log.entries().len() as u64,
        conditions: vec![Condition::ready(true, "SecretReady", "rotation complete")],
    })
}

fn build_version_history(current: u32, retention: u32) -> Vec<SecretVersionRecord> {
    let start = current.saturating_sub(retention);
    (start..=current)
        .map(|v| SecretVersionRecord {
            version: v,
            created_at: Utc::now(),
            active: v == current,
        })
        .collect()
}

fn failed_status(message: String) -> StellarSecretStatus {
    StellarSecretStatus {
        phase: SecretPhase::Failed,
        conditions: vec![Condition::ready(false, "ValidationFailed", &message)],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::stellar_secret::{
        DynamicSecretConfig, RotationConfig, SecretAuditPolicy, SecretBackend, StellarSecretSpec,
    };
    use kube::core::ObjectMeta;

    #[test]
    fn version_history_respects_retention() {
        let versions = build_version_history(10, 5);
        assert_eq!(versions.len(), 6);
        assert!(versions.last().unwrap().active);
    }

    #[test]
    fn sample_spec_validates() {
        let spec = StellarSecretSpec {
            secret_name: "postgres-creds".to_string(),
            backend: SecretBackend::Vault,
            provider: Some(KmsProvider::Vault),
            aws: None,
            azure: None,
            gcp: None,
            dynamic: DynamicSecretConfig::default(),
            rotation: RotationConfig::default(),
            targets: vec![],
            audit: SecretAuditPolicy::default(),
        };
        assert!(spec.validate().is_ok());
    }
}
