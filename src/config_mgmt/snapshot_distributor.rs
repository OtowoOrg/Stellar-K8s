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
//! Streaming Data Plane Config Distribution with Versioned Snapshots.
//!
//! Replaces per-object watch fan-out with versioned, content-addressed snapshots
//! (SHA-256 Merkle root) and delta updates. Agents run a two-phase apply:
//! verify the staged snapshot fully, then atomically swap the active pointer.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use tokio::sync::watch;

/// Versioned, content-addressed dataplane configuration snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfigSnapshotBundle {
    pub version: u64,
    pub merkle_root: String,
    pub objects: BTreeMap<String, serde_json::Value>,
    pub generated_at: DateTime<Utc>,
    pub serialized_bytes: usize,
}

/// Delta snapshot encoding only changed and removed keys relative to a prior version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeltaSnapshot {
    pub version: u64,
    pub base_version: u64,
    pub base_merkle_root: String,
    pub target_merkle_root: String,
    pub upserts: BTreeMap<String, serde_json::Value>,
    pub removals: Vec<String>,
    pub delta_bytes: usize,
}

impl DeltaSnapshot {
    /// Bandwidth reduction percentage compared to sending the full snapshot.
    pub fn bandwidth_savings_percent(&self, full_snapshot_bytes: usize) -> f64 {
        if full_snapshot_bytes == 0 {
            return 0.0;
        }
        let saved = full_snapshot_bytes.saturating_sub(self.delta_bytes);
        (saved as f64 / full_snapshot_bytes as f64) * 100.0
    }
}

/// Builds content-addressed snapshots and steady-state deltas.
pub struct SnapshotBuilder;

impl SnapshotBuilder {
    /// Compute canonical SHA-256 Merkle root over sorted `(key, value)` pairs.
    pub fn compute_merkle_root(objects: &BTreeMap<String, serde_json::Value>) -> String {
        let mut hasher = Sha256::new();
        for (k, v) in objects {
            hasher.update(k.as_bytes());
            hasher.update(b"=");
            let encoded = serde_json::to_vec(v).unwrap_or_default();
            hasher.update(&encoded);
            hasher.update(b"\n");
        }
        format!("{:x}", hasher.finalize())
    }

    /// Build a full versioned snapshot bundle.
    pub fn build(
        version: u64,
        objects: BTreeMap<String, serde_json::Value>,
    ) -> ConfigSnapshotBundle {
        let merkle_root = Self::compute_merkle_root(&objects);
        let serialized_bytes = serde_json::to_vec(&objects).map(|b| b.len()).unwrap_or(0);
        ConfigSnapshotBundle {
            version,
            merkle_root,
            objects,
            generated_at: Utc::now(),
            serialized_bytes,
        }
    }

    /// Compute a delta snapshot between `base` and `target`.
    pub fn diff(base: &ConfigSnapshotBundle, target: &ConfigSnapshotBundle) -> DeltaSnapshot {
        let mut upserts = BTreeMap::new();
        let mut removals = Vec::new();

        for (k, v) in &target.objects {
            if base.objects.get(k) != Some(v) {
                upserts.insert(k.clone(), v.clone());
            }
        }
        for k in base.objects.keys() {
            if !target.objects.contains_key(k) {
                removals.push(k.clone());
            }
        }

        let delta_bytes = serde_json::to_vec(&(&upserts, &removals))
            .map(|b| b.len())
            .unwrap_or(0);

        DeltaSnapshot {
            version: target.version,
            base_version: base.version,
            base_merkle_root: base.merkle_root.clone(),
            target_merkle_root: target.merkle_root.clone(),
            upserts,
            removals,
            delta_bytes,
        }
    }
}

/// Streaming distribution hub broadcasting versioned snapshots to dataplane agents.
pub struct SnapshotStreamHub {
    tx: watch::Sender<Arc<ConfigSnapshotBundle>>,
}

impl SnapshotStreamHub {
    pub fn new(initial: ConfigSnapshotBundle) -> Self {
        let (tx, _) = watch::channel(Arc::new(initial));
        Self { tx }
    }

    pub fn publish(&self, snapshot: ConfigSnapshotBundle) {
        let _ = self.tx.send(Arc::new(snapshot));
    }

    pub fn subscribe(&self) -> watch::Receiver<Arc<ConfigSnapshotBundle>> {
        self.tx.subscribe()
    }

    pub fn current(&self) -> Arc<ConfigSnapshotBundle> {
        self.tx.borrow().clone()
    }
}

/// Two-phase atomic snapshot applier on dataplane agents.
///
/// Phase 1: Stage and verify content-addressed Merkle root.
/// Phase 2: Atomically swap active snapshot pointer (zero partial state).
pub struct TwoPhaseAgentApplier {
    active: RwLock<Arc<ConfigSnapshotBundle>>,
    previous: RwLock<Option<Arc<ConfigSnapshotBundle>>>,
    alerts: RwLock<Vec<String>>,
}

impl TwoPhaseAgentApplier {
    pub fn new(initial: ConfigSnapshotBundle) -> Result<Self, String> {
        let expected = SnapshotBuilder::compute_merkle_root(&initial.objects);
        if expected != initial.merkle_root {
            return Err("Initial snapshot failed Merkle root verification".to_string());
        }
        Ok(Self {
            active: RwLock::new(Arc::new(initial)),
            previous: RwLock::new(None),
            alerts: RwLock::new(Vec::new()),
        })
    }

