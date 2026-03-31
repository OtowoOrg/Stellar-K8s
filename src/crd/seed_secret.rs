//! Seed secret source types for KMS / Vault / CSI integration
//!
//! This module defines the typed `SeedSecretSource` enum that replaces the plain
//! `seed_secret_ref: String` field on [`ValidatorConfig`].  It supports three
//! mutually-exclusive backends:
//!
//! | Variant        | Backend                                | Recommended for  |
//! |----------------|----------------------------------------|------------------|
//! | `LocalRef`     | Plain Kubernetes Secret                | Development only |
//! | `ExternalRef`  | External Secrets Operator (ESO)        | Production       |
//! | `CsiRef`       | Secrets Store CSI Driver               | Production       |
//! | `VaultRef`     | HashiCorp Vault Agent Injector (sidecar) | Production       |
//!
//! # Migration from the old string field
//!
//! Old spec (still accepted via `seed_secret_ref` string for backwards compat):
//! ```yaml
//! validatorConfig:
//!   seedSecretRef: "my-validator-seed"   # plain string → treated as LocalRef
//! ```
//!
//! New spec (any of the three variants):
//! ```yaml
//! validatorConfig:
//!   seedSecretSource:
//!     localRef:
//!       name: my-validator-seed
//!       key: STELLAR_CORE_SEED          # optional, defaults to STELLAR_CORE_SEED
//!
//!   # — or —
//!
//!   seedSecretSource:
//!     externalRef:
//!       name: validator-seed-es          # ExternalSecret CR name to create
//!       secretStoreRef:
//!         name: aws-secretsmanager
//!         kind: ClusterSecretStore
//!       remoteKey: "prod/stellar/validator-seed"
//!       remoteProperty: "seed"           # optional
//!
//!   # — or —
//!
//!   seedSecretSource:
//!     csiRef:
//!       secretProviderClassName: stellar-validator-seed-vault
//!       mountPath: /mnt/secrets/validator  # optional, default shown
//! ```

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ============================================================================
// Top-level discriminated union
// ============================================================================

/// Source of the validator seed secret.
///
/// Exactly **one** field must be set.  The controller validates this at
/// reconcile time and will set a `SeedSecretReady=False` condition with a
/// helpful message if the configuration is invalid.
///
/// > ⚠️  `local_ref` is provided for development convenience **only**.
/// > Plain Kubernetes Secrets are base64-encoded, not encrypted, and can be
/// > read by anyone with `get secret` RBAC.  Use `external_ref` or `csi_ref`
/// > for any environment that handles real funds.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SeedSecretSource {
    /// Plain Kubernetes Secret — **development only**.
    ///
    /// Points to an existing `Secret` in the same namespace.  The secret must
    /// contain the key specified in `key` (defaults to `STELLAR_CORE_SEED`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_ref: Option<LocalSecretRef>,

    /// External Secrets Operator — **recommended for production**.
    ///
    /// The operator creates an `ExternalSecret` CR which causes ESO to pull
    /// the seed from AWS Secrets Manager, GCP Secret Manager, HashiCorp Vault,
    /// or any other supported backend and materialise it as a Kubernetes Secret
    /// in the same namespace.  The seed value is never stored in the CRD itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_ref: Option<ExternalSecretRef>,

    /// Secrets Store CSI Driver — **recommended for production**.
    ///
    /// Mounts the seed directly from a KMS/Vault into the pod filesystem via a
    /// CSI volume.  The seed is never written to etcd.  The controller injects
    /// `STELLAR_SEED_FILE` into the container pointing at the mount path;
    /// stellar-core reads the key from that file path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csi_ref: Option<CsiSecretRef>,

    /// HashiCorp Vault via the **Vault Agent Injector** (init + sidecar).
    ///
    /// Requires the Vault Agent Injector mutating webhook in the cluster.
    /// The operator sets standard `vault.hashicorp.com/*` pod annotations;
    /// the injector adds the Vault Agent containers and renders the secret file
    /// under `/vault/secrets/`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vault_ref: Option<VaultSecretRef>,
}

