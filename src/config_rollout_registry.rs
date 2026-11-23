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
//! Queryable record of progressive config rollouts (#1998).
//!
//! [`crate::progressive_config::ProgressiveConfigRollout`] owns the state of
//! one rollout in memory. This registry is the read side: whoever drives a
//! rollout publishes a [`ConfigRolloutSnapshot`] after each step, and the REST
//! API (`rest_api::config_rollout_handlers`) answers "which config is this
//! component running?" and "where is this rollout?" from it without touching
//! the engine.
//!
//! Snapshots are plain data, so a reader never holds a lock on a rollout and a
//! rollout never waits on a reader.

use std::collections::BTreeMap;
use std::sync::{OnceLock, RwLock};

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::progressive_config::{
    ComponentConfigVersion, ProgressiveConfigRollout, RollbackRecord, RolloutStage,
    MAX_BLAST_RADIUS,
};

/// Point-in-time view of one rollout.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigRolloutSnapshot {
    /// Stable identifier of the rollout, e.g. `operator-settings`.
    pub id: String,
    /// Version of the bundle being rolled out.
    pub bundle_version: String,
    pub stage: RolloutStage,
    /// Fraction of the fleet currently exposed to the candidate bundle.
    pub blast_radius: f64,
    /// Ceiling the canary is held to (5%).
    pub max_blast_radius: f64,
    /// `namespace/node` of every canary target.
    pub canary: Vec<String>,
    pub target_count: usize,
    /// Gates that failed in the latest evaluation, if any.
    pub failing_gates: Vec<String>,
    /// Most recent automatic rollback, with its measured restore time.
    pub last_rollback: Option<RollbackRecord>,
    /// Config version of every component the rollout has touched, keyed by
    /// `namespace/node`.
    pub versions: BTreeMap<String, ComponentConfigVersion>,
    pub updated_at: DateTime<Utc>,
}

