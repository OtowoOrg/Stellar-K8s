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

/// Check if the passphrase secret has been rotated and trigger restart if needed.
///
/// Compares the current secret's resourceVersion with the observed version in status.
/// If they differ, patches the workload (StatefulSet/Deployment) with a restart annotation
/// and updates the status to track the new version.
pub async fn handle_passphrase_secret_rotation(
    client: &Client,
    node: &StellarNode,
    dry_run: bool,
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

    let current_rv = secret.resource_version();
    let observed_rv = node
        .status
        .as_ref()
        .and_then(|s| s.observed_passphrase_secret_version.as_deref());

    // If versions match, no rotation needed
    if observed_rv == current_rv.as_deref() {
        return Ok(false);
    }

    info!(
        "Passphrase secret {} was rotated (rv: {:?} -> {:?}), triggering rolling restart for {}/{}",
        secret_ref,
        observed_rv,
        current_rv,
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

    // Trigger rolling restart via pod template annotation
    let restart_annotation = "stellar.org/passphrase-rotated-at";
    let annotation_value = current_rv.as_deref().unwrap_or("unknown");
    let patch = json!({
        "spec": {
            "template": {
                "metadata": {
                    "annotations": {
                        restart_annotation: annotation_value
                    }
                }
            }
        }
    });

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

    let current_rv = secret.resource_version();
    let observed_rv = node
        .status
        .as_ref()
        .and_then(|s| s.observed_seed_secret_version.as_deref());

    // If versions match, no rotation needed
    if observed_rv == current_rv.as_deref() {
        return Ok(false);
    }

    info!(
        "Seed secret {} was rotated (rv: {:?} -> {:?}), triggering rolling restart for {}/{}",
        validator_config.seed_secret_ref,
        observed_rv,
        current_rv,
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

    // Trigger rolling restart via pod template annotation
    let restart_annotation = "stellar.org/seed-rotated-at";
    let annotation_value = current_rv.as_deref().unwrap_or("unknown");
    let patch = json!({
        "spec": {
            "template": {
                "metadata": {
                    "annotations": {
                        restart_annotation: annotation_value
                    }
                }
            }
        }
    });

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
