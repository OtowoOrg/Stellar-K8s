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
//! Namespace-scoped backup consistency groups (epic #1527).
//!
//! A [`ConsistencyGroup`] describes interdependent stateful workloads in one
//! namespace (e.g. DB + cache + queue). Instead of snapshotting each volume in
//! isolation, the group is captured as a single unit:
//!
//! 1. Members are quiesced in *reverse* dependency order (consumers first, so
//!    nothing writes into a dependency after it has been frozen). Members with
//!    an application-native hook use it; the rest fall back to a filesystem
//!    freeze before their volumes are snapshotted.
//! 2. Every volume of every member is snapshotted in one step that shares a
//!    single group timestamp.
//! 3. Members are unquiesced in dependency order. Unquiesce always runs for
//!    every member that was quiesced, even when a later step fails.
//!
//! Restore reconstructs the dependency order (dependencies first), and a
//! post-restore consistency check runs automatically for every member before
//! the group is reported healthy. RPO is evaluated for the whole group using
//! the oldest volume in the snapshot, not per volume.
//!
//! The planning and verification logic is pure; the [`GroupExecutor`] trait
//! is the seam to the Kubernetes API (pod exec, `VolumeSnapshot` objects).

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// Label placed on every `VolumeSnapshot` belonging to a group capture.
pub const GROUP_LABEL: &str = "stellar.org/consistency-group";
/// Label carrying the group capture identifier shared by all volumes.
pub const CAPTURE_LABEL: &str = "stellar.org/consistency-capture";

/// Application-native backup hook commands executed inside the member pod.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppHook {
    /// Command that flushes and blocks writes (e.g. `pg_backup_start`).
    pub quiesce: Vec<String>,
    /// Command that resumes writes.
    pub unquiesce: Vec<String>,
}

/// One stateful workload in the group.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GroupMember {
    /// Workload name (StatefulSet / StellarNode).
    pub name: String,
    /// PVCs owned by this member.
    pub pvcs: Vec<String>,
    /// Members this one depends on (must be restored before it).
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Application-native hook; `None` falls back to a filesystem freeze.
    #[serde(default)]
    pub hook: Option<AppHook>,
    /// Command run after restore; must exit 0 for the member to be consistent.
    #[serde(default)]
    pub verify: Option<Vec<String>>,
}

/// A namespace-scoped set of members captured atomically.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConsistencyGroup {
    pub name: String,
    pub namespace: String,
    pub members: Vec<GroupMember>,
    /// Recovery point objective for the whole group, in seconds.
    pub rpo_seconds: i64,
}

/// How a member is quiesced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuiesceMethod {
    /// Application-native hook.
    AppNative(AppHook),
    /// `fsfreeze` on the member's mounted volumes.
    FilesystemFreeze,
}

/// A single step of a group backup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackupStep {
    Quiesce {
        member: String,
        method: QuiesceMethod,
    },
    /// Snapshot every `(member, pvc)` in the group under one capture.
    SnapshotGroup { volumes: Vec<(String, String)> },
    Unquiesce {
        member: String,
        method: QuiesceMethod,
    },
}

/// A single step of a group restore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreStep {
    RestoreVolumes { member: String, pvcs: Vec<String> },
    StartAndAwaitReady { member: String },
    Verify { member: String },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GroupError {
    #[error("consistency group has no members")]
    Empty,
    #[error("duplicate member '{0}'")]
    DuplicateMember(String),
    #[error("member '{member}' depends on unknown member '{dependency}'")]
    UnknownDependency { member: String, dependency: String },
    #[error("dependency cycle involving: {0:?}")]
    Cycle(Vec<String>),
    #[error("step failed for '{member}': {reason}")]
    StepFailed { member: String, reason: String },
}

impl ConsistencyGroup {
    fn member(&self, name: &str) -> Option<&GroupMember> {
        self.members.iter().find(|m| m.name == name)
    }

