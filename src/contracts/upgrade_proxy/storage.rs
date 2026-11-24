//! Storage namespace isolation for the upgradable governance proxy.
//!
//! All persistent state in the upgrade proxy is keyed through typed key-prefix
//! envelopes, ensuring that logic-layer data and implementation-layer data can
//! never collide — even after WASM bytecode upgrades that add new storage keys.
//!
//! # Layout
//!
//! ```text
//! ┌────────────────────────────────────────────────────────────┐
//! │  IMPL namespace  (prefix = "impl:")                        │
//! │    impl:current_wasm    → WasmRecord (active bytecode)     │
//! │    impl:prev_wasm       → WasmRecord (rollback snapshot)   │
//! │    impl:upgrade_counter → u32                              │
//! ├────────────────────────────────────────────────────────────┤
//! │  GOV namespace   (prefix = "gov:")                         │
//! │    gov:admin_key        → String (Stellar public key)      │
//! │    gov:governance_keys  → Vec<String>                      │
//! │    gov:pending_upgrade  → Option<PendingUpgrade>           │
//! └────────────────────────────────────────────────────────────┘
//! ```
//!
//! Keys in different namespaces are guaranteed to be distinct by construction
//! because the prefix is an integral part of the serialised key string.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Prefixes ──────────────────────────────────────────────────────────────────

/// Prefix for all implementation-layer storage keys.
pub const IMPL_PREFIX: &str = "impl:";
/// Prefix for all governance-layer storage keys.
pub const GOV_PREFIX: &str = "gov:";

// ── Well-known key names (unprefixed) ─────────────────────────────────────────

const KEY_CURRENT_WASM: &str = "current_wasm";
const KEY_PREV_WASM: &str = "prev_wasm";
const KEY_UPGRADE_COUNTER: &str = "upgrade_counter";
const KEY_ADMIN_KEY: &str = "admin_key";
const KEY_GOVERNANCE_KEYS: &str = "governance_keys";
const KEY_PENDING_UPGRADE: &str = "pending_upgrade";

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Build a fully-qualified storage key by prepending the given prefix.
///
/// ```
/// # use stellar_k8s::contracts::upgrade_proxy::storage::{namespaced_key, IMPL_PREFIX};
/// let k = namespaced_key(IMPL_PREFIX, "current_wasm");
/// assert_eq!(k, "impl:current_wasm");
/// ```
pub fn namespaced_key(prefix: &str, name: &str) -> String {
    format!("{prefix}{name}")
}

/// Returns `true` when the two keys belong to different namespaces, i.e. they
/// cannot collide regardless of their suffix values.
pub fn keys_are_isolated(key_a: &str, key_b: &str) -> bool {
    fn namespace(k: &str) -> &str {
        k.split(':').next().unwrap_or("")
    }
    namespace(key_a) != namespace(key_b)
}

// ── Domain types ──────────────────────────────────────────────────────────────

/// A versioned record of a deployed WASM bytecode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WasmRecord {
    /// Hex-encoded SHA-256 hash of the WASM bytecode.
    pub wasm_hash: String,
    /// Semantic version string (e.g. "1.0.0").
    pub version: String,
    /// Unix timestamp (seconds) when this record was committed.
    pub deployed_at: u64,
    /// Stellar public key of the governance principal that authorised the deploy.
    pub deployed_by: String,
}

/// A pending upgrade awaiting execution.
///
/// Upgrades are proposed and committed in two steps so that governance keys
/// can review the WASM hash before it is activated.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingUpgrade {
    /// Target WASM hash (hex-SHA-256).
    pub target_wasm_hash: String,
    /// Target semantic version.
    pub target_version: String,
    /// Governance key that proposed the upgrade.
    pub proposed_by: String,
    /// Unix timestamp when the proposal was made.
    pub proposed_at: u64,
}

// ── Typed storage store (in-process simulation / unit-test harness) ───────────

