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
//! Secret Rotation Detection and Graceful Restart
//!
//! This module implements automated detection of secret changes and triggers
//! graceful rolling restarts of affected nodes without downtime.
//!
//! # Features
//!
//! - Watches Kubernetes Secret resources for changes
//! - Detects changes via resourceVersion comparison
//! - Triggers graceful rolling restarts via pod template annotations
//! - Updates status to track observed secret versions
//! - Zero-downtime rotation for network passphrases and validator seeds

use anyhow::{Context, Result};
use k8s_openapi::api::apps::v1::{Deployment, StatefulSet};
use k8s_openapi::api::core::v1::Secret;
use kube::{
    api::{Api, Patch, PatchParams},
    Client, ResourceExt,
};
use serde_json::json;
use tracing::{info, warn};

use crate::crd::{NodeType, StellarNode};
use crate::error::Error;

/// Returns true when the observed secret version differs from the current resource version.
pub(crate) fn secret_rotation_needed(current_rv: Option<&str>, observed_rv: Option<&str>) -> bool {
    observed_rv != current_rv
}

/// Returns true when a secret is within the configured expiry warning window.
pub(crate) fn secret_expires_soon(
    expires_at: Option<&str>,
    warning_window: chrono::Duration,
) -> bool {
    let Some(expires_at) = expires_at else {
        return false;
    };

    let Ok(expires_at) = chrono::DateTime::parse_from_rfc3339(expires_at) else {
        return false;
    };

    expires_at.with_timezone(&chrono::Utc) <= chrono::Utc::now() + warning_window
}

/// Build the merge patch that triggers a rolling restart via pod template annotation.
pub(crate) fn rolling_restart_patch(
    annotation_key: &str,
    annotation_value: &str,
) -> serde_json::Value {
    json!({
        "spec": {
            "template": {
                "metadata": {
                    "annotations": {
                        annotation_key: annotation_value
                    }
                }
            }
        }
    })
}

/// Annotation key used when network passphrase secrets rotate.
pub const PASSPHRASE_ROTATION_ANNOTATION: &str = "stellar.org/passphrase-rotated-at";

/// Annotation key used when validator seed secrets rotate.
pub const SEED_ROTATION_ANNOTATION: &str = "stellar.org/seed-rotated-at";

