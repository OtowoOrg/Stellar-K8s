//! Thread-safe in-memory store for Stellar custom metrics
//!
//! The `StellarMetricsStore` is the central cache that bridges the operator's
//! reconciliation loop (which scrapes live Horizon/Core metrics) with the
//! custom metrics API handlers (which serve those values to the Kubernetes HPA).
//!
//! # Design
//!
//! - Arc<RwLock<...>> for cheap cloning and concurrent async access.
//! - Each entry is keyed by (namespace, name) and carries a timestamp so the
//!   API handler can return a zero-fallback for stale data, preventing the HPA
//!   from making scaling decisions on data that is too old.
//! - The TTL is configurable but defaults to 120 seconds.
//!
//! # Example
//!
//! ```rust
//! use std::sync::Arc;
//! use stellar_k8s::rest_api::metrics_store::{StellarMetricsStore, StellarMetricsSnapshot};
//!
//! let store = Arc::new(StellarMetricsStore::new());
//!
//! // Writer (reconciler / collector)
//! store.upsert("default", "horizon-0", StellarMetricsSnapshot {
//!     tps: 120,
//!     queue_length: 300,
//!     ingestion_lag: 2,
//!     ledger_sequence: 49_500_000,
//!     active_connections: 15,
//!     updated_at: chrono::Utc::now(),
//! });
//!
//! // Reader (custom metrics handler)
//! let snap = store.get("default", "horizon-0");
//! assert!(snap.is_some());
//! ```

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use chrono::{DateTime, Utc};
use tracing::debug;

/// Maximum age of a metric snapshot before it is treated as stale.
/// When stale, the API returns zero so the HPA does not scale on outdated data.
pub const METRICS_STALENESS_SECS: i64 = 120;

/// Composite key identifying a Horizon/StellarNode instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MetricsKey {
    pub namespace: String,
    pub name: String,
}

impl MetricsKey {
    pub fn new(namespace: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
        }
    }
}

/// A point-in-time snapshot of Stellar metrics for a single Horizon instance.
#[derive(Debug, Clone)]
pub struct StellarMetricsSnapshot {
    /// Transactions per second ingested by Horizon.
    pub tps: i64,
    /// Number of transactions currently waiting in Horizon's submission queue.
    pub queue_length: i64,
    /// Lag (in ledgers) between the network tip and this node's latest ingested ledger.
    pub ingestion_lag: i64,
    /// Current ledger sequence number reported by this node.
    pub ledger_sequence: u64,
    /// Number of active peer connections.
    pub active_connections: i64,
    /// Wall-clock time when this snapshot was last written.
    pub updated_at: DateTime<Utc>,
}

impl Default for StellarMetricsSnapshot {
    fn default() -> Self {
        Self {
            tps: 0,
            queue_length: 0,
            ingestion_lag: 0,
            ledger_sequence: 0,
            active_connections: 0,
            updated_at: Utc::now(),
        }
    }
}

impl StellarMetricsSnapshot {
    /// Returns `true` if this snapshot was written within [`METRICS_STALENESS_SECS`].
    pub fn is_fresh(&self) -> bool {
        let age = Utc::now().signed_duration_since(self.updated_at);
        age.num_seconds() <= METRICS_STALENESS_SECS
    }
}

/// Thread-safe store for Stellar custom metrics, keyed by (namespace, name).
///
/// Shared between the Axum REST server (reader) and the background metrics
/// collector task (writer) via `Arc<StellarMetricsStore>`.
#[derive(Debug, Default)]
pub struct StellarMetricsStore {
    inner: RwLock<HashMap<MetricsKey, StellarMetricsSnapshot>>,
}

impl StellarMetricsStore {
    /// Create a new, empty metrics store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace the snapshot for `(namespace, name)`.
    pub fn upsert(&self, namespace: &str, name: &str, snapshot: StellarMetricsSnapshot) {
        let key = MetricsKey::new(namespace, name);
        debug!(
            namespace = %namespace,
            name = %name,
            tps = snapshot.tps,
            queue_length = snapshot.queue_length,
            "Upserting Stellar metrics snapshot"
        );
        if let Ok(mut guard) = self.inner.write() {
            guard.insert(key, snapshot);
        }
    }

    /// Retrieve the snapshot for `(namespace, name)`, returning `None` if absent.
    ///
    /// The caller is responsible for freshness checking via [`StellarMetricsSnapshot::is_fresh`].
    pub fn get(&self, namespace: &str, name: &str) -> Option<StellarMetricsSnapshot> {
        let key = MetricsKey::new(namespace, name);
        self.inner.read().ok()?.get(&key).cloned()
    }

    /// Retrieve the TPS value for `(namespace, name)`.
    ///
    /// Returns `0` if the snapshot is absent or stale.
    pub fn tps(&self, namespace: &str, name: &str) -> i64 {
        self.get(namespace, name)
            .filter(|s| s.is_fresh())
            .map(|s| s.tps)
            .unwrap_or(0)
    }

    /// Retrieve the queue length for `(namespace, name)`.
    ///
    /// Returns `0` if the snapshot is absent or stale.
    pub fn queue_length(&self, namespace: &str, name: &str) -> i64 {
        self.get(namespace, name)
            .filter(|s| s.is_fresh())
            .map(|s| s.queue_length)
            .unwrap_or(0)
    }

    /// Retrieve the ingestion lag for `(namespace, name)`.
    ///
    /// Returns `0` if the snapshot is absent or stale.
    pub fn ingestion_lag(&self, namespace: &str, name: &str) -> i64 {
        self.get(namespace, name)
            .filter(|s| s.is_fresh())
            .map(|s| s.ingestion_lag)
            .unwrap_or(0)
    }

