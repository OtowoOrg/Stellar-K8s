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
//! Runtime Drift Detection Between Git Source of Truth and Live Cluster
//!
//! Compares a live object against the state declared in Git with three-way
//! semantics and reports every divergence as a structured JSON Patch.
//!
//! # Design
//!
//! - **base** is the Git state last applied to the cluster, **desired** is the
//!   current Git state and **live** is the object read from the API server.
//! - The live object is first projected onto the fields declared in base or
//!   desired, so fields the API server or mutating webhooks default
//!   (`imagePullPolicy`, `terminationMessagePath`, `status`, server metadata)
//!   never show up as drift. User maps (`metadata.labels`,
//!   `metadata.annotations`) are kept whole so out-of-band additions there
//!   are still caught.
//! - Each differing path between desired and live is classified:
//!   [`DriftKind::ManualMutation`] when Git did not change it since base,
//!   [`DriftKind::PendingPropagation`] when live still matches base, and
//!   [`DriftKind::Conflict`] when both moved.
//! - The last-modifying actor is taken from the live object's
//!   `metadata.managedFields` (the newest entry not owned by the GitOps
//!   applier).
//! - [`plan_revert`] only reverts manual mutations, and only after the
//!   rollback safety checks pass. The live state is snapshotted in the plan
//!   so a revert can itself be undone.
//!
//! ## Acceptance Criteria (from #1970)
//! - Drift detected within 60s of mutation (pure, per-object diff that is
//!   cheap enough to run on every watch event)
//! - Zero false drift for server-side-defaulted fields (projection onto
//!   declared fields)
//! - Auto-revert path covered by rollback safety checks ([`plan_revert`])
//! - Report attributes drift to last-modifying actor ([`DriftActor`])

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::degradation::{DegradationGate, OperatorAction};

/// Metadata fields owned by the API server rather than by Git.
const SERVER_METADATA_FIELDS: &[&str] = &[
    "creationTimestamp",
    "deletionGracePeriodSeconds",
    "deletionTimestamp",
    "generation",
    "managedFields",
    "resourceVersion",
    "selfLink",
    "uid",
];

/// Annotations written by tooling, never part of the declared intent.
const IGNORED_ANNOTATIONS: &[&str] = &[
    "kubectl.kubernetes.io/last-applied-configuration",
    "deployment.kubernetes.io/revision",
];

/// How a drifted path diverged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DriftKind {
    /// Changed in the cluster out of band; Git did not change.
    ManualMutation,
    /// Changed in Git but not yet applied to the cluster.
    PendingPropagation,
    /// Changed in both Git and the cluster.
    Conflict,
}

/// One drifted path with the value on each side.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriftEntry {
    /// JSON Pointer to the drifted field.
    pub path: String,
    pub kind: DriftKind,
    pub desired: Option<Value>,
    pub live: Option<Value>,
}

/// Field manager that last wrote the live object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriftActor {
    pub manager: String,
    pub operation: Option<String>,
    pub time: Option<String>,
}

/// Drift report for a single object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitDriftReport {
    pub detected_at: String,
    pub entries: Vec<DriftEntry>,
    /// JSON Patch that takes the projected live object to the Git state.
    pub patch: Value,
    pub last_modified_by: Option<DriftActor>,
}

impl GitDriftReport {
    pub fn has_drift(&self) -> bool {
        !self.entries.is_empty()
    }

    pub fn count(&self, kind: DriftKind) -> usize {
        self.entries.iter().filter(|e| e.kind == kind).count()
    }
}