impl ConfigRolloutSnapshot {
    /// Capture the current state of `rollout`.
    pub fn from_rollout(
        id: &str,
        bundle_version: &str,
        rollout: &ProgressiveConfigRollout,
    ) -> Self {
        Self {
            id: id.to_string(),
            bundle_version: bundle_version.to_string(),
            stage: rollout.stage(),
            blast_radius: rollout.current_blast_radius(),
            max_blast_radius: MAX_BLAST_RADIUS,
            canary: rollout.canary().iter().map(|t| t.id()).collect(),
            target_count: rollout.targets().len(),
            failing_gates: rollout
                .reports()
                .last()
                .map(|report| {
                    report
                        .failing_gates()
                        .into_iter()
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            last_rollback: rollout.rollbacks().last().cloned(),
            versions: rollout.all_versions().clone(),
            updated_at: Utc::now(),
        }
    }
}

/// Snapshots of every known rollout, keyed by rollout id.
#[derive(Debug, Default)]
pub struct ConfigRolloutRegistry {
    rollouts: RwLock<BTreeMap<String, ConfigRolloutSnapshot>>,
}

impl ConfigRolloutRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a snapshot, replacing the previous one for the same rollout id.
    pub fn publish(&self, snapshot: ConfigRolloutSnapshot) {
        if let Ok(mut rollouts) = self.rollouts.write() {
            rollouts.insert(snapshot.id.clone(), snapshot);
        }
    }

    /// Capture `rollout` and store it under `id`.
    pub fn publish_rollout(
        &self,
        id: &str,
        bundle_version: &str,
        rollout: &ProgressiveConfigRollout,
    ) {
        self.publish(ConfigRolloutSnapshot::from_rollout(
            id,
            bundle_version,
            rollout,
        ));
    }

    pub fn get(&self, id: &str) -> Option<ConfigRolloutSnapshot> {
        self.rollouts.read().ok()?.get(id).cloned()
    }

    /// Every snapshot, ordered by rollout id.
    pub fn list(&self) -> Vec<ConfigRolloutSnapshot> {
        self.rollouts
            .read()
            .map(|rollouts| rollouts.values().cloned().collect())
            .unwrap_or_default()
    }

    /// The config version a component is running. When several rollouts have
    /// touched the same component, the most recently published one wins.
    pub fn component_version(&self, namespace: &str, node: &str) -> Option<ComponentConfigVersion> {
        let key = format!("{namespace}/{node}");
        self.rollouts
            .read()
            .ok()?
            .values()
            .filter(|snapshot| snapshot.versions.contains_key(&key))
            .max_by_key(|snapshot| snapshot.updated_at)
            .and_then(|snapshot| snapshot.versions.get(&key).cloned())
    }

    /// Latest config version of every component, keyed by `namespace/node`.
    pub fn component_versions(&self) -> BTreeMap<String, ComponentConfigVersion> {
        let mut snapshots = self.list();
        snapshots.sort_by_key(|snapshot| snapshot.updated_at);
        let mut merged = BTreeMap::new();
        for snapshot in snapshots {
            merged.extend(snapshot.versions);
        }
        merged
    }
}

/// The operator-wide rollout registry.
pub fn global() -> &'static ConfigRolloutRegistry {
    static GLOBAL: OnceLock<ConfigRolloutRegistry> = OnceLock::new();
    GLOBAL.get_or_init(ConfigRolloutRegistry::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progressive_config::{ConfigBundle, ManualClock, RolloutTarget};
    use std::sync::Arc;

    fn rollout_with(targets: &[(&str, &str)], version: &str) -> ProgressiveConfigRollout {
        let clock = Arc::new(ManualClock::new(1_000));
        let targets: Vec<RolloutTarget> = targets
            .iter()
            .map(|(namespace, node)| RolloutTarget::new(namespace, node))
            .collect();
        let mut rollout = ProgressiveConfigRollout::new(targets.clone(), clock);
        let bundle = ConfigBundle::from_pairs(version, &[("replica_count", "1")]);
        for target in &targets {
            rollout.register_active(target, &bundle);
        }
        rollout
    }

    #[test]
    fn snapshot_captures_stage_versions_and_the_blast_radius_ceiling() {
        let rollout = rollout_with(&[("ns-a", "node-1"), ("ns-a", "node-2")], "2.4.0");
        let snapshot = ConfigRolloutSnapshot::from_rollout("operator-settings", "2.4.0", &rollout);

        assert_eq!(snapshot.id, "operator-settings");
        assert_eq!(snapshot.stage, RolloutStage::Idle);
        assert_eq!(snapshot.target_count, 2);
        assert_eq!(snapshot.max_blast_radius, MAX_BLAST_RADIUS);
        assert_eq!(snapshot.versions.len(), 2);
        assert_eq!(snapshot.versions["ns-a/node-1"].version, "2.4.0");
        assert!(snapshot.failing_gates.is_empty());
        assert!(snapshot.last_rollback.is_none());
    }

    #[test]
    fn publish_replaces_the_snapshot_for_the_same_rollout() {
        let registry = ConfigRolloutRegistry::new();
        registry.publish_rollout("r1", "1.0.0", &rollout_with(&[("ns", "a")], "1.0.0"));
        registry.publish_rollout("r1", "1.1.0", &rollout_with(&[("ns", "a")], "1.1.0"));

        assert_eq!(registry.list().len(), 1);
        assert_eq!(registry.get("r1").unwrap().bundle_version, "1.1.0");
        assert!(registry.get("missing").is_none());
    }

    #[test]
    fn component_version_is_found_across_rollouts() {
        let registry = ConfigRolloutRegistry::new();
        registry.publish_rollout("r1", "1.0.0", &rollout_with(&[("ns", "a")], "1.0.0"));
        registry.publish_rollout("r2", "3.0.0", &rollout_with(&[("ns", "b")], "3.0.0"));

        assert_eq!(
            registry.component_version("ns", "a").unwrap().version,
            "1.0.0"
        );
        assert_eq!(
            registry.component_version("ns", "b").unwrap().version,
            "3.0.0"
        );
        assert!(registry.component_version("ns", "unknown").is_none());
        assert_eq!(registry.component_versions().len(), 2);
    }

    #[test]
    fn the_most_recent_rollout_wins_for_a_shared_component() {
        let registry = ConfigRolloutRegistry::new();
        registry.publish_rollout("old", "1.0.0", &rollout_with(&[("ns", "a")], "1.0.0"));
        std::thread::sleep(std::time::Duration::from_millis(5));
        registry.publish_rollout("new", "2.0.0", &rollout_with(&[("ns", "a")], "2.0.0"));

        assert_eq!(
            registry.component_version("ns", "a").unwrap().version,
            "2.0.0"
        );
        assert_eq!(registry.component_versions()["ns/a"].version, "2.0.0");
    }
}