    /// Retrieve the ledger sequence for `(namespace, name)`.
    ///
    /// Returns `0` if the snapshot is absent or stale.
    pub fn ledger_sequence(&self, namespace: &str, name: &str) -> i64 {
        self.get(namespace, name)
            .filter(|s| s.is_fresh())
            .map(|s| s.ledger_sequence as i64)
            .unwrap_or(0)
    }

    /// Retrieve the active connections for `(namespace, name)`.
    ///
    /// Returns `0` if the snapshot is absent or stale.
    pub fn active_connections(&self, namespace: &str, name: &str) -> i64 {
        self.get(namespace, name)
            .filter(|s| s.is_fresh())
            .map(|s| s.active_connections)
            .unwrap_or(0)
    }

    /// Remove all snapshots older than [`METRICS_STALENESS_SECS`].
    ///
    /// Called periodically by the collector to prevent unbounded memory growth.
    pub fn evict_stale(&self) {
        if let Ok(mut guard) = self.inner.write() {
            guard.retain(|_, v| v.is_fresh());
        }
    }

    /// Return all keys currently held in the store (including potentially stale ones).
    pub fn keys(&self) -> Vec<MetricsKey> {
        self.inner
            .read()
            .ok()
            .map(|g| g.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Returns the number of entries currently in the store.
    pub fn len(&self) -> usize {
        self.inner.read().ok().map(|g| g.len()).unwrap_or(0)
    }

    /// Returns `true` if the store contains no entries.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn make_snapshot(tps: i64, queue: i64) -> StellarMetricsSnapshot {
        StellarMetricsSnapshot {
            tps,
            queue_length: queue,
            ingestion_lag: 1,
            ledger_sequence: 50_000_000,
            active_connections: 8,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_upsert_and_get() {
        let store = StellarMetricsStore::new();
        store.upsert("default", "horizon-0", make_snapshot(100, 250));
        let snap = store.get("default", "horizon-0").unwrap();
        assert_eq!(snap.tps, 100);
        assert_eq!(snap.queue_length, 250);
    }

    #[test]
    fn test_get_missing_returns_none() {
        let store = StellarMetricsStore::new();
        assert!(store.get("default", "nonexistent").is_none());
    }

    #[test]
    fn test_tps_returns_zero_when_stale() {
        let store = StellarMetricsStore::new();
        let stale = StellarMetricsSnapshot {
            tps: 99,
            queue_length: 500,
            updated_at: Utc::now() - Duration::seconds(METRICS_STALENESS_SECS + 10),
            ..Default::default()
        };
        store.upsert("default", "horizon-0", stale);
        assert_eq!(store.tps("default", "horizon-0"), 0);
    }

    #[test]
    fn test_queue_length_returns_zero_when_stale() {
        let store = StellarMetricsStore::new();
        let stale = StellarMetricsSnapshot {
            tps: 50,
            queue_length: 999,
            updated_at: Utc::now() - Duration::seconds(METRICS_STALENESS_SECS + 10),
            ..Default::default()
        };
        store.upsert("default", "horizon-0", stale);
        assert_eq!(store.queue_length("default", "horizon-0"), 0);
    }

    #[test]
    fn test_upsert_overwrites_existing() {
        let store = StellarMetricsStore::new();
        store.upsert("default", "horizon-0", make_snapshot(50, 100));
        store.upsert("default", "horizon-0", make_snapshot(150, 300));
        assert_eq!(store.tps("default", "horizon-0"), 150);
        assert_eq!(store.queue_length("default", "horizon-0"), 300);
    }

    #[test]
    fn test_namespace_isolation() {
        let store = StellarMetricsStore::new();
        store.upsert("ns-a", "horizon-0", make_snapshot(100, 200));
        store.upsert("ns-b", "horizon-0", make_snapshot(50, 80));
        assert_eq!(store.tps("ns-a", "horizon-0"), 100);
        assert_eq!(store.tps("ns-b", "horizon-0"), 50);
    }

    #[test]
    fn test_evict_stale_removes_old_entries() {
        let store = StellarMetricsStore::new();
        let stale = StellarMetricsSnapshot {
            updated_at: Utc::now() - Duration::seconds(METRICS_STALENESS_SECS + 30),
            ..Default::default()
        };
        store.upsert("default", "old-horizon", stale);
        store.upsert("default", "fresh-horizon", make_snapshot(10, 20));
        assert_eq!(store.len(), 2);
        store.evict_stale();
        assert_eq!(store.len(), 1);
        assert!(store.get("default", "fresh-horizon").is_some());
        assert!(store.get("default", "old-horizon").is_none());
    }

    #[test]
    fn test_is_fresh_true_for_new_snapshot() {
        let snap = make_snapshot(10, 20);
        assert!(snap.is_fresh());
    }

    #[test]
    fn test_is_fresh_false_for_old_snapshot() {
        let snap = StellarMetricsSnapshot {
            updated_at: Utc::now() - Duration::seconds(METRICS_STALENESS_SECS + 1),
            ..Default::default()
        };
        assert!(!snap.is_fresh());
    }

    #[test]
    fn test_len_and_is_empty() {
        let store = StellarMetricsStore::new();
        assert!(store.is_empty());
        store.upsert("default", "h0", make_snapshot(1, 2));
        assert_eq!(store.len(), 1);
        assert!(!store.is_empty());
    }
}