    /// Topological order of members, dependencies first. Ties are broken by
    /// name so the order is deterministic.
    pub fn dependency_order(&self) -> Result<Vec<String>, GroupError> {
        if self.members.is_empty() {
            return Err(GroupError::Empty);
        }
        let mut indegree: BTreeMap<&str, usize> = BTreeMap::new();
        for m in &self.members {
            if indegree.insert(&m.name, 0).is_some() {
                return Err(GroupError::DuplicateMember(m.name.clone()));
            }
        }
        for m in &self.members {
            for dep in &m.depends_on {
                if !indegree.contains_key(dep.as_str()) {
                    return Err(GroupError::UnknownDependency {
                        member: m.name.clone(),
                        dependency: dep.clone(),
                    });
                }
            }
            *indegree.get_mut(m.name.as_str()).unwrap() = m.depends_on.len();
        }

        let mut ready: BTreeSet<&str> = indegree
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(n, _)| *n)
            .collect();
        let mut order = Vec::with_capacity(self.members.len());
        while let Some(next) = ready.pop_first() {
            order.push(next.to_string());
            for m in &self.members {
                if m.depends_on.iter().any(|d| d == next) {
                    let d = indegree.get_mut(m.name.as_str()).unwrap();
                    *d -= 1;
                    if *d == 0 {
                        ready.insert(&m.name);
                    }
                }
            }
        }
        if order.len() != self.members.len() {
            let stuck = indegree
                .into_iter()
                .filter(|(n, _)| !order.iter().any(|o| o == n))
                .map(|(n, _)| n.to_string())
                .collect();
            return Err(GroupError::Cycle(stuck));
        }
        Ok(order)
    }

    fn quiesce_method(member: &GroupMember) -> QuiesceMethod {
        match &member.hook {
            Some(h) => QuiesceMethod::AppNative(h.clone()),
            None => QuiesceMethod::FilesystemFreeze,
        }
    }

    /// Ordered backup plan: quiesce consumers-first, one group snapshot,
    /// unquiesce dependencies-first.
    pub fn backup_plan(&self) -> Result<Vec<BackupStep>, GroupError> {
        let order = self.dependency_order()?;
        let mut steps = Vec::new();
        for name in order.iter().rev() {
            let m = self.member(name).unwrap();
            steps.push(BackupStep::Quiesce {
                member: name.clone(),
                method: Self::quiesce_method(m),
            });
        }
        let volumes = order
            .iter()
            .flat_map(|name| {
                let m = self.member(name).unwrap();
                m.pvcs.iter().map(move |p| (name.clone(), p.clone()))
            })
            .collect();
        steps.push(BackupStep::SnapshotGroup { volumes });
        for name in &order {
            let m = self.member(name).unwrap();
            steps.push(BackupStep::Unquiesce {
                member: name.clone(),
                method: Self::quiesce_method(m),
            });
        }
        Ok(steps)
    }

    /// Ordered restore plan: each member is restored, started and verified
    /// before any member that depends on it.
    pub fn restore_plan(&self) -> Result<Vec<RestoreStep>, GroupError> {
        let order = self.dependency_order()?;
        let mut steps = Vec::new();
        for name in order {
            let m = self.member(&name).unwrap();
            steps.push(RestoreStep::RestoreVolumes {
                member: name.clone(),
                pvcs: m.pvcs.clone(),
            });
            steps.push(RestoreStep::StartAndAwaitReady {
                member: name.clone(),
            });
            steps.push(RestoreStep::Verify { member: name });
        }
        Ok(steps)
    }
}

/// One snapshotted volume within a group capture.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VolumeSnapshotRecord {
    pub member: String,
    pub pvc: String,
    pub snapshot_name: String,
    pub ready_at: DateTime<Utc>,
}

/// The result of a group capture.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GroupSnapshot {
    pub group: String,
    pub capture_id: String,
    /// Moment all members were quiesced; the group's recovery point.
    pub captured_at: DateTime<Utc>,
    pub volumes: Vec<VolumeSnapshotRecord>,
}

/// Build a `snapshot.storage.k8s.io/v1` `VolumeSnapshot` for one volume of a
/// group capture. All volumes of one capture share [`CAPTURE_LABEL`].
pub fn volume_snapshot_manifest(
    group: &ConsistencyGroup,
    capture_id: &str,
    member: &str,
    pvc: &str,
    snapshot_class: &str,
) -> serde_json::Value {
    serde_json::json!({
        "apiVersion": "snapshot.storage.k8s.io/v1",
        "kind": "VolumeSnapshot",
        "metadata": {
            "name": format!("{capture_id}-{pvc}"),
            "namespace": group.namespace,
            "labels": {
                GROUP_LABEL: group.name,
                CAPTURE_LABEL: capture_id,
                "stellar.org/consistency-member": member,
            }
        },
        "spec": {
            "volumeSnapshotClassName": snapshot_class,
            "source": { "persistentVolumeClaimName": pvc }
        }
    })
}