/// An in-memory key/value store with namespace-aware accessors.
///
/// In production Soroban contracts this maps to the `Env::storage()` instance;
/// here it serves as a deterministic test double that can be inspected directly.
#[derive(Debug, Default, Clone)]
pub struct ContractStorage {
    inner: HashMap<String, Vec<u8>>,
}

impl ContractStorage {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Low-level primitives ───────────────────────────────────────────────

    /// Write a serialisable value at `key`.
    pub fn set<T: Serialize>(&mut self, key: &str, value: &T) -> Result<(), StorageError> {
        let bytes = serde_json::to_vec(value).map_err(|e| StorageError::SerializeError(e.to_string()))?;
        self.inner.insert(key.to_string(), bytes);
        Ok(())
    }

    /// Read and deserialise a value at `key`.  Returns `None` if absent.
    pub fn get<T: for<'de> Deserialize<'de>>(&self, key: &str) -> Result<Option<T>, StorageError> {
        match self.inner.get(key) {
            None => Ok(None),
            Some(bytes) => {
                let v = serde_json::from_slice(bytes)
                    .map_err(|e| StorageError::DeserializeError(e.to_string()))?;
                Ok(Some(v))
            }
        }
    }

    /// Remove a key.  Returns `true` if the key was present.
    pub fn remove(&mut self, key: &str) -> bool {
        self.inner.remove(key).is_some()
    }

    /// Return `true` if the key exists.
    pub fn has(&self, key: &str) -> bool {
        self.inner.contains_key(key)
    }

    // ── Impl-namespace accessors ───────────────────────────────────────────

    /// Read the currently active WASM record.
    pub fn get_current_wasm(&self) -> Result<Option<WasmRecord>, StorageError> {
        self.get(&namespaced_key(IMPL_PREFIX, KEY_CURRENT_WASM))
    }

    /// Persist the currently active WASM record.
    pub fn set_current_wasm(&mut self, record: &WasmRecord) -> Result<(), StorageError> {
        self.set(&namespaced_key(IMPL_PREFIX, KEY_CURRENT_WASM), record)
    }

    /// Read the rollback snapshot (previous WASM record).
    pub fn get_prev_wasm(&self) -> Result<Option<WasmRecord>, StorageError> {
        self.get(&namespaced_key(IMPL_PREFIX, KEY_PREV_WASM))
    }

    /// Persist the rollback snapshot.
    pub fn set_prev_wasm(&mut self, record: &WasmRecord) -> Result<(), StorageError> {
        self.set(&namespaced_key(IMPL_PREFIX, KEY_PREV_WASM), record)
    }

    /// Read the running upgrade counter (incremented on every successful upgrade).
    pub fn get_upgrade_counter(&self) -> Result<u32, StorageError> {
        Ok(self
            .get::<u32>(&namespaced_key(IMPL_PREFIX, KEY_UPGRADE_COUNTER))?
            .unwrap_or(0))
    }

    /// Persist the upgrade counter.
    pub fn set_upgrade_counter(&mut self, count: u32) -> Result<(), StorageError> {
        self.set(&namespaced_key(IMPL_PREFIX, KEY_UPGRADE_COUNTER), &count)
    }

    // ── Gov-namespace accessors ────────────────────────────────────────────

    /// Read the admin key (a Stellar G… public key string).
    pub fn get_admin_key(&self) -> Result<Option<String>, StorageError> {
        self.get(&namespaced_key(GOV_PREFIX, KEY_ADMIN_KEY))
    }

    /// Persist the admin key.
    pub fn set_admin_key(&mut self, key: &str) -> Result<(), StorageError> {
        self.set(&namespaced_key(GOV_PREFIX, KEY_ADMIN_KEY), &key.to_string())
    }

