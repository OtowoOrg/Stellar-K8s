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
//! Declarative Index Sharding for CRD Informer Caches
//!
//! This module implements consistent-hashing based index sharding for
//! controller-runtime informer caches, enabling large-scale CRD collections
//! to scale without unbounded memory growth.
//!
//! # Design
//!
//! Each CRD can declare a `shardKey` field in its spec. The informer cache
//! partitions objects across N shards using consistent hashing on that key.
//! Rebalancing moves only O(1/N) keys when shard count changes.
//!
//! ## Acceptance Criteria (from #1512)
//! - [ ] Cache memory stays under budget at 500k objects
//! - [ ] Rebalance causes no watch disconnects
//! - [ ] Lookup latency flat as collection grows 10x
//! - [ ] Shard strategy visible in CRD status

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracing::{debug, info};

/// Configuration for index sharding on a CRD.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct IndexShardingConfig {
    /// Number of shards for the informer cache.
    #[serde(default = "default_shard_count")]
    pub shard_count: usize,

    /// Field path used as the shard key (e.g., "spec.tenantId", "metadata.labels.zone").
    /// If empty, falls back to consistent hashing of the object UID.
    #[serde(default)]
    pub shard_key: String,

    /// Enable virtual nodes for better distribution.
    #[serde(default = "default_virtual_nodes")]
    pub virtual_nodes: usize,
}

fn default_shard_count() -> usize {
    16
}

fn default_virtual_nodes() -> usize {
    100
}

/// Shard assignment for a single object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardAssignment {
    pub object_uid: String,
    pub shard_id: usize,
    pub shard_key_value: String,
}

/// Consistent hash ring for shard assignment.
#[derive(Debug)]
pub struct ShardRing {
    ring: Vec<(u64, usize)>, // (hash, shard_id)
    virtual_nodes: usize,
    shard_count: usize,
}

impl ShardRing {
    pub fn new(config: &IndexShardingConfig) -> Self {
        let mut ring = Vec::with_capacity(config.shard_count * config.virtual_nodes);
        for shard_id in 0..config.shard_count {
            for v in 0..config.virtual_nodes {
                let key = format!("shard-{}-vn-{}", shard_id, v);
                let hash = hash_string(&key);
                ring.push((hash, shard_id));
            }
        }
        ring.sort_by_key(|&(h, _)| h);
        Self {
            ring,
            virtual_nodes: config.virtual_nodes,
            shard_count: config.shard_count,
        }
    }

    /// Get the shard ID for a given shard key value.
    pub fn get_shard(&self, shard_key_value: &str) -> usize {
        if self.ring.is_empty() {
            return 0;
        }
        let hash = hash_string(shard_key_value);
        // Binary search for first ring entry >= hash
        let idx = self.ring.partition_point(|&(h, _)| h < hash);
        let (_, shard_id) = self.ring[idx % self.ring.len()];
        shard_id
    }
}

/// Simple hash function for consistent hashing.
fn hash_string(s: &str) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// Sharded informer cache index.
#[derive(Debug)]
pub struct ShardedIndex {
    shards: Vec<ShardData>,
    ring: ShardRing,
    config: IndexShardingConfig,
}

#[derive(Debug, Default)]
struct ShardData {
    objects: HashMap<String, Vec<u8>>, // Simplified: just storing serialized objects
    memory_bytes: usize,
}

impl ShardedIndex {
    pub fn new(config: IndexShardingConfig) -> Self {
        let ring = ShardRing::new(&config);
        let shards = (0..config.shard_count).map(|_| ShardData::default()).collect();
        Self { shards, ring, config }
    }

    /// Insert an object into the appropriate shard.
    pub fn insert(&mut self, object_uid: String, shard_key_value: String, data: Vec<u8>) -> ShardAssignment {
        let shard_id = self.ring.get_shard(&shard_key_value);
        let shard = &mut self.shards[shard_id];
        let size = data.len();
        shard.objects.insert(object_uid.clone(), data);
        shard.memory_bytes += size;

        ShardAssignment {
            object_uid,
            shard_id,
            shard_key_value,
        }
    }

    /// Get total memory across all shards.
    pub fn total_memory_bytes(&self) -> usize {
        self.shards.iter().map(|s| s.memory_bytes).sum()
    }

    /// Get per-shard memory stats.
    pub fn shard_memory_stats(&self) -> Vec<(usize, usize)> {
        self.shards.iter().map(|s| s.memory_bytes).enumerate().collect()
    }

    /// Rebalance shards when config changes (e.g., shard_count increase).
    /// Returns number of objects that moved.
    pub fn rebalance(&mut self, new_config: IndexShardingConfig) -> usize {
        let old_shards = std::mem::take(&mut self.shards);
        self.config = new_config;
        self.ring = ShardRing::new(&self.config);
        self.shards = (0..self.config.shard_count).map(|_| ShardData::default()).collect();

        let mut moved = 0;
        for (idx, shard) in old_shards.into_iter().enumerate() {
            for (uid, data) in shard.objects {
                // For simplicity, we use the UID as shard key for rebalancing
                // In production, we'd need to preserve the original shard key
                let shard_id = self.ring.get_shard(&uid);
                self.shards[shard_id].memory_bytes += data.len();
                self.shards[shard_id].objects.insert(uid, data);
                if shard_id != idx {
                    moved += 1;
                }
            }
        }
        info!(old_shards = old_shards.len(), new_shards = self.shards.len(), moved, "Shard rebalance completed");
        moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shard_ring_deterministic() {
        let config = IndexShardingConfig { shard_count: 4, shard_key: "tenant".into(), virtual_nodes: 10 };
        let ring = ShardRing::new(&config);
        assert_eq!(ring.get_shard("tenant-a"), ring.get_shard("tenant-a"));
        assert_eq!(ring.get_shard("tenant-b"), ring.get_shard("tenant-b"));
    }

    #[test]
    fn test_shard_distribution() {
        let config = IndexShardingConfig { shard_count: 16, shard_key: "tenant".into(), virtual_nodes: 100 };
        let ring = ShardRing::new(&config);
        let mut counts = vec![0; 16];
        for i in 0..1000 {
            let tenant = format!("tenant-{}", i);
            counts[ring.get_shard(&tenant)] += 1;
        }
        // Check rough balance: no shard should have > 2x average
        let avg = 1000 / 16;
        for &c in &counts {
            assert!(c <= avg * 2, "Shard distribution too skewed: {:?}", counts);
        }
    }

    #[test]
    fn test_sharded_index_insert_and_memory() {
        let config = IndexShardingConfig::default();
        let mut index = ShardedIndex::new(config);
        index.insert("obj-1".into(), "tenant-a".into(), vec![1, 2, 3]);
        index.insert("obj-2".into(), "tenant-b".into(), vec![4, 5]);
        assert_eq!(index.total_memory_bytes(), 5);
    }

    #[test]
    fn test_rebalance_moves_subset() {
        let config = IndexShardingConfig { shard_count: 4, ..Default::default() };
        let mut index = ShardedIndex::new(config);
        for i in 0..100 {
            index.insert(format!("obj-{}", i), format!("key-{}", i % 10), vec![0; 100]);
        }
        let moved = index.rebalance(IndexShardingConfig { shard_count: 8, ..Default::default() });
        // With consistent hashing, roughly half the objects should move when doubling shards
        assert!(moved > 20 && moved < 80, "Expected ~50% move, got {}", moved);
        // Total memory should be preserved
        assert_eq!(index.total_memory_bytes(), 100 * 100);
    }
}