impl SeedSecretSource {
    /// Returns a human-readable description of the active source for logging.
    /// **Never includes secret values.**
    pub fn describe(&self) -> &'static str {
        match (
            self.local_ref.is_some(),
            self.external_ref.is_some(),
            self.csi_ref.is_some(),
            self.vault_ref.is_some(),
        ) {
            (true, false, false, false) => "local Kubernetes Secret (dev only)",
            (false, true, false, false) => "External Secrets Operator",
            (false, false, true, false) => "Secrets Store CSI Driver",
            (false, false, false, true) => "HashiCorp Vault Agent Injector",
            _ => "invalid (multiple sources set)",
        }
    }

    /// Validates that exactly one variant is set.
    pub fn validate(&self) -> Result<(), String> {
        let count = [
            self.local_ref.is_some(),
            self.external_ref.is_some(),
            self.csi_ref.is_some(),
            self.vault_ref.is_some(),
        ]
        .iter()
        .filter(|&&b| b)
        .count();

        match count {
            1 => Ok(()),
            0 => Err(
                "seedSecretSource: at least one of localRef, externalRef, csiRef, or vaultRef must be set"
                    .to_string(),
            ),
            _ => Err(
                "seedSecretSource: exactly one of localRef, externalRef, csiRef, or vaultRef must be set; multiple fields are set"
                    .to_string(),
            ),
        }
    }

    /// Returns `true` when the source is a plain local Secret.
    /// Used to emit a warning in non-development environments.
    pub fn is_local(&self) -> bool {
        self.local_ref.is_some()
    }
}

// ============================================================================
// Variant 1 — Local Kubernetes Secret
// ============================================================================

/// Reference to a plain Kubernetes `Secret`.
///
/// ```yaml
/// localRef:
///   name: my-dev-validator-seed
///   key: STELLAR_CORE_SEED   # optional
/// ```
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalSecretRef {
    /// Name of the `Secret` in the same namespace.
    pub name: String,

    /// Key within the secret that holds the seed value.
    /// Defaults to `STELLAR_CORE_SEED` if not specified.
    #[serde(default = "default_seed_key", skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

impl LocalSecretRef {
    /// Returns the effective key name, defaulting to `STELLAR_CORE_SEED`.
    pub fn effective_key(&self) -> &str {
        self.key.as_deref().unwrap_or(DEFAULT_SEED_KEY)
    }
}

// ============================================================================
// Variant 2 — External Secrets Operator
// ============================================================================

/// Instructs the operator to create an `ExternalSecret` CR so that ESO pulls
/// the seed from a remote KMS/Vault and materialises it as a local Secret.
///
/// The produced Secret is named `<node-name>-seed` and is owned by the
/// `ExternalSecret` CR (which in turn is owned by the `StellarNode`).
///
/// ```yaml
/// externalRef:
///   name: validator-prod-seed-es
///   secretStoreRef:
///     name: aws-secretsmanager
///     kind: ClusterSecretStore
///   remoteKey: "prod/stellar/validator-seed"
///   remoteProperty: "seed"
///   refreshInterval: "1h"
/// ```
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalSecretRef {
    /// Name of the `ExternalSecret` CR the operator will create/manage.
    /// Must be unique within the namespace.
    pub name: String,

    /// Reference to the `SecretStore` or `ClusterSecretStore` that connects
    /// ESO to the remote backend (AWS SM, GCP SM, Vault, etc.).
    pub secret_store_ref: SecretStoreRef,

    /// Path / identifier of the secret in the remote backend.
    ///
    /// Examples:
    /// - AWS Secrets Manager: `"prod/stellar/validator-seed"`
    /// - GCP Secret Manager: `"projects/MY_PROJECT/secrets/stellar-validator-seed"`
    /// - HashiCorp Vault: `"secret/data/stellar/validator"`
    pub remote_key: String,

    /// Property (field) inside the remote secret to extract.
    ///
    /// Required for secrets that store a JSON object (e.g., `{"seed": "S..."}`)
    /// and you only want the `seed` value.  Leave empty to use the whole secret
    /// value as the seed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_property: Option<String>,

    /// How often ESO should re-sync the secret from the remote backend.
    /// Kubernetes duration string, e.g. `"1h"`, `"30m"`.
    /// Defaults to `"1h"` if not specified.
    #[serde(
        default = "default_refresh_interval",
        skip_serializing_if = "Option::is_none"
    )]
    pub refresh_interval: Option<String>,
}

