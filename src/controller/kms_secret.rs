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
//! `src/controller/kms_secret.rs`
//!
//! Reconciles the KMS / Vault / CSI secret integration for Validator nodes.
//!
//! This module is called from `apply_stellar_node` (in `src/controller/reconciler.rs`)
//! **before** `resources::ensure_statefulset` so that by the time the pod spec
//! is built, the seed is either already available in a Kubernetes Secret (ESO
//! path) or will be mounted via CSI (CSI path).
//!
//! # What this module does
//!
//! | Backend    | What the reconciler creates              | How the pod reads it       |
//! |------------|------------------------------------------|----------------------------|
//! | `LocalRef` | nothing — Secret already exists          | env var from `secretKeyRef`|
//! | `ExternalRef` | `ExternalSecret` CR (ESO managed)     | env var from `secretKeyRef`|
//! | `CsiRef`   | nothing — `SecretProviderClass` pre-exists | file mount via CSI volume |
//! | `VaultRef` | Vault Agent Injector annotations only      | `STELLAR_SEED_FILE` under `/vault/secrets/` |
//!
//! # Security guarantees
//!
//! - The seed value is **never** read, logged, or stored by this module.
//! - Log messages only reference secret **names** and **resource types**.
//! - Status conditions reference resource names, never values.

use k8s_openapi::api::apps::v1::StatefulSet;
use k8s_openapi::api::core::v1::{
    CSIVolumeSource, EnvVar, EnvVarSource, Pod, SecretKeySelector, Volume, VolumeMount,
};
use kube::{
    api::{Api, ListParams, Patch, PatchParams},
    Client, ResourceExt,
};
use serde_json::json;
use tracing::{info, warn};

use std::collections::BTreeMap;

use crate::crd::{
    seed_secret::{
        CsiSecretRef, ExternalSecretRef, SeedSecretSource, VaultSecretRef, DEFAULT_SEED_KEY,
        LEGACY_SEED_KEY,
    },
    StellarNode,
};
use crate::error::{Error, Result};

// ── ESO API group / version ──────────────────────────────────────────────────
// We drive ESO via raw JSON patches rather than generated structs so that
// users do not need to have ESO installed to compile the operator.  The
// operator will gracefully degrade if ESO CRDs are absent (see below).

const ESO_API_VERSION: &str = "external-secrets.io/v1beta1";
const ESO_KIND: &str = "ExternalSecret";

/// Name of the Kubernetes Secret that ESO produces for a given node.
/// This is the name injected into the pod as `secretKeyRef`.
pub fn eso_target_secret_name(node_name: &str) -> String {
    format!("{node_name}-seed")
}

// ============================================================================
// Entry point called from the reconcile loop
// ============================================================================