/// Computes the drift of `live` from `desired`.
///
/// `base` is the Git state last applied; pass `None` when unknown, in which
/// case every divergence is treated as a manual mutation.
/// `applier_manager` is the field manager the GitOps controller applies with,
/// excluded from actor attribution.
pub fn detect(
    base: Option<&Value>,
    desired: &Value,
    live: &Value,
    applier_manager: &str,
    now: DateTime<Utc>,
) -> GitDriftReport {
    let base = base.map(normalize).unwrap_or_else(|| normalize(desired));
    let desired = normalize(desired);
    let shape = merge_shape(&base, &desired);
    let live_projected = project(&normalize(live), &shape, "");

    let patch = json_patch::diff(&live_projected, &desired);
    let patch = serde_json::to_value(&patch).unwrap_or_else(|_| Value::Array(Vec::new()));

    let entries = patch_paths(&patch)
        .into_iter()
        .map(|path| {
            let at = |v: &Value| v.pointer(&path).cloned();
            let (b, d, l) = (at(&base), at(&desired), at(&live_projected));
            let kind = if b == d {
                DriftKind::ManualMutation
            } else if b == l {
                DriftKind::PendingPropagation
            } else {
                DriftKind::Conflict
            };
            DriftEntry {
                path,
                kind,
                desired: d,
                live: l,
            }
        })
        .collect();

    GitDriftReport {
        detected_at: now.to_rfc3339(),
        entries,
        patch,
        last_modified_by: last_modifier(live, applier_manager),
    }
}

/// Why an auto-revert was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevertBlocked {
    /// Auto-revert is not enabled for this resource class.
    NotEnabled,
    /// No manual mutations to revert.
    NothingToRevert,
    /// Git also changed the drifted fields; a human has to decide.
    Conflicting,
    /// The object is being deleted.
    Terminating,
    /// The control plane is degraded and withholds writes.
    ControlPlaneDegraded,
}

/// A revert that passed every safety check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevertPlan {
    /// JSON Patch reverting only the manual mutations.
    pub patch: Value,
    /// Live object before the revert, kept so the revert can be undone.
    pub snapshot: Value,
    pub reverted_paths: Vec<String>,
    pub justification: String,
}

/// Builds a revert for the manual mutations in `report`, after the rollback
/// safety checks: auto-revert enabled, no conflicting Git change, object not
/// terminating and the control plane accepting writes.
pub fn plan_revert(
    report: &GitDriftReport,
    live: &Value,
    auto_revert_enabled: bool,
    gate: &DegradationGate,
) -> Result<RevertPlan, RevertBlocked> {
    if !auto_revert_enabled {
        return Err(RevertBlocked::NotEnabled);
    }
    if report.count(DriftKind::Conflict) > 0 {
        return Err(RevertBlocked::Conflicting);
    }
    let reverted_paths: Vec<String> = report
        .entries
        .iter()
        .filter(|e| e.kind == DriftKind::ManualMutation)
        .map(|e| e.path.clone())
        .collect();
    if reverted_paths.is_empty() {
        return Err(RevertBlocked::NothingToRevert);
    }
    if live.pointer("/metadata/deletionTimestamp").is_some() {
        return Err(RevertBlocked::Terminating);
    }
    if gate.check(OperatorAction::Write).is_err() {
        return Err(RevertBlocked::ControlPlaneDegraded);
    }

    let ops = report
        .patch
        .as_array()
        .into_iter()
        .flatten()
        .filter(|op| {
            op.get("path")
                .and_then(Value::as_str)
                .is_some_and(|p| reverted_paths.iter().any(|r| r == p))
        })
        .cloned()
        .collect();
    let actor = report
        .last_modified_by
        .as_ref()
        .map_or("unknown", |a| a.manager.as_str());

    Ok(RevertPlan {
        patch: Value::Array(ops),
        snapshot: live.clone(),
        justification: format!(
            "Reverting {} out-of-band change(s) by '{}' to the Git-declared state",
            reverted_paths.len(),
            actor
        ),
        reverted_paths,
    })
}

/// Strips server-owned metadata, tooling annotations and `status`.
fn normalize(obj: &Value) -> Value {
    let mut obj = obj.clone();
    if let Some(map) = obj.as_object_mut() {
        map.remove("status");
        if let Some(meta) = map.get_mut("metadata").and_then(Value::as_object_mut) {
            for field in SERVER_METADATA_FIELDS {
                meta.remove(*field);
            }
            if let Some(ann) = meta.get_mut("annotations").and_then(Value::as_object_mut) {
                for key in IGNORED_ANNOTATIONS {
                    ann.remove(*key);
                }
                if ann.is_empty() {
                    meta.remove("annotations");
                }
            }
        }
    }
    obj
}