/// Group-level RPO check: the recovery point is the capture timestamp, and
/// the group only meets RPO when the capture covers every member volume.
pub fn meets_rpo(group: &ConsistencyGroup, snapshot: &GroupSnapshot, now: DateTime<Utc>) -> bool {
    let complete = group.members.iter().all(|m| {
        m.pvcs.iter().all(|p| {
            snapshot
                .volumes
                .iter()
                .any(|v| v.member == m.name && &v.pvc == p)
        })
    });
    complete && now - snapshot.captured_at <= Duration::seconds(group.rpo_seconds)
}

/// A consistency problem found by the post-restore check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsistencyError {
    pub member: String,
    pub reason: String,
}

/// Outcome of a group restore.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    /// Members in the order they were restored.
    pub restored: Vec<String>,
    pub errors: Vec<ConsistencyError>,
}

impl RestoreReport {
    pub fn is_consistent(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Seam to the cluster. Implementations run pod exec and manage
/// `VolumeSnapshot` / PVC objects.
#[async_trait]
pub trait GroupExecutor: Send + Sync {
    async fn quiesce(&self, member: &GroupMember, method: &QuiesceMethod) -> Result<(), String>;
    async fn unquiesce(&self, member: &GroupMember, method: &QuiesceMethod) -> Result<(), String>;
    /// Snapshot all volumes under one capture id and wait until ready.
    async fn snapshot(
        &self,
        capture_id: &str,
        volumes: &[(String, String)],
    ) -> Result<Vec<VolumeSnapshotRecord>, String>;
    async fn restore_volumes(
        &self,
        member: &GroupMember,
        snapshot: &GroupSnapshot,
    ) -> Result<(), String>;
    async fn start_and_await_ready(&self, member: &GroupMember) -> Result<(), String>;
    /// Run the member's verify command; `Err` carries the failure reason.
    async fn verify(&self, member: &GroupMember) -> Result<(), String>;
}

/// Run a group backup. Every quiesced member is unquiesced even if a later
/// quiesce or the snapshot fails.
pub async fn run_backup<E: GroupExecutor>(
    group: &ConsistencyGroup,
    exec: &E,
    capture_id: &str,
    now: DateTime<Utc>,
) -> Result<GroupSnapshot, GroupError> {
    let plan = group.backup_plan()?;
    let mut quiesced: Vec<(&GroupMember, QuiesceMethod)> = Vec::new();
    let mut result: Result<Vec<VolumeSnapshotRecord>, GroupError> = Err(GroupError::Empty);

    for step in &plan {
        match step {
            BackupStep::Quiesce { member, method } => {
                let m = group.member(member).unwrap();
                if let Err(reason) = exec.quiesce(m, method).await {
                    result = Err(GroupError::StepFailed {
                        member: member.clone(),
                        reason,
                    });
                    break;
                }
                quiesced.push((m, method.clone()));
            }
            BackupStep::SnapshotGroup { volumes } => {
                result = exec.snapshot(capture_id, volumes).await.map_err(|reason| {
                    GroupError::StepFailed {
                        member: group.name.clone(),
                        reason,
                    }
                });
            }
            BackupStep::Unquiesce { .. } => break,
        }
    }

    // Unquiesce in dependency order (reverse of quiesce order).
    for (m, method) in quiesced.iter().rev() {
        if let Err(reason) = exec.unquiesce(m, method).await {
            if result.is_ok() {
                result = Err(GroupError::StepFailed {
                    member: m.name.clone(),
                    reason,
                });
            }
        }
    }

    Ok(GroupSnapshot {
        group: group.name.clone(),
        capture_id: capture_id.to_string(),
        captured_at: now,
        volumes: result?,
    })
}

/// Restore a group in dependency order and run the consistency check for
/// every member. A member whose dependency failed is not started.
pub async fn run_restore<E: GroupExecutor>(
    group: &ConsistencyGroup,
    snapshot: &GroupSnapshot,
    exec: &E,
) -> Result<RestoreReport, GroupError> {
    let order = group.dependency_order()?;
    let mut report = RestoreReport {
        restored: Vec::new(),
        errors: Vec::new(),
    };
    let mut failed: BTreeSet<String> = BTreeSet::new();

    for name in order {
        let m = group.member(&name).unwrap();
        if let Some(dep) = m.depends_on.iter().find(|d| failed.contains(*d)) {
            report.errors.push(ConsistencyError {
                member: name.clone(),
                reason: format!("dependency '{dep}' failed"),
            });
            failed.insert(name);
            continue;
        }
        let outcome = async {
            exec.restore_volumes(m, snapshot).await?;
            exec.start_and_await_ready(m).await?;
            exec.verify(m).await
        }
        .await;
        match outcome {
            Ok(()) => report.restored.push(name),
            Err(reason) => {
                report.errors.push(ConsistencyError {
                    member: name.clone(),
                    reason,
                });
                failed.insert(name);
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn member(name: &str, deps: &[&str], hook: bool) -> GroupMember {
        GroupMember {
            name: name.into(),
            pvcs: vec![format!("{name}-data")],
            depends_on: deps.iter().map(|d| d.to_string()).collect(),
            hook: hook.then(|| AppHook {
                quiesce: vec!["freeze".into()],
                unquiesce: vec!["thaw".into()],
            }),
            verify: Some(vec!["check".into()]),
        }
    }

    /// DB <- cache, DB <- queue.
    fn three_tier() -> ConsistencyGroup {
        ConsistencyGroup {
            name: "app".into(),
            namespace: "stellar".into(),
            members: vec![
                member("queue", &["db"], false),
                member("cache", &["db"], false),
                member("db", &[], true),
            ],
            rpo_seconds: 3600,
        }
    }

    #[derive(Default)]
    struct Recorder {
        log: Mutex<Vec<String>>,
        fail_quiesce: Option<&'static str>,
        fail_verify: Option<&'static str>,
    }

    #[async_trait]
    impl GroupExecutor for Recorder {
        async fn quiesce(&self, m: &GroupMember, _: &QuiesceMethod) -> Result<(), String> {
            self.log.lock().unwrap().push(format!("quiesce:{}", m.name));
            if self.fail_quiesce == Some(m.name.as_str()) {
                return Err("boom".into());
            }
            Ok(())
        }
        async fn unquiesce(&self, m: &GroupMember, _: &QuiesceMethod) -> Result<(), String> {
            self.log
                .lock()
                .unwrap()
                .push(format!("unquiesce:{}", m.name));
            Ok(())
        }
        async fn snapshot(
            &self,
            id: &str,
            volumes: &[(String, String)],
        ) -> Result<Vec<VolumeSnapshotRecord>, String> {
            self.log.lock().unwrap().push("snapshot".into());
            Ok(volumes
                .iter()
                .map(|(m, p)| VolumeSnapshotRecord {
                    member: m.clone(),
                    pvc: p.clone(),
                    snapshot_name: format!("{id}-{p}"),
                    ready_at: Utc::now(),
                })
                .collect())
        }
        async fn restore_volumes(&self, m: &GroupMember, _: &GroupSnapshot) -> Result<(), String> {
            self.log.lock().unwrap().push(format!("restore:{}", m.name));
            Ok(())
        }
        async fn start_and_await_ready(&self, _: &GroupMember) -> Result<(), String> {
            Ok(())
        }
        async fn verify(&self, m: &GroupMember) -> Result<(), String> {
            self.log.lock().unwrap().push(format!("verify:{}", m.name));
            if self.fail_verify == Some(m.name.as_str()) {
                return Err("integrity check failed".into());
            }
            Ok(())
        }
    }

    #[test]
    fn dependency_order_is_deterministic() {
        assert_eq!(
            three_tier().dependency_order().unwrap(),
            vec!["db", "cache", "queue"]
        );
    }

    #[test]
    fn rejects_cycles_and_unknown_deps() {
        let mut g = three_tier();
        g.members[2].depends_on = vec!["queue".into()];
        assert!(matches!(g.dependency_order(), Err(GroupError::Cycle(_))));

        let mut g = three_tier();
        g.members[0].depends_on = vec!["missing".into()];
        assert!(matches!(
            g.dependency_order(),
            Err(GroupError::UnknownDependency { .. })
        ));
    }

    #[test]
    fn backup_plan_uses_native_hook_or_fs_fallback() {
        let plan = three_tier().backup_plan().unwrap();
        assert_eq!(plan.len(), 7);
        assert_eq!(
            plan[2],
            BackupStep::Quiesce {
                member: "db".into(),
                method: QuiesceMethod::AppNative(AppHook {
                    quiesce: vec!["freeze".into()],
                    unquiesce: vec!["thaw".into()],
                }),
            }
        );
        assert!(matches!(
            &plan[0],
            BackupStep::Quiesce {
                method: QuiesceMethod::FilesystemFreeze,
                ..
            }
        ));
        match &plan[3] {
            BackupStep::SnapshotGroup { volumes } => assert_eq!(volumes.len(), 3),
            other => panic!("expected group snapshot, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn three_tier_backup_and_restore_respects_order() {
        let g = three_tier();
        let exec = Recorder::default();
        let snap = run_backup(&g, &exec, "cap-1", Utc::now()).await.unwrap();
        assert_eq!(snap.volumes.len(), 3);

        let report = run_restore(&g, &snap, &exec).await.unwrap();
        assert!(report.is_consistent());
        assert_eq!(report.restored, vec!["db", "cache", "queue"]);

        let log = exec.log.lock().unwrap().clone();
        assert_eq!(
            log,
            vec![
                "quiesce:queue",
                "quiesce:cache",
                "quiesce:db",
                "snapshot",
                "unquiesce:db",
                "unquiesce:cache",
                "unquiesce:queue",
                "restore:db",
                "verify:db",
                "restore:cache",
                "verify:cache",
                "restore:queue",
                "verify:queue",
            ]
        );
    }

    #[tokio::test]
    async fn failed_quiesce_still_unquiesces_and_skips_snapshot() {
        let g = three_tier();
        let exec = Recorder {
            fail_quiesce: Some("cache"),
            ..Default::default()
        };
        let err = run_backup(&g, &exec, "cap-1", Utc::now())
            .await
            .unwrap_err();
        assert!(matches!(err, GroupError::StepFailed { member, .. } if member == "cache"));
        let log = exec.log.lock().unwrap().clone();
        assert_eq!(
            log,
            vec!["quiesce:queue", "quiesce:cache", "unquiesce:queue"]
        );
    }

    #[tokio::test]
    async fn failed_verify_blocks_dependents() {
        let g = three_tier();
        let exec = Recorder {
            fail_verify: Some("db"),
            ..Default::default()
        };
        let snap = run_backup(&g, &exec, "cap-1", Utc::now()).await.unwrap();
        let report = run_restore(&g, &snap, &exec).await.unwrap();
        assert!(!report.is_consistent());
        assert!(report.restored.is_empty());
        assert_eq!(report.errors.len(), 3);
        assert_eq!(report.errors[1].reason, "dependency 'db' failed");
    }

    #[test]
    fn rpo_is_evaluated_for_the_whole_group() {
        let g = three_tier();
        let t0 = Utc::now();
        let vol = |m: &str| VolumeSnapshotRecord {
            member: m.into(),
            pvc: format!("{m}-data"),
            snapshot_name: "s".into(),
            ready_at: t0,
        };
        let mut snap = GroupSnapshot {
            group: "app".into(),
            capture_id: "c".into(),
            captured_at: t0,
            volumes: vec![vol("db"), vol("cache"), vol("queue")],
        };
        assert!(meets_rpo(&g, &snap, t0 + Duration::minutes(30)));
        assert!(!meets_rpo(&g, &snap, t0 + Duration::hours(2)));
        snap.volumes.pop();
        assert!(!meets_rpo(&g, &snap, t0));
    }

    #[test]
    fn snapshot_manifest_carries_group_labels() {
        let g = three_tier();
        let m = volume_snapshot_manifest(&g, "cap-1", "db", "db-data", "csi");
        assert_eq!(m["metadata"]["labels"][GROUP_LABEL], "app");
        assert_eq!(m["metadata"]["labels"][CAPTURE_LABEL], "cap-1");
        assert_eq!(m["spec"]["source"]["persistentVolumeClaimName"], "db-data");
    }
}