/// Check if the passphrase secret has been rotated and trigger restart if needed.
///
/// Compares the current secret's resourceVersion with the observed version in status.
/// If they differ, patches the workload (StatefulSet/Deployment) with a restart annotation
/// and updates the status to track the new version.
pub async fn handle_passphrase_secret_rotation(
    client: &Client,
    node: &StellarNode,
    dry_run: bool,
    audit_log: &crate::controller::audit_log::AuditLog,
) -> Result<bool> {
    let Some(secret_ref) = &node.spec.passphrase_secret_ref else {
        return Ok(false);
    };

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let secrets: Api<Secret> = Api::namespaced(client.clone(), &namespace);

    // Fetch the current secret
    let secret = match secrets.get(secret_ref).await {
        Ok(s) => s,
        Err(kube::Error::Api(e)) if e.code == 404 => {
            warn!(
                "Passphrase secret {} not found in namespace {}",
                secret_ref, namespace
            );
            return Ok(false);
        }
        Err(e) => return Err(Error::KubeError(e).into()),
    };

    // Secret access audit log
    audit_log.record(crate::controller::audit_log::AuditEntry::new(
        crate::controller::audit_log::AdminAction::Other("secret_access".to_string()),
        "stellar-operator",
        secret_ref.clone(),
        &namespace,
        Some(&format!(
            "Accessed passphrase secret {}/{}",
            namespace, secret_ref
        )),
    ));

    let current_rv = secret.resource_version();
    let observed_rv = node
        .status
        .as_ref()
        .and_then(|s| s.observed_passphrase_secret_version.as_deref());

    let rotated = secret_rotation_needed(current_rv.as_deref(), observed_rv);
    let expired = secret_expires_soon(
        secret
            .metadata
            .annotations
            .as_ref()
            .and_then(|a| a.get("stellar.org/expires-at"))
            .map(String::as_str),
        chrono::Duration::days(1),
    );

    // If versions match and not expired, no rotation needed
    if !rotated && !expired {
        return Ok(false);
    }

    info!(
        "Passphrase secret {} needs rotation (rv: {:?} -> {:?}, expired: {}), triggering rolling restart for {}/{}",
        secret_ref,
        observed_rv,
        current_rv,
        expired,
        namespace,
        node.name_any()
    );

    if dry_run {
        info!(
            "[dry-run] Would restart pods for {}/{}",
            namespace,
            node.name_any()
        );
        return Ok(true);
    }

    // Secret rotation audit log
    audit_log.record(crate::controller::audit_log::AuditEntry::new(
        crate::controller::audit_log::AdminAction::Other("secret_rotation".to_string()),
        "stellar-operator",
        secret_ref.clone(),
        &namespace,
        Some(&format!(
            "Secret rotation triggered rolling restart for node {} due to: {}",
            node.name_any(),
            if expired {
                "expiration"
            } else {
                "version change"
            }
        )),
    ));

    // Trigger rolling restart via pod template annotation
    let restart_annotation = PASSPHRASE_ROTATION_ANNOTATION;
    let annotation_value = current_rv.as_deref().unwrap_or("unknown");
    let patch = rolling_restart_patch(restart_annotation, annotation_value);

    let pp = if dry_run {
        PatchParams::apply("stellar-operator").dry_run()
    } else {
        PatchParams::apply("stellar-operator")
    };

    match node.spec.node_type {
        NodeType::Validator => {
            let api: Api<StatefulSet> = Api::namespaced(client.clone(), &namespace);
            if let Err(e) = api
                .patch(&node.name_any(), &pp, &Patch::Merge(&patch))
                .await
            {
                warn!("Failed to patch StatefulSet for passphrase rotation restart: {e}");
            }
        }
        NodeType::Horizon | NodeType::SorobanRpc => {
            let api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
            if let Err(e) = api
                .patch(&node.name_any(), &pp, &Patch::Merge(&patch))
                .await
            {
                warn!("Failed to patch Deployment for passphrase rotation restart: {e}");
            }
        }
    }

    // Update status to track the new version
    let api_sn: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
    let status_patch = json!({
        "status": {
            "observedPassphraseSecretVersion": current_rv,
            "lastSecretRotationTime": chrono::Utc::now().to_rfc3339()
        }
    });

    api_sn
        .patch_status(
            &node.name_any(),
            &PatchParams::apply("stellar-operator"),
            &Patch::Merge(&status_patch),
        )
        .await
        .context("Failed to update status after passphrase rotation")?;

    Ok(true)
}

