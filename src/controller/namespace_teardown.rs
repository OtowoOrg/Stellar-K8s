// Copyright 2026 Stellar-K8s Contributors
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
//! Namespace teardown state machine driver (#1978).
//!
//! Drives a [`NamespaceTeardown`] CR through
//! `Pending -> Archiving -> RevokingCredentials -> CleaningDns -> Attesting ->
//! Deleting -> Complete`, one step per [`advance`] call. Every step records a
//! [`TeardownCondition`], so a stuck step shows up in `.status.conditions`
//! with an actionable message instead of failing silently in a shell pipeline.
//!
//! Guarantees:
//! - the archive checksum is verified against the expected SHA-256 *before*
//!   the namespace is deleted;
//! - credential revocation is *confirmed* (the backend reports no live
//!   credential) within [`REVOCATION_CONFIRM_DEADLINE_SECS`] or the teardown
//!   fails with `RevocationNotConfirmed`;
//! - DNS records are removed and verified gone before deletion;
//! - the namespace is only reported `Complete` once it no longer exists.
//!
//! External systems (archive store, credential store, DNS controller) sit
//! behind [`TeardownBackend`] so the state machine is testable without a
//! cluster; [`KubeTeardownBackend`] is the in-cluster implementation.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use k8s_openapi::api::core::v1::{ConfigMap, Namespace, Secret};
use kube::api::{Api, ApiResource, DeleteParams, DynamicObject, Patch, PatchParams, PostParams};
use kube::core::GroupVersionKind;
use kube::Client;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::crd::namespace_teardown::{
    ArchiveSpec, NamespaceTeardown, NamespaceTeardownSpec, NamespaceTeardownStatus,
    TeardownCondition, TeardownPhase, TeardownStepStatus,
};

/// Credential revocation must be confirmed within this many seconds.
pub const REVOCATION_CONFIRM_DEADLINE_SECS: i64 = 60;
/// Per-step timeout used when `spec.stepTimeoutSeconds` is 0.
pub const DEFAULT_STEP_TIMEOUT_SECS: i64 = 300;

/// Final compliance attestation written to `spec.attestationSink`.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TeardownAttestation {
    pub target_namespace: String,
    pub archive_checksum: Option<String>,
    pub credentials_revoked: Vec<String>,
    pub dns_records_removed: Vec<String>,
    pub attested_at: String,
}

/// Side effects the teardown needs. Errors are plain strings: they are
/// surfaced verbatim in the step condition message.
#[async_trait]
pub trait TeardownBackend: Send + Sync {
    /// Archive the namespace and return the SHA-256 (hex) of the archive.
    async fn archive(&self, namespace: &str, archive: &ArchiveSpec) -> Result<String, String>;
    async fn revoke_credential(&self, namespace: &str, credential: &str) -> Result<(), String>;
    /// True while the credential is still usable.
    async fn credential_live(&self, namespace: &str, credential: &str) -> Result<bool, String>;
    async fn delete_dns_record(&self, namespace: &str, record: &str) -> Result<(), String>;
    async fn dns_record_exists(&self, namespace: &str, record: &str) -> Result<bool, String>;
    async fn write_attestation(
        &self,
        sink: &str,
        attestation: &TeardownAttestation,
    ) -> Result<(), String>;
    async fn delete_namespace(&self, namespace: &str) -> Result<(), String>;
    async fn namespace_exists(&self, namespace: &str) -> Result<bool, String>;
}

fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339()
}

fn phase_name(p: &TeardownPhase) -> String {
    format!("{p:?}")
}

fn step_timeout(spec: &NamespaceTeardownSpec) -> i64 {
    if spec.step_timeout_seconds == 0 {
        DEFAULT_STEP_TIMEOUT_SECS
    } else {
        i64::from(spec.step_timeout_seconds)
    }
}

/// Phase that follows `current` in the spec's pipeline.
pub fn next_phase(spec: &NamespaceTeardownSpec, current: &TeardownPhase) -> TeardownPhase {
    let order = spec.step_order();
    order
        .iter()
        .position(|p| p == current)
        .and_then(|i| order.get(i + 1))
        .cloned()
        .unwrap_or(TeardownPhase::Complete)
}