    /// Read the list of authorised governance keys.
    pub fn get_governance_keys(&self) -> Result<Vec<String>, StorageError> {
        Ok(self
            .get::<Vec<String>>(&namespaced_key(GOV_PREFIX, KEY_GOVERNANCE_KEYS))?
            .unwrap_or_default())
    }

    /// Persist the list of authorised governance keys.
    pub fn set_governance_keys(&mut self, keys: &[String]) -> Result<(), StorageError> {
        self.set(
            &namespaced_key(GOV_PREFIX, KEY_GOVERNANCE_KEYS),
            &keys.to_vec(),
        )
    }

    /// Read the pending upgrade proposal, if any.
    pub fn get_pending_upgrade(&self) -> Result<Option<PendingUpgrade>, StorageError> {
        self.get(&namespaced_key(GOV_PREFIX, KEY_PENDING_UPGRADE))
    }

    /// Persist a pending upgrade proposal.
    pub fn set_pending_upgrade(&mut self, proposal: &PendingUpgrade) -> Result<(), StorageError> {
        self.set(&namespaced_key(GOV_PREFIX, KEY_PENDING_UPGRADE), proposal)
    }

    /// Clear the pending upgrade proposal (called after execution or rollback).
    pub fn clear_pending_upgrade(&mut self) {
        self.remove(&namespaced_key(GOV_PREFIX, KEY_PENDING_UPGRADE));
    }
}

// ── StorageError ──────────────────────────────────────────────────────────────