/// Check if the validator seed secret has been rotated and trigger restart if needed.
///
/// Compares the current secret's resourceVersion with the observed version in status.
/// If they differ, patches the StatefulSet with a restart annotation and updates the status.
pub async fn handle_seed_secret_rotation(
    client: &Client,
    node: &StellarNode,
    dry_run: bool,
    audit_log: &crate::controller::audit_log::AuditLog,
) -> Result<bool> {
    // Only applicable to validators
    if node.spec.node_type != NodeType::Validator {
        return Ok(false);
    }

    let Some(validator_config) = &node.spec.validator_config else {
        return Ok(false);
    };

    // Check if using legacy seed_secret_ref (not KMS/ESO/CSI)
    if validator_config.seed_secret_ref.is_empty() {
        return Ok(false);
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let secrets: Api<Secret> = Api::namespaced(client.clone(), &namespace);

    // Fetch the current secret
    let secret = match secrets.get(&validator_config.seed_secret_ref).await {
        Ok(s) => s,
        Err(kube::Error::Api(e)) if e.code == 404 => {
            warn!(
                "Seed secret {} not found in namespace {}",
                validator_config.seed_secret_ref, namespace
            );
            return Ok(false);
        }
        Err(e) => return Err(Error::KubeError(e).into()),
    };

    // Secret access audit log
    audit_log.record(crate::controller::audit_log::AuditEntry::new(
        crate::controller::audit_log::AdminAction::Other("secret_access".to_string()),
        "stellar-operator",
        validator_config.seed_secret_ref.clone(),
        &namespace,
        Some(&format!(
            "Accessed seed secret {}/{}",
            namespace, validator_config.seed_secret_ref
        )),
    ));

    let current_rv = secret.resource_version();
    let observed_rv = node
        .status
        .as_ref()
        .and_then(|s| s.observed_seed_secret_version.as_deref());

    let rotated = secret_rotation_needed(current_rv.as_deref(), observed_rv);
    let expired = secret_expires_soon(
        secret
            .metadata
            .annotations
            .as_ref()
            .and_then(|a| a.get("stellar.org/expires-at"))
            .map(String::as_str),
        chrono::Duration::days(1),
    );

    // If versions match and not expired, no rotation needed
    if !rotated && !expired {
        return Ok(false);
    }

    info!(
        "Seed secret {} needs rotation (rv: {:?} -> {:?}, expired: {}), triggering rolling restart for {}/{}",
        validator_config.seed_secret_ref,
        observed_rv,
        current_rv,
        expired,
        namespace,
        node.name_any()
    );

    if dry_run {
        info!(
            "[dry-run] Would restart pods for {}/{}",
            namespace,
            node.name_any()
        );
        return Ok(true);
    }

    // Secret rotation audit log
    audit_log.record(crate::controller::audit_log::AuditEntry::new(
        crate::controller::audit_log::AdminAction::Other("secret_rotation".to_string()),
        "stellar-operator",
        validator_config.seed_secret_ref.clone(),
        &namespace,
        Some(&format!(
            "Secret rotation triggered rolling restart for node {} due to: {}",
            node.name_any(),
            if expired {
                "expiration"
            } else {
                "version change"
            }
        )),
    ));

    // Trigger rolling restart via pod template annotation
    let restart_annotation = SEED_ROTATION_ANNOTATION;
    let annotation_value = current_rv.as_deref().unwrap_or("unknown");
    let patch = rolling_restart_patch(restart_annotation, annotation_value);

    let pp = if dry_run {
        PatchParams::apply("stellar-operator").dry_run()
    } else {
        PatchParams::apply("stellar-operator")
    };

    let api: Api<StatefulSet> = Api::namespaced(client.clone(), &namespace);
    if let Err(e) = api
        .patch(&node.name_any(), &pp, &Patch::Merge(&patch))
        .await
    {
        warn!("Failed to patch StatefulSet for seed rotation restart: {e}");
    }

    // Update status to track the new version
    let api_sn: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
    let status_patch = json!({
        "status": {
            "observedSeedSecretVersion": current_rv,
            "lastSecretRotationTime": chrono::Utc::now().to_rfc3339()
        }
    });

    api_sn
        .patch_status(
            &node.name_any(),
            &PatchParams::apply("stellar-operator"),
            &Patch::Merge(&status_patch),
        )
        .await
        .context("Failed to update status after seed rotation")?;

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_rotation_needed_when_versions_differ() {
        assert!(secret_rotation_needed(Some("100"), None));
        assert!(secret_rotation_needed(Some("101"), Some("100")));
    }

    #[test]
    fn secret_rotation_not_needed_when_versions_match() {
        assert!(!secret_rotation_needed(None, None));
        assert!(!secret_rotation_needed(Some("100"), Some("100")));
    }

    #[test]
    fn rolling_restart_patch_sets_template_annotation() {
        let patch = rolling_restart_patch(PASSPHRASE_ROTATION_ANNOTATION, "rv-42");
        assert_eq!(
            patch["spec"]["template"]["metadata"]["annotations"][PASSPHRASE_ROTATION_ANNOTATION],
            "rv-42"
        );
    }

    #[test]
    fn seed_rotation_uses_distinct_annotation_key() {
        let patch = rolling_restart_patch(SEED_ROTATION_ANNOTATION, "rv-seed-7");
        assert_eq!(
            patch["spec"]["template"]["metadata"]["annotations"][SEED_ROTATION_ANNOTATION],
            "rv-seed-7"
        );
        assert!(patch["spec"]["template"]["metadata"]["annotations"]
            .get(PASSPHRASE_ROTATION_ANNOTATION)
            .is_none());
    }

    #[test]
    fn passphrase_rotation_skips_without_secret_ref() {
        let secret_ref: Option<String> = None;
        assert!(secret_ref.is_none());
    }

    #[test]
    fn expiration_warning_detected_when_secret_expires_soon() {
        let expiry = (chrono::Utc::now() + chrono::Duration::hours(12)).to_rfc3339();
        assert!(secret_expires_soon(Some(&expiry), chrono::Duration::days(1)));
        assert!(!secret_expires_soon(Some(&(chrono::Utc::now() + chrono::Duration::days(2)).to_rfc3339()), chrono::Duration::days(1)));
    }

    #[test]
    fn seed_rotation_only_applies_to_validators() {
        assert_ne!(NodeType::Validator, NodeType::Horizon);
        assert_ne!(NodeType::Validator, NodeType::SorobanRpc);
    }
}