fn set_condition(
    status: &mut NamespaceTeardownStatus,
    step: &str,
    state: TeardownStepStatus,
    reason: Option<&str>,
    message: Option<String>,
    now: DateTime<Utc>,
) {
    let previous = status.conditions.iter().position(|c| c.step == step);
    // Keep the original transition time while a step keeps reporting the same
    // status, so "how long has this been stuck" stays answerable.
    let last_transition = match previous.map(|i| &status.conditions[i]) {
        Some(c) if c.status == state => c.last_transition.clone(),
        _ => Some(rfc3339(now)),
    };
    let cond = TeardownCondition {
        step: step.to_string(),
        status: state,
        reason: reason.map(str::to_string),
        message,
        last_transition,
    };
    match previous {
        Some(i) => status.conditions[i] = cond,
        None => status.conditions.push(cond),
    }
}

fn step_running_since(status: &NamespaceTeardownStatus, step: &str) -> Option<DateTime<Utc>> {
    status
        .conditions
        .iter()
        .find(|c| c.step == step && c.status == TeardownStepStatus::Running)
        .and_then(|c| c.last_transition.as_deref())
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc))
}

fn fail(
    status: &mut NamespaceTeardownStatus,
    step: &str,
    reason: &str,
    message: String,
    now: DateTime<Utc>,
) {
    set_condition(
        status,
        step,
        TeardownStepStatus::Failed,
        Some(reason),
        Some(message),
        now,
    );
    status.phase = TeardownPhase::Failed;
}

fn succeed(
    spec: &NamespaceTeardownSpec,
    status: &mut NamespaceTeardownStatus,
    step: &str,
    now: DateTime<Utc>,
) {
    set_condition(status, step, TeardownStepStatus::Succeeded, None, None, now);
    status.phase = next_phase(spec, &status.phase);
}