/// Reconcile the seed secret for a validator node.
///
/// Returns a [`SeedInjectionSpec`] that the pod-builder uses to configure the
/// container — either an env-var reference or a CSI volume mount.
///
/// This function **never** logs or returns the secret value itself.
pub async fn reconcile_seed_secret(
    client: &Client,
    node: &StellarNode,
) -> Result<SeedInjectionSpec> {
    let vc = match &node.spec.validator_config {
        Some(vc) => vc,
        None => {
            return Err(Error::ConfigError(
                "reconcile_seed_secret called on non-validator node".to_string(),
            ))
        }
    };

    let source = match vc.resolve_seed_source() {
        Some(s) => s,
        None => {
            return Err(Error::ValidationError(
                "No seed source configured on ValidatorConfig".to_string(),
            ))
        }
    };

    // Warn loudly if a plain local Secret is used outside dev
    if source.is_local() {
        warn!(
            node = %node.name_any(),
            namespace = %node.namespace().unwrap_or_default(),
            "ValidatorConfig is using a plain Kubernetes Secret as the seed source. \
             This is NOT recommended for production. Use seedSecretSource.externalRef, \
             csiRef, or vaultRef instead."
        );
    }

    info!(
        node = %node.name_any(),
        source = source.describe(),
        "Reconciling seed secret"
    );

    match &source {
        SeedSecretSource {
            local_ref: Some(lr),
            ..
        } => {
            // Prefer STELLAR_CORE_SEED; fall back to the legacy SEED key when
            // the referenced Secret only contains SEED. Never log the value.
            let secret_key =
                resolve_secret_key(client, node, &lr.name, lr.effective_key()).await?;
            Ok(SeedInjectionSpec::env_from_secret(
                lr.name.clone(),
                secret_key,
            ))
        }

        SeedSecretSource {
            external_ref: Some(er),
            ..
        } => {
            let secret_name = reconcile_external_secret(client, node, er).await?;
            Ok(SeedInjectionSpec::env_from_secret(
                secret_name,
                DEFAULT_SEED_KEY.to_string(),
            ))
        }

        SeedSecretSource {
            csi_ref: Some(cr), ..
        } => Ok(SeedInjectionSpec::csi_mount(cr.clone())),

        SeedSecretSource {
            vault_ref: Some(vr),
            ..
        } => Ok(SeedInjectionSpec::vault_agent(vr.clone())),

        _ => Err(Error::ValidationError(
            "SeedSecretSource has no variant set — this should have been caught by validation"
                .to_string(),
        )),
    }
}

/// Build Vault Agent Injector annotations for a [`VaultSecretRef`].
pub fn vault_agent_annotations(vr: &VaultSecretRef) -> BTreeMap<String, String> {
    let file = vr.effective_secret_file_name();
    let mut m = BTreeMap::new();
    m.insert(
        "vault.hashicorp.com/agent-inject".to_string(),
        "true".to_string(),
    );
    m.insert(
        "vault.hashicorp.com/agent-pre-populate-only".to_string(),
        "false".to_string(),
    );
    m.insert(
        format!("vault.hashicorp.com/agent-inject-secret-{file}"),
        vr.secret_path.clone(),
    );
    if let Some(tpl) = &vr.template {
        m.insert(
            format!("vault.hashicorp.com/agent-inject-template-{file}"),
            tpl.clone(),
        );
    } else {
        let key = vr.secret_key.as_deref().unwrap_or("seed");
        let tpl = format!(
            "{{{{ with secret \"{}\" }}}}\n{{{{ index .Data.data \"{}\" }}}}\n{{{{ end }}}}",
            vr.secret_path, key
        );
        m.insert(
            format!("vault.hashicorp.com/agent-inject-template-{file}"),
            tpl,
        );
    }
    m.insert("vault.hashicorp.com/role".to_string(), vr.role.clone());
    for pair in &vr.extra_pod_annotations {
        m.insert(pair.name.clone(), pair.value.clone());
    }
    m
}

// ============================================================================
// Seed key resolution (Issue #1554)
// ============================================================================