/// Reference to an ESO `SecretStore` or `ClusterSecretStore`.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecretStoreRef {
    /// Name of the `SecretStore` / `ClusterSecretStore` resource.
    pub name: String,

    /// Kind of the store resource.
    ///
    /// - `"SecretStore"` — namespaced store (only works within the same namespace)
    /// - `"ClusterSecretStore"` — cluster-wide store (recommended for production)
    #[serde(default = "default_secret_store_kind")]
    pub kind: String,
}

// ============================================================================
// Variant 3 — Secrets Store CSI Driver
// ============================================================================

/// Mounts secrets directly from a KMS/Vault into the pod via a CSI volume,
/// bypassing etcd entirely.
///
/// The operator does **not** create the `SecretProviderClass`; you must apply
/// that separately (see `config/samples/secretproviderclass-*.yaml`).
///
/// ```yaml
/// csiRef:
///   secretProviderClassName: stellar-validator-seed-vault
///   mountPath: /mnt/secrets/validator
///   seedFileName: seed
/// ```
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CsiSecretRef {
    /// Name of the `SecretProviderClass` CR (from secrets-store.csi.x-k8s.io)
    /// that defines which secrets to mount and from which provider.
    pub secret_provider_class_name: String,

    /// Directory inside the container where the CSI driver mounts secrets.
    /// Defaults to `/mnt/secrets/validator`.
    #[serde(
        default = "default_mount_path",
        skip_serializing_if = "Option::is_none"
    )]
    pub mount_path: Option<String>,

    /// File name within `mount_path` that contains the seed value.
    /// Defaults to `seed`.
    #[serde(
        default = "default_seed_file_name",
        skip_serializing_if = "Option::is_none"
    )]
    pub seed_file_name: Option<String>,
}

impl CsiSecretRef {
    /// Returns the effective mount path.
    pub fn effective_mount_path(&self) -> &str {
        self.mount_path.as_deref().unwrap_or(DEFAULT_MOUNT_PATH)
    }

    /// Returns the full path to the seed file inside the container.
    pub fn seed_file_path(&self) -> String {
        format!(
            "{}/{}",
            self.effective_mount_path(),
            self.seed_file_name
                .as_deref()
                .unwrap_or(DEFAULT_SEED_FILE_NAME)
        )
    }
}

// ============================================================================
// Variant 4 — HashiCorp Vault Agent Injector
// ============================================================================

/// Native Vault integration: pod annotations consumed by the Vault Agent Injector.
///
/// Install the [Vault Helm chart](https://github.com/hashicorp/vault-helm) with
/// `injector.enabled=true`. The operator never reads secret material; it only
/// sets annotations so the injector adds the Agent init container and sidecar.
///
/// ```yaml
/// seedSecretSource:
///   vaultRef:
///     role: stellar-validator
///     secretPath: secret/data/stellar/validator
///     secretKey: seed
///     secretFileName: stellar-seed
///     restartOnSecretRotation: true
/// ```
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VaultSecretRef {
    /// Vault Kubernetes auth role bound to this pod's ServiceAccount.
    pub role: String,

    /// Path passed to `vault.hashicorp.com/agent-inject-secret-<file>` (KV v1/v2 path as in Vault).
    pub secret_path: String,

    /// JSON field under `.Data.data` for KV v2 (default `seed`). Ignored if `template` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_key: Option<String>,

    /// Base file name rendered under `/vault/secrets/` (default `stellar-seed`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_file_name: Option<String>,

    /// Custom Agent template; when set, overrides the default KV v2 template.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,

    /// When true, the operator compares Vault secret-version annotations on pods
    /// and rolls the StatefulSet when the version changes after sync.
    #[serde(default)]
    pub restart_on_secret_rotation: bool,

    /// Additional `vault.hashicorp.com/*` or other pod annotations to merge.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_pod_annotations: Vec<VaultPodAnnotation>,
}

/// Key/value pair for extra Vault Agent pod annotations (CRD-friendly vs raw maps).
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VaultPodAnnotation {
    pub name: String,
    pub value: String,
}

impl VaultSecretRef {
    /// Effective secret file name for injector annotations and `STELLAR_SEED_FILE`.
    pub fn effective_secret_file_name(&self) -> &str {
        self.secret_file_name
            .as_deref()
            .unwrap_or(DEFAULT_VAULT_SECRET_FILE)
    }
}