/// Advance the teardown by at most one step and return the new status.
/// Pure with respect to `status` (the input is not mutated) so callers can
/// diff and patch.
pub async fn advance(
    spec: &NamespaceTeardownSpec,
    current: &NamespaceTeardownStatus,
    backend: &dyn TeardownBackend,
    now: DateTime<Utc>,
) -> NamespaceTeardownStatus {
    let mut status = current.clone();
    if NamespaceTeardownSpec::is_terminal(&status.phase) {
        return status;
    }

    if let Err(msg) = spec.validate() {
        fail(
            &mut status,
            "Validation",
            "InvalidSpec",
            format!("{msg}. Fix the NamespaceTeardown spec and re-create it."),
            now,
        );
        return status;
    }

    let ns = spec.target_namespace.as_str();

    if status.phase == TeardownPhase::Pending {
        status.started_at.get_or_insert_with(|| rfc3339(now));
        status.phase = next_phase(spec, &TeardownPhase::Pending);
        return status;
    }

    let step = phase_name(&status.phase);

    // Stuck-step detection: a step that keeps retrying past its timeout
    // becomes a Failed condition with a pointer to what to check.
    if let Some(since) = step_running_since(&status, &step) {
        let age = (now - since).num_seconds();
        let limit = step_timeout(spec);
        if age > limit {
            let last = status
                .conditions
                .iter()
                .find(|c| c.step == step)
                .and_then(|c| c.message.clone())
                .unwrap_or_default();
            fail(
                &mut status,
                &step,
                "StepTimedOut",
                format!(
                    "step {step} made no progress for {age}s (limit {limit}s). Last error: {last}. \
                     Check the backing system for this step, then re-create the NamespaceTeardown to retry."
                ),
                now,
            );
            return status;
        }
    }

    match status.phase.clone() {
        TeardownPhase::Archiving => {
            let Some(archive) = spec.archive.as_ref() else {
                succeed(spec, &mut status, &step, now);
                return status;
            };
            match backend.archive(ns, archive).await {
                Ok(checksum) => {
                    if let Some(expected) = archive.expected_checksum.as_ref() {
                        if !checksum.eq_ignore_ascii_case(expected) {
                            fail(
                                &mut status,
                                &step,
                                "ChecksumMismatch",
                                format!(
                                    "archive checksum {checksum} does not match expected {expected}; \
                                     namespace {ns} was NOT deleted. Re-run the archive or correct archive.expectedChecksum."
                                ),
                                now,
                            );
                            return status;
                        }
                    }
                    status.archive_checksum = Some(checksum);
                    succeed(spec, &mut status, &step, now);
                }
                Err(e) => set_condition(
                    &mut status,
                    &step,
                    TeardownStepStatus::Running,
                    Some("ArchiveRetrying"),
                    Some(e),
                    now,
                ),
            }
        }
        TeardownPhase::RevokingCredentials => {
            let mut errors = Vec::new();
            for cred in &spec.credential_refs {
                if let Err(e) = backend.revoke_credential(ns, cred).await {
                    errors.push(format!("{cred}: {e}"));
                }
            }
            let mut live = Vec::new();
            for cred in &spec.credential_refs {
                match backend.credential_live(ns, cred).await {
                    Ok(false) => {}
                    Ok(true) => live.push(cred.clone()),
                    Err(e) => errors.push(format!("{cred}: {e}")),
                }
            }
            if live.is_empty() && errors.is_empty() {
                succeed(spec, &mut status, &step, now);
                return status;
            }
            let started = step_running_since(&status, &step).unwrap_or(now);
            let waited = (now - started).num_seconds();
            let detail = format!(
                "live: [{}]; errors: [{}]",
                live.join(", "),
                errors.join("; ")
            );
            if waited > REVOCATION_CONFIRM_DEADLINE_SECS {
                fail(
                    &mut status,
                    &step,
                    "RevocationNotConfirmed",
                    format!(
                        "credentials not confirmed revoked within {REVOCATION_CONFIRM_DEADLINE_SECS}s ({detail}). \
                         Revoke them manually in the credential store, then re-create the NamespaceTeardown."
                    ),
                    now,
                );
            } else {
                set_condition(
                    &mut status,
                    &step,
                    TeardownStepStatus::Running,
                    Some("AwaitingRevocationConfirmation"),
                    Some(detail),
                    now,
                );
            }
        }
        TeardownPhase::CleaningDns => {
            let mut pending = Vec::new();
            for rec in &spec.dns_records {
                if let Err(e) = backend.delete_dns_record(ns, rec).await {
                    pending.push(format!("{rec}: {e}"));
                    continue;
                }
                match backend.dns_record_exists(ns, rec).await {
                    Ok(false) => {}
                    Ok(true) => pending.push(format!("{rec}: still present")),
                    Err(e) => pending.push(format!("{rec}: {e}")),
                }
            }
            if pending.is_empty() {
                succeed(spec, &mut status, &step, now);
            } else {
                set_condition(
                    &mut status,
                    &step,
                    TeardownStepStatus::Running,
                    Some("DnsCleanupPending"),
                    Some(pending.join("; ")),
                    now,
                );
            }
        }
        TeardownPhase::Attesting => {
            let Some(sink) = spec.attestation_sink.as_deref() else {
                succeed(spec, &mut status, &step, now);
                return status;
            };
            let attestation = TeardownAttestation {
                target_namespace: ns.to_string(),
                archive_checksum: status.archive_checksum.clone(),
                credentials_revoked: spec.credential_refs.clone(),
                dns_records_removed: spec.dns_records.clone(),
                attested_at: rfc3339(now),
            };
            match backend.write_attestation(sink, &attestation).await {
                Ok(()) => succeed(spec, &mut status, &step, now),
                Err(e) => set_condition(
                    &mut status,
                    &step,
                    TeardownStepStatus::Running,
                    Some("AttestationRetrying"),
                    Some(e),
                    now,
                ),
            }
        }
        TeardownPhase::Deleting => {
            // Safety gate: never delete while a verified archive is required
            // but missing.
            if spec.archive.as_ref().is_some_and(|a| a.verify) && status.archive_checksum.is_none()
            {
                fail(
                    &mut status,
                    &step,
                    "ArchiveNotVerified",
                    format!(
                        "refusing to delete namespace {ns}: no verified archive checksum recorded"
                    ),
                    now,
                );
                return status;
            }
            if let Err(e) = backend.delete_namespace(ns).await {
                set_condition(
                    &mut status,
                    &step,
                    TeardownStepStatus::Running,
                    Some("DeleteRetrying"),
                    Some(e),
                    now,
                );
                return status;
            }
            match backend.namespace_exists(ns).await {
                Ok(false) => {
                    succeed(spec, &mut status, &step, now);
                    status.completed_at = Some(rfc3339(now));
                }
                Ok(true) => set_condition(
                    &mut status,
                    &step,
                    TeardownStepStatus::Running,
                    Some("NamespaceTerminating"),
                    Some(format!(
                        "namespace {ns} still exists; a finalizer may be blocking deletion"
                    )),
                    now,
                ),
                Err(e) => set_condition(
                    &mut status,
                    &step,
                    TeardownStepStatus::Running,
                    Some("DeleteRetrying"),
                    Some(e),
                    now,
                ),
            }
        }
        TeardownPhase::Pending | TeardownPhase::Complete | TeardownPhase::Failed => {}
    }
    status
}

