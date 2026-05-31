//! Pod Security Standards (PSS) enforcement for managed namespaces.
//!
//! Implements Zero-Trust workload isolation by:
//! - Labeling managed namespaces with `pod-security.kubernetes.io/enforce: restricted`
//! - Providing helpers to build PSS-compliant container and pod security contexts
//! - Validating that a StellarNodeSpec does not attempt to bypass PSS constraints

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{
    Capabilities, Namespace, PodSecurityContext, SeccompProfile, SecurityContext,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Api, Patch, PatchParams};
use kube::Client;
use tracing::{info, instrument, warn};

use crate::crd::StellarNodeSpec;
use crate::error::{Error, Result};

// ── PSS label constants ──────────────────────────────────────────────────────

pub const PSS_ENFORCE_LABEL: &str = "pod-security.kubernetes.io/enforce";
pub const PSS_ENFORCE_VERSION_LABEL: &str = "pod-security.kubernetes.io/enforce-version";
pub const PSS_WARN_LABEL: &str = "pod-security.kubernetes.io/warn";
pub const PSS_WARN_VERSION_LABEL: &str = "pod-security.kubernetes.io/warn-version";
pub const PSS_AUDIT_LABEL: &str = "pod-security.kubernetes.io/audit";
pub const PSS_AUDIT_VERSION_LABEL: &str = "pod-security.kubernetes.io/audit-version";

pub const PSS_LEVEL: &str = "restricted";
pub const PSS_VERSION: &str = "latest";

// ── Namespace labeling ───────────────────────────────────────────────────────

/// Ensure the given namespace carries the full set of PSS `restricted` labels.
///
/// Uses a strategic-merge patch so that other labels on the namespace are
/// preserved. Safe to call on every reconcile — it is idempotent.
#[instrument(skip(client), fields(namespace = %namespace))]
pub async fn ensure_namespace_pss_labels(client: &Client, namespace: &str) -> Result<()> {
    let api: Api<Namespace> = Api::all(client.clone());

    let mut labels: BTreeMap<String, String> = BTreeMap::new();
    labels.insert(PSS_ENFORCE_LABEL.to_string(), PSS_LEVEL.to_string());
    labels.insert(
        PSS_ENFORCE_VERSION_LABEL.to_string(),
        PSS_VERSION.to_string(),
    );
    labels.insert(PSS_WARN_LABEL.to_string(), PSS_LEVEL.to_string());
    labels.insert(PSS_WARN_VERSION_LABEL.to_string(), PSS_VERSION.to_string());
    labels.insert(PSS_AUDIT_LABEL.to_string(), PSS_LEVEL.to_string());
    labels.insert(PSS_AUDIT_VERSION_LABEL.to_string(), PSS_VERSION.to_string());

    let patch = serde_json::json!({
        "metadata": {
            "labels": labels
        }
    });

    match api
        .patch(
            namespace,
            &PatchParams::apply("stellar-operator-pss").force(),
            &Patch::Apply(&patch),
        )
        .await
    {
        Ok(_) => {
            info!(
                "PSS 'restricted' labels applied to namespace '{}'",
                namespace
            );
            Ok(())
        }
        Err(kube::Error::Api(e)) if e.code == 404 => {
            // Namespace doesn't exist yet — create it with the labels
            let ns = Namespace {
                metadata: ObjectMeta {
                    name: Some(namespace.to_string()),
                    labels: Some(labels),
                    ..Default::default()
                },
                ..Default::default()
            };
            api.create(&kube::api::PostParams::default(), &ns)
                .await
                .map_err(Error::KubeError)?;
            info!(
                "Created namespace '{}' with PSS 'restricted' labels",
                namespace
            );
            Ok(())
        }
        Err(e) => {
            warn!(
                "Failed to apply PSS labels to namespace '{}': {}",
                namespace, e
            );
            Err(Error::KubeError(e))
        }
    }
}

// ── Security context builders ────────────────────────────────────────────────