/// Resolve which key within a Kubernetes Secret holds the seed value.
///
/// Precedence:
/// 1. The explicitly configured key (`configured_key`), if set.
/// 2. `STELLAR_CORE_SEED` if present in the Secret.
/// 3. Legacy `SEED` if present (logs a deprecation warning).
/// 4. `STELLAR_CORE_SEED` as the default when the Secret cannot be read or
///    contains neither key (the pod will surface a clear `CreateContainerConfigError`).
///
/// The secret **value** is never read or logged — only key names are inspected.
pub async fn resolve_secret_key(
    client: &Client,
    node: &StellarNode,
    secret_name: &str,
    configured_key: &str,
) -> Result<String> {
    // An explicit, non-default key always wins.
    if configured_key != DEFAULT_SEED_KEY && configured_key != LEGACY_SEED_KEY {
        return Ok(configured_key.to_string());
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let secrets: Api<k8s_openapi::api::core::v1::Secret> =
        Api::namespaced(client.clone(), &namespace);

    let keys: Option<Vec<String>> = match secrets.get(secret_name).await {
        Ok(secret) => secret
            .data
            .map(|d| d.keys().cloned().collect::<Vec<_>>()),
        Err(kube::Error::Api(ae)) if ae.code == 404 => {
            // Secret not created yet (e.g. ESO still syncing). Default to the
            // preferred key; the pod will retry once the Secret exists.
            return Ok(DEFAULT_SEED_KEY.to_string());
        }
        Err(e) => return Err(Error::KubeError(e)),
    };

    let keys = match keys {
        Some(k) => k,
        None => return Ok(DEFAULT_SEED_KEY.to_string()),
    };

    let has_preferred = keys.iter().any(|k| k == DEFAULT_SEED_KEY);
    let has_legacy = keys.iter().any(|k| k == LEGACY_SEED_KEY);

    if has_preferred {
        // Preferred key present — use it. If the legacy key is also present we
        // still use the preferred key and do not emit duplicates.
        Ok(DEFAULT_SEED_KEY.to_string())
    } else if has_legacy {
        warn!(
            node = %node.name_any(),
            namespace = %namespace,
            secret = %secret_name,
            legacy_key = %LEGACY_SEED_KEY,
            preferred_key = %DEFAULT_SEED_KEY,
            "Secret uses the deprecated 'SEED' key. Falling back to it for now, \
             but please migrate to 'STELLAR_CORE_SEED' (see scripts/migrate-seed-key.sh)."
        );
        Ok(LEGACY_SEED_KEY.to_string())
    } else {
        // Neither key present — default to the preferred key so the error
        // message from the kubelet references the standardized name.
        Ok(DEFAULT_SEED_KEY.to_string())
    }
}

// ============================================================================
// ESO reconciliation
// ============================================================================

/// Ensures an `ExternalSecret` CR exists and is syncing correctly.
/// Returns the name of the Kubernetes Secret that ESO will produce.
async fn reconcile_external_secret(
    client: &Client,
    node: &StellarNode,
    er: &ExternalSecretRef,
) -> Result<String> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let node_name = node.name_any();
    let target_secret_name = eso_target_secret_name(&node_name);

    // Build the ExternalSecret manifest as raw JSON.
    // We intentionally avoid generated ESO structs to keep the operator
    // compilable even without ESO in the cluster.
    let refresh = er.refresh_interval.as_deref().unwrap_or("1h");

    let remote_ref = if let Some(prop) = &er.remote_property {
        json!({
            "key": er.remote_key,
            "property": prop
        })
    } else {
        json!({ "key": er.remote_key })
    };

    let external_secret = json!({
        "apiVersion": ESO_API_VERSION,
        "kind": ESO_KIND,
        "metadata": {
            "name": er.name,
            "namespace": namespace,
            // Owner reference so the ExternalSecret is deleted with the StellarNode
            "ownerReferences": [{
                "apiVersion": "stellar.org/v1alpha1",
                "kind": "StellarNode",
                "name": node_name,
                "uid": node.metadata.uid.as_deref().unwrap_or(""),
                "blockOwnerDeletion": true,
                "controller": true,
            }]
        },
        "spec": {
            "refreshInterval": refresh,
            "secretStoreRef": {
                "name": er.secret_store_ref.name,
                "kind": er.secret_store_ref.kind,
            },
            "target": {
                "name": target_secret_name,
                // ESO owns creation; deletion policy Retain so the Secret
                // survives a brief ESO outage
                "creationPolicy": "Owner",
                "deletionPolicy": "Retain",
            },
            "data": [{
                // Inside the produced Secret the key is always STELLAR_CORE_SEED
                // so the pod injection is uniform regardless of what the remote
                // backend calls it.
                "secretKey": DEFAULT_SEED_KEY,
                "remoteRef": remote_ref,
            }]
        }
    });

    // Use server-side apply so the patch is idempotent.
    // If ESO CRDs are not installed this will fail with a 404 — we surface that
    // as a clear error rather than panicking.
    let api: Api<k8s_openapi::api::core::v1::ConfigMap> =
        Api::namespaced(client.clone(), &namespace);

    // Use the dynamic API to apply the ExternalSecret.
    // We reuse the kube DynamicObject path via raw JSON patch.
    let _dynamic_api = kube::api::DynamicObject::new(
        &er.name,
        &kube::discovery::ApiResource {
            group: "external-secrets.io".to_string(),
            version: "v1beta1".to_string(),
            api_version: ESO_API_VERSION.to_string(),
            kind: ESO_KIND.to_string(),
            plural: "externalsecrets".to_string(),
        },
    );

    let typed_api: Api<kube::api::DynamicObject> = Api::namespaced_with(
        client.clone(),
        &namespace,
        &kube::discovery::ApiResource {
            group: "external-secrets.io".to_string(),
            version: "v1beta1".to_string(),
            api_version: ESO_API_VERSION.to_string(),
            kind: ESO_KIND.to_string(),
            plural: "externalsecrets".to_string(),
        },
    );

    let _ = api; // suppress unused warning — we use typed_api below

    match typed_api
        .patch(
            &er.name,
            &PatchParams::apply("stellar-operator").force(),
            &Patch::Apply(&external_secret),
        )
        .await
    {
        Ok(_) => {
            info!(
                node = %node_name,
                external_secret = %er.name,
                target_secret = %target_secret_name,
                store = %er.secret_store_ref.name,
                "ExternalSecret applied successfully"
            );
        }
        Err(kube::Error::Api(ae)) if ae.code == 404 => {
            return Err(Error::ConfigError(format!(
                "ExternalSecret CRD not found (ESO is not installed). \
                 Install External Secrets Operator or switch to csiRef. \
                 Error: {ae}"
            )));
        }
        Err(e) => return Err(Error::KubeError(e)),
    }

    // Check if ESO has already synced the Secret (best-effort; not blocking)
    check_eso_sync_status(client, node, &er.name, &namespace).await;

    Ok(target_secret_name)
}