    /// Return an atomic handle to the currently active snapshot.
    pub fn active_snapshot(&self) -> Arc<ConfigSnapshotBundle> {
        self.active.read().expect("lock poisoned").clone()
    }

    /// Recorded verification failure alerts.
    pub fn alerts(&self) -> Vec<String> {
        self.alerts.read().expect("lock poisoned").clone()
    }

    /// Two-phase apply of a full snapshot.
    pub fn apply_full(&self, candidate: ConfigSnapshotBundle) -> Result<u64, String> {
        // Phase 1: Verify integrity before touching active pointer
        let computed = SnapshotBuilder::compute_merkle_root(&candidate.objects);
        if computed != candidate.merkle_root {
            let msg = format!(
                "Snapshot v{} failed verification (expected {}, got {}); retaining prior valid snapshot",
                candidate.version, candidate.merkle_root, computed
            );
            self.alerts.write().expect("lock poisoned").push(msg.clone());
            return Err(msg);
        }

        // Phase 2: Atomic pointer swap
        let new_arc = Arc::new(candidate);
        let version = new_arc.version;
        let mut active = self.active.write().expect("lock poisoned");
        let prev = std::mem::replace(&mut *active, new_arc);
        *self.previous.write().expect("lock poisoned") = Some(prev);
        Ok(version)
    }

    /// Two-phase apply of a delta snapshot.
    pub fn apply_delta(&self, delta: &DeltaSnapshot) -> Result<u64, String> {
        let current = self.active_snapshot();
        if current.merkle_root != delta.base_merkle_root {
            let msg = format!(
                "Delta v{} base mismatch (active {}, delta expects {})",
                delta.version, current.merkle_root, delta.base_merkle_root
            );
            self.alerts.write().expect("lock poisoned").push(msg.clone());
            return Err(msg);
        }

        // Stage mutated map off-line
        let mut staged = current.objects.clone();
        for k in &delta.removals {
            staged.remove(k);
        }
        for (k, v) in &delta.upserts {
            staged.insert(k.clone(), v.clone());
        }

        let computed = SnapshotBuilder::compute_merkle_root(&staged);
        if computed != delta.target_merkle_root {
            let msg = format!(
                "Delta v{} produced invalid Merkle root (expected {}, got {}); retaining v{}",
                delta.version, delta.target_merkle_root, computed, current.version
            );
            self.alerts.write().expect("lock poisoned").push(msg.clone());
            return Err(msg);
        }

        let serialized_bytes = serde_json::to_vec(&staged).map(|b| b.len()).unwrap_or(0);
        let next = Arc::new(ConfigSnapshotBundle {
            version: delta.version,
            merkle_root: computed,
            objects: staged,
            generated_at: Utc::now(),
            serialized_bytes,
        });

        let mut active = self.active.write().expect("lock poisoned");
        let prev = std::mem::replace(&mut *active, next);
        *self.previous.write().expect("lock poisoned") = Some(prev);
        Ok(delta.version)
    }

    /// Single-pointer-move rollback to the prior verified snapshot.
    pub fn rollback(&self) -> Result<u64, String> {
        let prev = self
            .previous
            .write()
            .expect("lock poisoned")
            .take()
            .ok_or_else(|| "No prior snapshot available for rollback".to_string())?;
        let target_version = prev.version;
        let mut active = self.active.write().expect("lock poisoned");
        let current = std::mem::replace(&mut *active, prev);
        *self.previous.write().expect("lock poisoned") = Some(current);
        Ok(target_version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn generates_10k_object_snapshot_quickly_and_saves_delta_bandwidth() {
        let mut objects = BTreeMap::new();
        for i in 0..10_000 {
            objects.insert(
                format!("peer-{i:05}"),
                serde_json::json!({ "endpoint": format!("10.0.{}.{}:11625", i / 256, i % 256), "weight": 100 }),
            );
        }

        let start = Instant::now();
        let snap1 = SnapshotBuilder::build(1, objects.clone());
        let elapsed = start.elapsed();
        assert!(elapsed.as_secs_f64() < 2.0, "10k snapshot took {elapsed:?}");
        assert_eq!(snap1.objects.len(), 10_000);

        // Mutate 10 objects out of 10k -> steady-state delta should save >= 80% bandwidth
        for i in 0..10 {
            objects.insert(
                format!("peer-{i:05}"),
                serde_json::json!({ "endpoint": format!("10.1.0.{i}:11625"), "weight": 200 }),
            );
        }
        let snap2 = SnapshotBuilder::build(2, objects);
        let delta = SnapshotBuilder::diff(&snap1, &snap2);
        assert!(delta.bandwidth_savings_percent(snap2.serialized_bytes) >= 80.0);

        let applier = TwoPhaseAgentApplier::new(snap1.clone()).unwrap();
        assert_eq!(applier.apply_delta(&delta).unwrap(), 2);
        assert_eq!(applier.active_snapshot().version, 2);

        // Corrupt snapshot must be rejected and retain v2
        let mut corrupt = snap2.clone();
        corrupt.version = 3;
        corrupt.merkle_root = "deadbeef".into();
        assert!(applier.apply_full(corrupt).is_err());
        assert_eq!(applier.active_snapshot().version, 2);
        assert_eq!(applier.alerts().len(), 1);

        // Single-pointer rollback restores v1
        assert_eq!(applier.rollback().unwrap(), 1);
        assert_eq!(applier.active_snapshot().version, 1);
    }
}