/// Anything left behind after a teardown, for orphan audits.
pub async fn audit_orphans(
    spec: &NamespaceTeardownSpec,
    backend: &dyn TeardownBackend,
) -> Vec<String> {
    let ns = spec.target_namespace.as_str();
    let mut orphans = Vec::new();
    if backend.namespace_exists(ns).await.unwrap_or(true) {
        orphans.push(format!("namespace/{ns}"));
    }
    for rec in &spec.dns_records {
        if backend.dns_record_exists(ns, rec).await.unwrap_or(true) {
            orphans.push(format!("dns/{rec}"));
        }
    }
    for cred in &spec.credential_refs {
        if backend.credential_live(ns, cred).await.unwrap_or(true) {
            orphans.push(format!("credential/{cred}"));
        }
    }
    orphans
}

// ---------------------------------------------------------------------------
// In-cluster backend
// ---------------------------------------------------------------------------

/// Produces the archive for a namespace. Implementations own the transport
/// (S3, GCS, ...); the teardown only needs the resulting checksum.
#[async_trait]
pub trait ArchiveStore: Send + Sync {
    async fn archive(&self, namespace: &str, archive: &ArchiveSpec) -> Result<String, String>;
}

/// Kubernetes-backed teardown:
/// - credentials are `Secret`s named by `credentialRefs` in the target namespace;
/// - DNS records are external-dns `DNSEndpoint` objects named by `dnsRecords`;
/// - the attestation is a `ConfigMap` named by `attestationSink` in `attestation_namespace`.
pub struct KubeTeardownBackend {
    client: Client,
    archive_store: Arc<dyn ArchiveStore>,
    attestation_namespace: String,
}

impl KubeTeardownBackend {
    pub fn new(
        client: Client,
        archive_store: Arc<dyn ArchiveStore>,
        attestation_namespace: impl Into<String>,
    ) -> Self {
        Self {
            client,
            archive_store,
            attestation_namespace: attestation_namespace.into(),
        }
    }

    fn dns_api(&self, namespace: &str) -> Api<DynamicObject> {
        let gvk = GroupVersionKind::gvk("externaldns.k8s.io", "v1alpha1", "DNSEndpoint");
        Api::namespaced_with(self.client.clone(), namespace, &ApiResource::from_gvk(&gvk))
    }
}

fn is_not_found(e: &kube::Error) -> bool {
    matches!(e, kube::Error::Api(r) if r.code == 404)
}