/// Logs the current ESO sync status without blocking reconciliation.
/// We don't fail here — ESO may still be pulling; the pod will start once
/// the Secret exists.
async fn check_eso_sync_status(
    client: &Client,
    node: &StellarNode,
    es_name: &str,
    namespace: &str,
) {
    let typed_api: Api<kube::api::DynamicObject> = Api::namespaced_with(
        client.clone(),
        namespace,
        &kube::discovery::ApiResource {
            group: "external-secrets.io".to_string(),
            version: "v1beta1".to_string(),
            api_version: ESO_API_VERSION.to_string(),
            kind: ESO_KIND.to_string(),
            plural: "externalsecrets".to_string(),
        },
    );

    match typed_api.get(es_name).await {
        Ok(es) => {
            // Check .status.conditions[type=Ready]
            if let Some(conditions) = es
                .data
                .get("status")
                .and_then(|s| s.get("conditions"))
                .and_then(|c| c.as_array())
            {
                for cond in conditions {
                    let type_ = cond.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    let status = cond
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown");
                    let message = cond.get("message").and_then(|v| v.as_str()).unwrap_or("");

                    if type_ == "Ready" {
                        if status == "True" {
                            info!(
                                node = %node.name_any(),
                                external_secret = %es_name,
                                "ExternalSecret is Ready — seed synced from remote backend"
                            );
                        } else {
                            warn!(
                                node = %node.name_any(),
                                external_secret = %es_name,
                                status = %status,
                                // message may contain provider error details but
                                // NOT the secret value — safe to log
                                message = %message,
                                "ExternalSecret not yet Ready; will retry on next reconcile"
                            );
                        }
                    }
                }
            }
        }
        Err(e) => {
            warn!(
                node = %node.name_any(),
                external_secret = %es_name,
                error = %e,
                "Could not read ExternalSecret status"
            );
        }
    }
}

// ============================================================================
// Pod injection spec — returned to the pod builder
// ============================================================================