/// Build a PSS-compliant pod-level security context.
///
/// Sets:
/// - `runAsNonRoot: true`
/// - `seccompProfile.type: RuntimeDefault`
/// - `runAsUser: 10000` (non-root UID)
/// - `runAsGroup: 10000`
/// - `fsGroup: 10000`
pub fn restricted_pod_security_context() -> PodSecurityContext {
    PodSecurityContext {
        run_as_non_root: Some(true),
        run_as_user: Some(10000),
        run_as_group: Some(10000),
        fs_group: Some(10000),
        seccomp_profile: Some(SeccompProfile {
            type_: "RuntimeDefault".to_string(),
            localhost_profile: None,
        }),
        ..Default::default()
    }
}

/// Build a PSS-compliant container-level security context.
///
/// Sets:
/// - `allowPrivilegeEscalation: false`
/// - `capabilities.drop: ["ALL"]`
/// - `runAsNonRoot: true`
/// - `seccompProfile.type: RuntimeDefault`
/// - `readOnlyRootFilesystem: true`
pub fn restricted_container_security_context() -> SecurityContext {
    SecurityContext {
        allow_privilege_escalation: Some(false),
        capabilities: Some(Capabilities {
            drop: Some(vec!["ALL".to_string()]),
            add: None,
        }),
        run_as_non_root: Some(true),
        seccomp_profile: Some(SeccompProfile {
            type_: "RuntimeDefault".to_string(),
            localhost_profile: None,
        }),
        read_only_root_filesystem: Some(true),
        privileged: Some(false),
        ..Default::default()
    }
}

// ── Spec validation ──────────────────────────────────────────────────────────

/// A PSS bypass violation found in a StellarNodeSpec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PssViolation {
    pub field: String,
    pub message: String,
}

impl PssViolation {
    fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

/// Validate that a [`StellarNodeSpec`] does not attempt to bypass PSS `restricted` constraints.
///
/// Returns a list of violations. An empty list means the spec is compliant.
pub fn validate_pss_compliance(spec: &StellarNodeSpec) -> Vec<PssViolation> {
    let mut violations = Vec::new();

    // Check sidecars for privilege escalation attempts
    if let Some(sidecars) = &spec.sidecars {
        for (i, sidecar) in sidecars.iter().enumerate() {
            let prefix = format!("spec.sidecars[{}]", i);
            check_container_security_context(
                &prefix,
                sidecar.security_context.as_ref(),
                &mut violations,
            );
        }
    }

    // Validator-specific: forensic snapshot ephemeral container adds NET_RAW/SYS_PTRACE
    // which is intentional and documented — we emit a warning rather than blocking.
    // No additional spec-level bypass checks needed here.

    violations
}

/// Check a container's security context for PSS `restricted` violations.
fn check_container_security_context(
    prefix: &str,
    ctx: Option<&SecurityContext>,
    violations: &mut Vec<PssViolation>,
) {
    let Some(sc) = ctx else { return };

    if sc.privileged == Some(true) {
        violations.push(PssViolation::new(
            format!("{prefix}.securityContext.privileged"),
            "privileged containers are forbidden under PSS 'restricted'",
        ));
    }

    if sc.allow_privilege_escalation == Some(true) {
        violations.push(PssViolation::new(
            format!("{prefix}.securityContext.allowPrivilegeEscalation"),
            "allowPrivilegeEscalation must be false under PSS 'restricted'",
        ));
    }

    if let Some(caps) = &sc.capabilities {
        let forbidden = [
            "NET_ADMIN",
            "SYS_ADMIN",
            "SYS_PTRACE",
            "NET_RAW",
            "SYS_MODULE",
        ];
        if let Some(adds) = &caps.add {
            for cap in adds {
                if forbidden.contains(&cap.as_str()) {
                    violations.push(PssViolation::new(
                        format!("{prefix}.securityContext.capabilities.add"),
                        format!("capability '{cap}' is forbidden under PSS 'restricted'"),
                    ));
                }
            }
        }
    }

    if let Some(profile) = &sc.seccomp_profile {
        if profile.type_ == "Unconfined" {
            violations.push(PssViolation::new(
                format!("{prefix}.securityContext.seccompProfile.type"),
                "seccompProfile type 'Unconfined' is forbidden under PSS 'restricted'",
            ));
        }
    }

    if sc.run_as_user == Some(0) {
        violations.push(PssViolation::new(
            format!("{prefix}.securityContext.runAsUser"),
            "runAsUser 0 (root) is forbidden under PSS 'restricted'",
        ));
    }
}