/// Union of the fields declared in `a` and `b`.
fn merge_shape(a: &Value, b: &Value) -> Value {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let mut out = x.clone();
            for (k, v) in y {
                let merged = match x.get(k) {
                    Some(existing) => merge_shape(existing, v),
                    None => v.clone(),
                };
                out.insert(k.clone(), merged);
            }
            Value::Object(out)
        }
        (Value::Array(x), Value::Array(y)) => {
            let len = x.len().max(y.len());
            Value::Array(
                (0..len)
                    .map(|i| match (x.get(i), y.get(i)) {
                        (Some(p), Some(q)) => merge_shape(p, q),
                        (Some(v), None) | (None, Some(v)) => v.clone(),
                        (None, None) => Value::Null,
                    })
                    .collect(),
            )
        }
        (_, other) => other.clone(),
    }
}

/// Keeps only the parts of `live` declared in `shape`. Array elements beyond
/// the declared length and keys of user maps are kept so additions are seen.
fn project(live: &Value, shape: &Value, path: &str) -> Value {
    if path == "/metadata/labels" || path == "/metadata/annotations" {
        return live.clone();
    }
    match (live, shape) {
        (Value::Object(l), Value::Object(s)) => Value::Object(
            l.iter()
                .filter_map(|(k, v)| {
                    s.get(k)
                        .map(|sv| (k.clone(), project(v, sv, &format!("{path}/{}", escape(k)))))
                })
                .collect::<Map<_, _>>(),
        ),
        (Value::Array(l), Value::Array(s)) => Value::Array(
            l.iter()
                .enumerate()
                .map(|(i, v)| match s.get(i) {
                    Some(sv) => project(v, sv, &format!("{path}/{i}")),
                    None => v.clone(),
                })
                .collect(),
        ),
        _ => live.clone(),
    }
}

fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

fn patch_paths(patch: &Value) -> Vec<String> {
    patch
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|op| op.get("path").and_then(Value::as_str).map(str::to_string))
        .collect()
}