/// Describes how the seed should be injected into the validator container.
///
/// The pod-builder in `resources::ensure_statefulset` consumes this to
/// configure the container spec without ever seeing the secret value.
#[derive(Clone, Debug)]
pub enum SeedInjectionSpec {
    /// Inject the seed as an environment variable sourced from a Kubernetes Secret.
    ///
    /// Used for both `localRef` (existing Secret) and `externalRef`
    /// (ESO-produced Secret).
    EnvFromSecret {
        /// Name of the Kubernetes Secret.
        secret_name: String,
        /// Key within the Secret.
        secret_key: String,
    },

    /// Mount the seed as a file via the Secrets Store CSI Driver.
    ///
    /// The pod-builder will add a CSI volume + mount and set
    /// `STELLAR_SEED_FILE` to the file path instead of `STELLAR_CORE_SEED`.
    CsiMount { config: CsiSecretRef },

    /// HashiCorp Vault Agent Injector: annotations + `STELLAR_SEED_FILE` under `/vault/secrets/`.
    VaultAgent {
        config: VaultSecretRef,
        pod_annotations: BTreeMap<String, String>,
    },
}

impl SeedInjectionSpec {
    fn env_from_secret(secret_name: String, secret_key: String) -> Self {
        Self::EnvFromSecret {
            secret_name,
            secret_key,
        }
    }

    fn csi_mount(config: CsiSecretRef) -> Self {
        Self::CsiMount { config }
    }

    fn vault_agent(config: VaultSecretRef) -> Self {
        let pod_annotations = vault_agent_annotations(&config);
        Self::VaultAgent {
            config,
            pod_annotations,
        }
    }

    /// Pod template annotations (Vault Agent only).
    pub fn pod_annotations(&self) -> Option<&BTreeMap<String, String>> {
        match self {
            Self::VaultAgent {
                pod_annotations, ..
            } => Some(pod_annotations),
            _ => None,
        }
    }

    /// Whether the operator should roll pods when Vault secret version changes.
    pub fn vault_restart_on_rotation(&self) -> bool {
        match self {
            Self::VaultAgent { config, .. } => config.restart_on_secret_rotation,
            _ => false,
        }
    }

    /// Annotation key the injector sets for secret versioning (best-effort).
    pub fn vault_version_annotation_key(&self) -> Option<String> {
        match self {
            Self::VaultAgent { config, .. } => Some(format!(
                "vault.hashicorp.com/secret-version-{}",
                config.effective_secret_file_name()
            )),
            _ => None,
        }
    }

    // ── Helpers for the pod builder ──────────────────────────────────────────