const DEFAULT_VAULT_SECRET_FILE: &str = "stellar-seed";

// ============================================================================
// Defaults
// ============================================================================

pub const DEFAULT_SEED_KEY: &str = "STELLAR_CORE_SEED";
pub const DEFAULT_MOUNT_PATH: &str = "/mnt/secrets/validator";
pub const DEFAULT_SEED_FILE_NAME: &str = "seed";

fn default_seed_key() -> Option<String> {
    Some(DEFAULT_SEED_KEY.to_string())
}

fn default_refresh_interval() -> Option<String> {
    Some("1h".to_string())
}

fn default_secret_store_kind() -> String {
    "ClusterSecretStore".to_string()
}

fn default_mount_path() -> Option<String> {
    Some(DEFAULT_MOUNT_PATH.to_string())
}

fn default_seed_file_name() -> Option<String> {
    Some(DEFAULT_SEED_FILE_NAME.to_string())
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn local_source() -> SeedSecretSource {
        SeedSecretSource {
            local_ref: Some(LocalSecretRef {
                name: "dev-secret".to_string(),
                key: None,
            }),
            external_ref: None,
            csi_ref: None,
            vault_ref: None,
        }
    }

    fn external_source() -> SeedSecretSource {
        SeedSecretSource {
            local_ref: None,
            external_ref: Some(ExternalSecretRef {
                name: "validator-es".to_string(),
                secret_store_ref: SecretStoreRef {
                    name: "aws-sm".to_string(),
                    kind: "ClusterSecretStore".to_string(),
                },
                remote_key: "prod/stellar/seed".to_string(),
                remote_property: Some("seed".to_string()),
                refresh_interval: Some("1h".to_string()),
            }),
            csi_ref: None,
            vault_ref: None,
        }
    }

    fn csi_source() -> SeedSecretSource {
        SeedSecretSource {
            local_ref: None,
            external_ref: None,
            csi_ref: Some(CsiSecretRef {
                secret_provider_class_name: "stellar-vault-spc".to_string(),
                mount_path: None,
                seed_file_name: None,
            }),
            vault_ref: None,
        }
    }

    fn vault_source() -> SeedSecretSource {
        SeedSecretSource {
            local_ref: None,
            external_ref: None,
            csi_ref: None,
            vault_ref: Some(VaultSecretRef {
                role: "validator".to_string(),
                secret_path: "secret/data/stellar/validator".to_string(),
                secret_key: Some("seed".to_string()),
                secret_file_name: None,
                template: None,
                restart_on_secret_rotation: false,
                extra_pod_annotations: vec![],
            }),
        }
    }

    #[test]
    fn test_validate_single_source_ok() {
        assert!(local_source().validate().is_ok());
        assert!(external_source().validate().is_ok());
        assert!(csi_source().validate().is_ok());
        assert!(vault_source().validate().is_ok());
    }

    #[test]
    fn test_validate_empty_source_err() {
        let s = SeedSecretSource {
            local_ref: None,
            external_ref: None,
            csi_ref: None,
            vault_ref: None,
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn test_validate_multiple_sources_err() {
        let mut s = local_source();
        s.external_ref = external_source().external_ref;
        assert!(s.validate().is_err());
    }

    #[test]
    fn test_is_local() {
        assert!(local_source().is_local());
        assert!(!external_source().is_local());
        assert!(!csi_source().is_local());
        assert!(!vault_source().is_local());
    }

    #[test]
    fn test_csi_defaults() {
        let csi = CsiSecretRef {
            secret_provider_class_name: "my-spc".to_string(),
            mount_path: None,
            seed_file_name: None,
        };
        assert_eq!(csi.effective_mount_path(), "/mnt/secrets/validator");
        assert_eq!(csi.seed_file_path(), "/mnt/secrets/validator/seed");
    }

    #[test]
    fn test_local_ref_default_key() {
        let r = LocalSecretRef {
            name: "my-secret".to_string(),
            key: None,
        };
        assert_eq!(r.effective_key(), "STELLAR_CORE_SEED");
    }

    #[test]
    fn test_describe() {
        assert!(local_source().describe().contains("dev only"));
        assert!(external_source().describe().contains("External Secrets"));
        assert!(csi_source().describe().contains("CSI"));
        assert!(vault_source().describe().contains("Vault"));
    }
}