/// Newest `managedFields` entry not owned by the GitOps applier.
fn last_modifier(live: &Value, applier_manager: &str) -> Option<DriftActor> {
    live.pointer("/metadata/managedFields")?
        .as_array()?
        .iter()
        .filter_map(|entry| {
            let manager = entry.get("manager")?.as_str()?;
            (manager != applier_manager).then(|| DriftActor {
                manager: manager.to_string(),
                operation: entry
                    .get("operation")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                time: entry.get("time").and_then(Value::as_str).map(str::to_string),
            })
        })
        // RFC 3339 timestamps in UTC sort lexicographically.
        .max_by(|a, b| a.time.cmp(&b.time))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::degradation::DegradationLevel;
    use serde_json::json;

    fn git() -> Value {
        json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": { "name": "horizon", "labels": { "app": "horizon" } },
            "spec": {
                "replicas": 2,
                "template": { "spec": { "containers": [
                    { "name": "horizon", "image": "stellar/horizon:2.30" }
                ]}}
            }
        })
    }

    fn live_with_defaults() -> Value {
        json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": {
                "name": "horizon",
                "labels": { "app": "horizon" },
                "uid": "1234",
                "resourceVersion": "99",
                "generation": 3,
                "annotations": { "deployment.kubernetes.io/revision": "3" },
                "managedFields": [
                    { "manager": "stellar-gitops", "operation": "Apply", "time": "2026-11-01T10:00:00Z" },
                    { "manager": "kubectl-edit", "operation": "Update", "time": "2026-11-02T09:30:00Z" },
                    { "manager": "kube-controller-manager", "operation": "Update", "time": "2026-11-01T11:00:00Z" }
                ]
            },
            "spec": {
                "replicas": 2,
                "revisionHistoryLimit": 10,
                "progressDeadlineSeconds": 600,
                "template": { "spec": {
                    "restartPolicy": "Always",
                    "containers": [{
                        "name": "horizon",
                        "image": "stellar/horizon:2.30",
                        "imagePullPolicy": "IfNotPresent",
                        "terminationMessagePath": "/dev/termination-log"
                    }]
                }}
            },
            "status": { "readyReplicas": 2 }
        })
    }

    fn now() -> DateTime<Utc> {
        "2026-11-02T10:00:00Z".parse().unwrap()
    }

    #[test]
    fn server_defaults_are_not_drift() {
        let report = detect(Some(&git()), &git(), &live_with_defaults(), "stellar-gitops", now());
        assert!(!report.has_drift(), "{:?}", report.entries);
    }

    #[test]
    fn classifies_manual_pending_and_conflict() {
        let base = git();
        let mut desired = git();
        desired["spec"]["template"]["spec"]["containers"][0]["image"] =
            json!("stellar/horizon:2.31");
        desired["metadata"]["labels"]["tier"] = json!("api");
        let mut live = live_with_defaults();
        live["spec"]["replicas"] = json!(5);
        live["metadata"]["labels"]["tier"] = json!("debug");

        let report = detect(Some(&base), &desired, &live, "stellar-gitops", now());
        let kind_of = |p: &str| report.entries.iter().find(|e| e.path == p).map(|e| e.kind);

        assert_eq!(kind_of("/spec/replicas"), Some(DriftKind::ManualMutation));
        assert_eq!(
            kind_of("/spec/template/spec/containers/0/image"),
            Some(DriftKind::PendingPropagation)
        );
        assert_eq!(kind_of("/metadata/labels/tier"), Some(DriftKind::Conflict));
        assert_eq!(report.entries.len(), 3);
    }

    #[test]
    fn out_of_band_label_addition_is_detected() {
        let mut live = live_with_defaults();
        live["metadata"]["labels"]["hotfix"] = json!("true");
        let report = detect(Some(&git()), &git(), &live, "stellar-gitops", now());
        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].path, "/metadata/labels/hotfix");
        assert_eq!(report.entries[0].kind, DriftKind::ManualMutation);
    }

    #[test]
    fn attributes_drift_to_last_non_applier_manager() {
        let report = detect(Some(&git()), &git(), &live_with_defaults(), "stellar-gitops", now());
        let actor = report.last_modified_by.unwrap();
        assert_eq!(actor.manager, "kubectl-edit");
        assert_eq!(actor.operation.as_deref(), Some("Update"));
    }

    #[test]
    fn revert_plan_reverts_only_manual_mutations() {
        let mut live = live_with_defaults();
        live["spec"]["replicas"] = json!(5);
        let report = detect(Some(&git()), &git(), &live, "stellar-gitops", now());

        let plan = plan_revert(&report, &live, true, &DegradationGate::default()).unwrap();
        assert_eq!(plan.reverted_paths, ["/spec/replicas"]);
        assert_eq!(
            plan.patch,
            json!([{ "op": "replace", "path": "/spec/replicas", "value": 2 }])
        );
        assert_eq!(plan.snapshot, live);
        assert!(plan.justification.contains("kubectl-edit"));
    }

    #[test]
    fn revert_safety_checks_block_unsafe_reverts() {
        let mut live = live_with_defaults();
        live["spec"]["replicas"] = json!(5);
        let report = detect(Some(&git()), &git(), &live, "stellar-gitops", now());
        let gate = DegradationGate::default();

        assert_eq!(
            plan_revert(&report, &live, false, &gate),
            Err(RevertBlocked::NotEnabled)
        );

        let frozen = DegradationGate::default();
        frozen.set_level(DegradationLevel::Frozen);
        assert_eq!(
            plan_revert(&report, &live, true, &frozen),
            Err(RevertBlocked::ControlPlaneDegraded)
        );

        let mut terminating = live.clone();
        terminating["metadata"]["deletionTimestamp"] = json!("2026-11-02T09:59:00Z");
        assert_eq!(
            plan_revert(&report, &terminating, true, &gate),
            Err(RevertBlocked::Terminating)
        );

        let clean = detect(Some(&git()), &git(), &live_with_defaults(), "stellar-gitops", now());
        assert_eq!(
            plan_revert(&clean, &live_with_defaults(), true, &gate),
            Err(RevertBlocked::NothingToRevert)
        );

        let mut desired = git();
        desired["spec"]["replicas"] = json!(3);
        let conflict = detect(Some(&git()), &desired, &live, "stellar-gitops", now());
        assert_eq!(
            plan_revert(&conflict, &live, true, &gate),
            Err(RevertBlocked::Conflicting)
        );
    }
}