    /// Returns the `EnvVar` entries to add to the container spec.
    ///
    /// - `EnvFromSecret` → one env var: `STELLAR_CORE_SEED` from secretKeyRef.
    ///   The env var name is always the standardized `STELLAR_CORE_SEED`; the
    ///   `secret_key` selects which key inside the Secret is read (which may be
    ///   the legacy `SEED` during migration). Exactly one env var is emitted,
    ///   so no duplicates are produced when both keys exist.
    /// - `CsiMount`      → one env var: `STELLAR_SEED_FILE` pointing at mount
    pub fn env_vars(&self) -> Vec<EnvVar> {
        match self {
            Self::VaultAgent { config, .. } => {
                let path = format!("/vault/secrets/{}", config.effective_secret_file_name());
                vec![EnvVar {
                    name: "STELLAR_SEED_FILE".to_string(),
                    value: Some(path),
                    ..Default::default()
                }]
            }
            Self::EnvFromSecret {
                secret_name,
                secret_key,
            } => vec![EnvVar {
                name: DEFAULT_SEED_KEY.to_string(),
                value_from: Some(EnvVarSource {
                    secret_key_ref: Some(SecretKeySelector {
                        name: Some(secret_name.clone()),
                        key: secret_key.clone(),
                        optional: Some(false),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }],

            Self::CsiMount { config } => vec![EnvVar {
                name: "STELLAR_SEED_FILE".to_string(),
                value: Some(config.seed_file_path()),
                ..Default::default()
            }],
        }
    }

    /// Returns any `VolumeMount` entries to add to the container spec.
    /// Only non-empty for the `CsiMount` variant.
    pub fn volume_mounts(&self) -> Vec<VolumeMount> {
        match self {
            Self::CsiMount { config } => vec![VolumeMount {
                name: "stellar-seed-csi".to_string(),
                mount_path: config.effective_mount_path().to_string(),
                read_only: Some(true),
                ..Default::default()
            }],
            _ => vec![],
        }
    }

    /// Returns any `Volume` entries to add to the pod spec.
    /// Only non-empty for the `CsiMount` variant.
    pub fn volumes(&self) -> Vec<Volume> {
        match self {
            Self::CsiMount { config } => vec![Volume {
                name: "stellar-seed-csi".to_string(),
                csi: Some(CSIVolumeSource {
                    driver: "secrets-store.csi.k8s.io".to_string(),
                    read_only: Some(true),
                    volume_attributes: Some(
                        [(
                            "secretProviderClass".to_string(),
                            config.secret_provider_class_name.clone(),
                        )]
                        .into_iter()
                        .collect(),
                    ),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            _ => vec![],
        }
    }

    /// A safe description string for logs/events — contains NO secret values.
    pub fn describe(&self) -> String {
        match self {
            Self::EnvFromSecret { secret_name, .. } => {
                format!("env-var from Secret '{secret_name}'")
            }
            Self::CsiMount { config } => format!(
                "CSI mount (SecretProviderClass='{}', path='{}')",
                config.secret_provider_class_name,
                config.effective_mount_path()
            ),
            Self::VaultAgent { config, .. } => format!(
                "Vault Agent Injector (role='{}', path='{}')",
                config.role, config.secret_path
            ),
        }
    }
}

/// When Vault rotates a secret, the Agent updates pod annotations; roll pods so stellar-core reloads.
pub async fn reconcile_vault_secret_rotation(
    client: &Client,
    node: &StellarNode,
    seed: Option<&SeedInjectionSpec>,
) -> Result<()> {
    let inj = match seed {
        Some(s) if s.vault_restart_on_rotation() => s,
        _ => return Ok(()),
    };
    let ver_key = match inj.vault_version_annotation_key() {
        Some(k) => k,
        None => return Ok(()),
    };
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let selector = format!("app.kubernetes.io/instance={}", node.name_any());
    let pods: Api<Pod> = Api::namespaced(client.clone(), &namespace);
    let list = pods
        .list(&ListParams::default().labels(&selector))
        .await
        .map_err(Error::KubeError)?;

    let mut observed: Option<String> = None;
    for p in list.items {
        if let Some(ann) = p.metadata.annotations.as_ref() {
            if let Some(v) = ann.get(&ver_key) {
                if !v.is_empty() {
                    observed = Some(v.clone());
                    break;
                }
            }
        }
    }
    let obs_str = match &observed {
        Some(s) => s.as_str(),
        None => return Ok(()),
    };

    let current = node
        .status
        .as_ref()
        .and_then(|st| st.vault_observed_secret_version.as_deref());

    if current == Some(obs_str) {
        return Ok(());
    }

    let sts_api: Api<StatefulSet> = Api::namespaced(client.clone(), &namespace);
    let name = node.name_any();
    let restart_ts = chrono::Utc::now().to_rfc3339();
    let patch = json!({
        "spec": {
            "template": {
                "metadata": {
                    "annotations": {
                        "stellar.org/vault-rollout-restart": restart_ts
                    }
                }
            }
        }
    });
    sts_api
        .patch(
            &name,
            &PatchParams::apply("stellar-operator").force(),
            &Patch::Merge(&patch),
        )
        .await
        .map_err(Error::KubeError)?;

    let api_sn: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
    let st_patch = json!({
        "status": {
            "vaultObservedSecretVersion": obs_str
        }
    });
    api_sn
        .patch_status(
            &node.name_any(),
            &PatchParams::apply("stellar-operator"),
            &Patch::Merge(&st_patch),
        )
        .await
        .map_err(Error::KubeError)?;

    info!(
        node = %node.name_any(),
        annotation = %ver_key,
        version = %obs_str,
        "Triggered StatefulSet rollout for Vault secret rotation"
    );
    Ok(())
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::seed_secret::{CsiSecretRef, VaultSecretRef};

    #[test]
    fn test_vault_agent_env_and_annotations() {
        let vr = VaultSecretRef {
            role: "r".to_string(),
            secret_path: "secret/data/x".to_string(),
            secret_key: None,
            secret_file_name: Some("myseed".to_string()),
            template: None,
            restart_on_secret_rotation: true,
            extra_pod_annotations: vec![],
        };
        let spec = SeedInjectionSpec::vault_agent(vr);
        let vars = spec.env_vars();
        assert_eq!(vars[0].name, "STELLAR_SEED_FILE");
        assert_eq!(vars[0].value.as_deref(), Some("/vault/secrets/myseed"));
        assert!(spec.pod_annotations().is_some());
        assert!(spec.vault_restart_on_rotation());
    }

    #[test]
    fn test_env_from_secret_vars() {
        let spec = SeedInjectionSpec::env_from_secret(
            "my-secret".to_string(),
            "STELLAR_CORE_SEED".to_string(),
        );
        let vars = spec.env_vars();
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].name, "STELLAR_CORE_SEED");
        assert!(vars[0].value.is_none(), "value must not be set directly");
        assert!(vars[0].value_from.is_some());
        assert!(spec.volume_mounts().is_empty());
        assert!(spec.volumes().is_empty());
    }

    #[test]
    fn test_env_from_secret_legacy_key_uses_standard_env_name() {
        // During migration the Secret may still only contain the legacy SEED
        // key; the env var name must remain STELLAR_CORE_SEED and exactly one
        // env var must be produced.
        let spec = SeedInjectionSpec::env_from_secret(
            "my-secret".to_string(),
            LEGACY_SEED_KEY.to_string(),
        );
        let vars = spec.env_vars();
        assert_eq!(vars.len(), 1, "must not emit duplicate env vars");
        assert_eq!(vars[0].name, DEFAULT_SEED_KEY);
        let key_ref = vars[0]
            .value_from
            .as_ref()
            .and_then(|v| v.secret_key_ref.as_ref())
            .expect("secretKeyRef must be set");
        assert_eq!(key_ref.key, LEGACY_SEED_KEY);
        assert!(vars[0].value.is_none(), "value must not be set directly");
    }

    #[test]
    fn test_env_from_secret_preferred_key() {
        let spec = SeedInjectionSpec::env_from_secret(
            "my-secret".to_string(),
            DEFAULT_SEED_KEY.to_string(),
        );
        let vars = spec.env_vars();
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].name, DEFAULT_SEED_KEY);
        let key_ref = vars[0]
            .value_from
            .as_ref()
            .and_then(|v| v.secret_key_ref.as_ref())
            .expect("secretKeyRef must be set");
        assert_eq!(key_ref.key, DEFAULT_SEED_KEY);
    }

    #[test]
    fn test_csi_mount_vars_and_volumes() {
        let config = CsiSecretRef {
            secret_provider_class_name: "my-spc".to_string(),
            mount_path: None,
            seed_file_name: None,
        };
        let spec = SeedInjectionSpec::csi_mount(config);
        let vars = spec.env_vars();
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].name, "STELLAR_SEED_FILE");
        assert_eq!(
            vars[0].value.as_deref(),
            Some("/mnt/secrets/validator/seed")
        );
        assert_eq!(spec.volume_mounts().len(), 1);
        assert_eq!(spec.volumes().len(), 1);
    }
}