/// Errors produced by storage operations.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StorageError {
    #[error("serialization error: {0}")]
    SerializeError(String),
    #[error("deserialization error: {0}")]
    DeserializeError(String),
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_wasm_record(ver: &str) -> WasmRecord {
        WasmRecord {
            wasm_hash: format!("deadbeef{ver}"),
            version: ver.to_string(),
            deployed_at: 1_700_000_000,
            deployed_by: "GADMIN1".to_string(),
        }
    }

    fn sample_pending() -> PendingUpgrade {
        PendingUpgrade {
            target_wasm_hash: "cafebabe".to_string(),
            target_version: "2.0.0".to_string(),
            proposed_by: "GPROPOSER".to_string(),
            proposed_at: 1_700_001_000,
        }
    }

    // ── namespaced_key ────────────────────────────────────────────────────

    #[test]
    fn test_namespaced_key_impl() {
        assert_eq!(
            namespaced_key(IMPL_PREFIX, "current_wasm"),
            "impl:current_wasm"
        );
    }

    #[test]
    fn test_namespaced_key_gov() {
        assert_eq!(namespaced_key(GOV_PREFIX, "admin_key"), "gov:admin_key");
    }

    // ── keys_are_isolated ─────────────────────────────────────────────────

    #[test]
    fn test_impl_and_gov_keys_are_isolated() {
        let k1 = namespaced_key(IMPL_PREFIX, "current_wasm");
        let k2 = namespaced_key(GOV_PREFIX, "admin_key");
        assert!(keys_are_isolated(&k1, &k2));
    }

    #[test]
    fn test_same_namespace_keys_are_not_isolated() {
        let k1 = namespaced_key(IMPL_PREFIX, "current_wasm");
        let k2 = namespaced_key(IMPL_PREFIX, "prev_wasm");
        assert!(!keys_are_isolated(&k1, &k2));
    }

    #[test]
    fn test_impl_current_and_gov_pending_are_isolated() {
        let k1 = namespaced_key(IMPL_PREFIX, "upgrade_counter");
        let k2 = namespaced_key(GOV_PREFIX, "pending_upgrade");
        assert!(keys_are_isolated(&k1, &k2));
    }

    // ── ContractStorage: basic primitives ─────────────────────────────────

    #[test]
    fn test_set_and_get_roundtrip() {
        let mut store = ContractStorage::new();
        store.set("foo", &42u32).unwrap();
        let v: u32 = store.get("foo").unwrap().unwrap();
        assert_eq!(v, 42);
    }

    #[test]
    fn test_get_absent_key_returns_none() {
        let store = ContractStorage::new();
        let v: Option<u32> = store.get("missing").unwrap();
        assert!(v.is_none());
    }

    #[test]
    fn test_remove_returns_true_when_present() {
        let mut store = ContractStorage::new();
        store.set("x", &"hello").unwrap();
        assert!(store.remove("x"));
        assert!(!store.has("x"));
    }

    #[test]
    fn test_remove_returns_false_when_absent() {
        let mut store = ContractStorage::new();
        assert!(!store.remove("ghost"));
    }

    // ── Impl-namespace accessors ──────────────────────────────────────────

    #[test]
    fn test_current_wasm_roundtrip() {
        let mut store = ContractStorage::new();
        let rec = sample_wasm_record("1.0.0");
        store.set_current_wasm(&rec).unwrap();
        assert_eq!(store.get_current_wasm().unwrap(), Some(rec));
    }

    #[test]
    fn test_prev_wasm_roundtrip() {
        let mut store = ContractStorage::new();
        let rec = sample_wasm_record("0.9.0");
        store.set_prev_wasm(&rec).unwrap();
        assert_eq!(store.get_prev_wasm().unwrap(), Some(rec));
    }

    #[test]
    fn test_upgrade_counter_default_is_zero() {
        let store = ContractStorage::new();
        assert_eq!(store.get_upgrade_counter().unwrap(), 0);
    }

    #[test]
    fn test_upgrade_counter_increments() {
        let mut store = ContractStorage::new();
        store.set_upgrade_counter(3).unwrap();
        assert_eq!(store.get_upgrade_counter().unwrap(), 3);
    }

    // ── Gov-namespace accessors ───────────────────────────────────────────

    #[test]
    fn test_admin_key_roundtrip() {
        let mut store = ContractStorage::new();
        store.set_admin_key("GADMIN1").unwrap();
        assert_eq!(
            store.get_admin_key().unwrap(),
            Some("GADMIN1".to_string())
        );
    }

    #[test]
    fn test_governance_keys_default_empty() {
        let store = ContractStorage::new();
        assert!(store.get_governance_keys().unwrap().is_empty());
    }

    #[test]
    fn test_governance_keys_roundtrip() {
        let mut store = ContractStorage::new();
        let keys = vec!["GK1".to_string(), "GK2".to_string()];
        store.set_governance_keys(&keys).unwrap();
        assert_eq!(store.get_governance_keys().unwrap(), keys);
    }

    #[test]
    fn test_pending_upgrade_roundtrip() {
        let mut store = ContractStorage::new();
        let p = sample_pending();
        store.set_pending_upgrade(&p).unwrap();
        assert_eq!(store.get_pending_upgrade().unwrap(), Some(p));
    }

    #[test]
    fn test_clear_pending_upgrade() {
        let mut store = ContractStorage::new();
        store.set_pending_upgrade(&sample_pending()).unwrap();
        store.clear_pending_upgrade();
        assert!(store.get_pending_upgrade().unwrap().is_none());
    }

    // ── Namespace collision test ───────────────────────────────────────────

    #[test]
    fn test_impl_and_gov_same_suffix_do_not_collide() {
        // If both namespaces used the bare key "counter", they'd collide.
        // With prefixes they must not.
        let mut store = ContractStorage::new();
        store
            .set(&namespaced_key(IMPL_PREFIX, "counter"), &100u32)
            .unwrap();
        store
            .set(&namespaced_key(GOV_PREFIX, "counter"), &999u32)
            .unwrap();

        let impl_val: u32 = store
            .get(&namespaced_key(IMPL_PREFIX, "counter"))
            .unwrap()
            .unwrap();
        let gov_val: u32 = store
            .get(&namespaced_key(GOV_PREFIX, "counter"))
            .unwrap()
            .unwrap();

        assert_eq!(impl_val, 100);
        assert_eq!(gov_val, 999);
    }
}