#[async_trait]
impl TeardownBackend for KubeTeardownBackend {
    async fn archive(&self, namespace: &str, archive: &ArchiveSpec) -> Result<String, String> {
        self.archive_store.archive(namespace, archive).await
    }

    async fn revoke_credential(&self, namespace: &str, credential: &str) -> Result<(), String> {
        let api: Api<Secret> = Api::namespaced(self.client.clone(), namespace);
        match api.delete(credential, &DeleteParams::default()).await {
            Ok(_) => Ok(()),
            Err(e) if is_not_found(&e) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }

    async fn credential_live(&self, namespace: &str, credential: &str) -> Result<bool, String> {
        let api: Api<Secret> = Api::namespaced(self.client.clone(), namespace);
        api.get_opt(credential)
            .await
            .map(|s| s.is_some())
            .map_err(|e| e.to_string())
    }

    async fn delete_dns_record(&self, namespace: &str, record: &str) -> Result<(), String> {
        match self
            .dns_api(namespace)
            .delete(record, &DeleteParams::default())
            .await
        {
            Ok(_) => Ok(()),
            Err(e) if is_not_found(&e) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }

    async fn dns_record_exists(&self, namespace: &str, record: &str) -> Result<bool, String> {
        self.dns_api(namespace)
            .get_opt(record)
            .await
            .map(|o| o.is_some())
            .map_err(|e| e.to_string())
    }

    async fn write_attestation(
        &self,
        sink: &str,
        attestation: &TeardownAttestation,
    ) -> Result<(), String> {
        let body = serde_json::to_string_pretty(attestation).map_err(|e| e.to_string())?;
        let digest = hex::encode(Sha256::digest(body.as_bytes()));
        let cm = ConfigMap {
            metadata: kube::core::ObjectMeta {
                name: Some(sink.to_string()),
                namespace: Some(self.attestation_namespace.clone()),
                ..Default::default()
            },
            data: Some(
                [
                    ("attestation.json".to_string(), body),
                    ("sha256".to_string(), digest),
                ]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        };
        let api: Api<ConfigMap> = Api::namespaced(self.client.clone(), &self.attestation_namespace);
        match api.create(&PostParams::default(), &cm).await {
            Ok(_) => Ok(()),
            Err(kube::Error::Api(r)) if r.code == 409 => api
                .patch(sink, &PatchParams::default(), &Patch::Merge(&cm))
                .await
                .map(|_| ())
                .map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        }
    }

    async fn delete_namespace(&self, namespace: &str) -> Result<(), String> {
        let api: Api<Namespace> = Api::all(self.client.clone());
        match api.delete(namespace, &DeleteParams::default()).await {
            Ok(_) => Ok(()),
            Err(e) if is_not_found(&e) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }

    async fn namespace_exists(&self, namespace: &str) -> Result<bool, String> {
        let api: Api<Namespace> = Api::all(self.client.clone());
        api.get_opt(namespace)
            .await
            .map(|n| n.is_some())
            .map_err(|e| e.to_string())
    }
}

/// Reconcile one `NamespaceTeardown`: advance a step and persist the status.
pub async fn reconcile_teardown(
    client: &Client,
    teardown: &NamespaceTeardown,
    backend: &dyn TeardownBackend,
) -> Result<NamespaceTeardownStatus, kube::Error> {
    let current = teardown.status.clone().unwrap_or_default();
    let next = advance(&teardown.spec, &current, backend, Utc::now()).await;
    if next != current {
        let ns = teardown.metadata.namespace.clone().unwrap_or_default();
        let name = teardown.metadata.name.clone().unwrap_or_default();
        let api: Api<NamespaceTeardown> = Api::namespaced(client.clone(), &ns);
        let patch = serde_json::json!({ "status": next });
        api.patch_status(&name, &PatchParams::default(), &Patch::Merge(&patch))
            .await?;
        if next.phase == TeardownPhase::Failed {
            warn!(teardown = %name, "namespace teardown failed; see status conditions");
        } else {
            info!(teardown = %name, phase = ?next.phase, "namespace teardown advanced");
        }
    }
    Ok(next)
}
