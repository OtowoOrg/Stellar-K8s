//! Upgradable Governance Proxy with Storage Layout Protection
//!
//! Implements a Soroban-style proxy contract that manages safe WASM bytecode
//! upgrades with:
//!
//! - **Strict key-prefix isolation** — logic-layer and implementation-layer
//!   storage keys cannot collide across versions (see [`storage`]).
//! - **Authorization gate** — only governance keys registered at init time
//!   may propose or execute upgrades.
//! - **Two-step commit** — upgrades are first *proposed* (storing a
//!   [`PendingUpgrade`]) and then *executed*, giving governance participants
//!   time to inspect the target WASM hash.
//! - **Emergency rollback** — a single call reverts the active WASM record to
//!   the previous snapshot and decrements the upgrade counter.
//!
//! # Upgrade lifecycle
//!
//! ```text
//!  propose_upgrade(wasm_hash, version, caller)
//!         │
//!         ▼
//!  [gov:pending_upgrade] is written
//!         │
//!  execute_upgrade(caller)
//!         │
//!         ▼  ┌─ snapshots current → impl:prev_wasm
//!            ├─ writes new → impl:current_wasm
//!            ├─ increments impl:upgrade_counter
//!            └─ clears gov:pending_upgrade
//!         │
//!  rollback_upgrade(caller)   ← only if something goes wrong post-upgrade
//!         │
//!         ▼  ┌─ restores impl:prev_wasm → impl:current_wasm
//!            └─ decrements impl:upgrade_counter
//! ```
//!
//! # State persistence across versions
//!
//! Because all keys are prefixed (`impl:` or `gov:`), a v2 contract that adds
//! new keys like `impl:feature_flags` will never overwrite `gov:admin_key` from
//! v1.  The unit tests in this module verify this invariant explicitly.

use serde::{Deserialize, Serialize};

use crate::contracts::upgrade_proxy::storage::{
    ContractStorage, PendingUpgrade, StorageError, WasmRecord,
};

// ── Error type ────────────────────────────────────────────────────────────────

/// Errors produced by the governance proxy contract.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProxyError {
    /// The caller is not in the list of authorised governance keys.
    #[error("unauthorized: caller '{0}' is not an approved governance key")]
    Unauthorized(String),

    /// An upgrade was proposed with a WASM hash that is already active.
    #[error("invalid upgrade: target version '{0}' is already deployed")]
    AlreadyDeployed(String),

    /// `execute_upgrade` was called but no proposal is pending.
    #[error("no pending upgrade to execute")]
    NoPendingUpgrade,

    /// `rollback_upgrade` was called but there is no previous WASM snapshot.
    #[error("no previous WASM snapshot available for rollback")]
    NoRollbackTarget,

    /// The contract has not been initialised yet.
    #[error("contract not initialised")]
    NotInitialised,

    /// Underlying storage failure.
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
}

// ── Contract state ────────────────────────────────────────────────────────────

/// The governance proxy contract.
///
/// Holds a [`ContractStorage`] instance (simulating `Env::storage()` in
/// Soroban) and exposes the upgrade lifecycle methods.
#[derive(Debug, Default)]
pub struct UpgradeProxy {
    pub(crate) storage: ContractStorage,
}

impl UpgradeProxy {
    /// Create a new, uninitialised proxy instance.
    pub fn new() -> Self {
        Self::default()
    }

    // ── Initialisation ────────────────────────────────────────────────────

    /// Initialise the proxy with an admin key, a set of governance keys, and
    /// the genesis WASM record.
    ///
    /// May only be called once; subsequent calls return
    /// [`ProxyError::AlreadyDeployed`].
    pub fn initialize(
        &mut self,
        admin_key: &str,
        governance_keys: &[String],
        initial_wasm: WasmRecord,
    ) -> Result<(), ProxyError> {
        if self.storage.get_current_wasm()?.is_some() {
            return Err(ProxyError::AlreadyDeployed(
                initial_wasm.version.clone(),
            ));
        }

        self.storage.set_admin_key(admin_key)?;
        self.storage.set_governance_keys(governance_keys)?;
        self.storage.set_current_wasm(&initial_wasm)?;
        self.storage.set_upgrade_counter(0)?;

        Ok(())
    }

    // ── Queries ───────────────────────────────────────────────────────────

    /// Return the currently active WASM record.
    pub fn current_wasm(&self) -> Result<WasmRecord, ProxyError> {
        self.storage
            .get_current_wasm()?
            .ok_or(ProxyError::NotInitialised)
    }

    /// Return the previous WASM record (rollback snapshot), if any.
    pub fn prev_wasm(&self) -> Result<Option<WasmRecord>, ProxyError> {
        Ok(self.storage.get_prev_wasm()?)
    }

    /// Return the current upgrade counter.
    pub fn upgrade_count(&self) -> Result<u32, ProxyError> {
        Ok(self.storage.get_upgrade_counter()?)
    }

    /// Return the pending upgrade proposal, if any.
    pub fn pending_upgrade(&self) -> Result<Option<PendingUpgrade>, ProxyError> {
        Ok(self.storage.get_pending_upgrade()?)
    }

    /// Return all authorised governance keys.
    pub fn governance_keys(&self) -> Result<Vec<String>, ProxyError> {
        Ok(self.storage.get_governance_keys()?)
    }

    // ── Upgrade lifecycle ─────────────────────────────────────────────────

    /// Propose a new WASM upgrade.
    ///
    /// The proposal is stored under `gov:pending_upgrade`; it is not activated
    /// until [`execute_upgrade`] is called.
    ///
    /// # Errors
    ///
    /// - [`ProxyError::Unauthorized`] — `caller` is not a governance key.
    /// - [`ProxyError::AlreadyDeployed`] — `target_version` matches the active version.
    pub fn propose_upgrade(
        &mut self,
        target_wasm_hash: &str,
        target_version: &str,
        caller: &str,
        now: u64,
    ) -> Result<(), ProxyError> {
        self.require_governance_key(caller)?;

        // Guard: must be initialised
        let current = self.current_wasm()?;

        // Guard: do not propose the already-active version
        if current.version == target_version {
            return Err(ProxyError::AlreadyDeployed(target_version.to_string()));
        }

        let proposal = PendingUpgrade {
            target_wasm_hash: target_wasm_hash.to_string(),
            target_version: target_version.to_string(),
            proposed_by: caller.to_string(),
            proposed_at: now,
        };

        self.storage.set_pending_upgrade(&proposal)?;
        Ok(())
    }

    /// Execute the pending upgrade.
    ///
    /// Snapshots the current WASM record into `impl:prev_wasm`, writes the
    /// new record into `impl:current_wasm`, increments `impl:upgrade_counter`,
    /// and clears `gov:pending_upgrade`.
    ///
    /// # Errors
    ///
    /// - [`ProxyError::Unauthorized`] — `caller` is not a governance key.
    /// - [`ProxyError::NoPendingUpgrade`] — no proposal is stored.
    pub fn execute_upgrade(
        &mut self,
        caller: &str,
        now: u64,
    ) -> Result<WasmRecord, ProxyError> {
        self.require_governance_key(caller)?;

        let proposal = self
            .storage
            .get_pending_upgrade()?
            .ok_or(ProxyError::NoPendingUpgrade)?;

        // Snapshot current → prev
        let current = self.current_wasm()?;
        self.storage.set_prev_wasm(&current)?;

        // Activate new WASM record
        let new_record = WasmRecord {
            wasm_hash: proposal.target_wasm_hash,
            version: proposal.target_version,
            deployed_at: now,
            deployed_by: caller.to_string(),
        };
        self.storage.set_current_wasm(&new_record)?;

        // Increment counter
        let counter = self.storage.get_upgrade_counter()?;
        self.storage.set_upgrade_counter(counter + 1)?;

        // Clear proposal
        self.storage.clear_pending_upgrade();

        Ok(new_record)
    }

    /// Roll back to the previous WASM snapshot.
    ///
    /// Restores `impl:prev_wasm` → `impl:current_wasm` and decrements the
    /// upgrade counter.  Clears the previous-wasm slot afterwards.
    ///
    /// # Errors
    ///
    /// - [`ProxyError::Unauthorized`] — `caller` is not a governance key.
    /// - [`ProxyError::NoRollbackTarget`] — there is no previous snapshot.
    pub fn rollback_upgrade(&mut self, caller: &str) -> Result<WasmRecord, ProxyError> {
        self.require_governance_key(caller)?;

        let prev = self
            .storage
            .get_prev_wasm()?
            .ok_or(ProxyError::NoRollbackTarget)?;

        self.storage.set_current_wasm(&prev)?;
        self.storage.remove(&crate::contracts::upgrade_proxy::storage::namespaced_key(
            crate::contracts::upgrade_proxy::storage::IMPL_PREFIX,
            "prev_wasm",
        ));

        // Decrement counter (saturating at 0)
        let counter = self.storage.get_upgrade_counter()?;
        self.storage.set_upgrade_counter(counter.saturating_sub(1))?;

        Ok(prev)
    }

    /// Add a new governance key.  Only the admin may do this.
    pub fn add_governance_key(
        &mut self,
        new_key: &str,
        caller: &str,
    ) -> Result<(), ProxyError> {
        let admin = self.storage.get_admin_key()?.ok_or(ProxyError::NotInitialised)?;
        if caller != admin {
            return Err(ProxyError::Unauthorized(caller.to_string()));
        }
        let mut keys = self.storage.get_governance_keys()?;
        if !keys.contains(&new_key.to_string()) {
            keys.push(new_key.to_string());
            self.storage.set_governance_keys(&keys)?;
        }
        Ok(())
    }

    // ── Internal helpers ──────────────────────────────────────────────────

    fn require_governance_key(&self, caller: &str) -> Result<(), ProxyError> {
        let keys = self.storage.get_governance_keys()?;
        if keys.iter().any(|k| k == caller) {
            Ok(())
        } else {
            Err(ProxyError::Unauthorized(caller.to_string()))
        }
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const ADMIN: &str = "GADMIN_KEY_1";
    const GOV1: &str = "GGOV_KEY_1";
    const GOV2: &str = "GGOV_KEY_2";
    const STRANGER: &str = "GSTRANGER_KEY";

    fn v1_wasm() -> WasmRecord {
        WasmRecord {
            wasm_hash: "aabbccdd11111111".to_string(),
            version: "1.0.0".to_string(),
            deployed_at: 1_000_000,
            deployed_by: ADMIN.to_string(),
        }
    }

    fn initialised_proxy() -> UpgradeProxy {
        let mut proxy = UpgradeProxy::new();
        proxy
            .initialize(
                ADMIN,
                &[GOV1.to_string(), GOV2.to_string()],
                v1_wasm(),
            )
            .unwrap();
        proxy
    }

    // ── Initialisation ────────────────────────────────────────────────────

    #[test]
    fn test_initialize_sets_current_wasm() {
        let proxy = initialised_proxy();
        assert_eq!(proxy.current_wasm().unwrap().version, "1.0.0");
    }

    #[test]
    fn test_initialize_sets_governance_keys() {
        let proxy = initialised_proxy();
        let keys = proxy.governance_keys().unwrap();
        assert!(keys.contains(&GOV1.to_string()));
        assert!(keys.contains(&GOV2.to_string()));
    }

    #[test]
    fn test_initialize_counter_is_zero() {
        let proxy = initialised_proxy();
        assert_eq!(proxy.upgrade_count().unwrap(), 0);
    }

    #[test]
    fn test_double_initialize_fails() {
        let mut proxy = initialised_proxy();
        let result = proxy.initialize(ADMIN, &[], v1_wasm());
        assert!(matches!(result, Err(ProxyError::AlreadyDeployed(_))));
    }

    // ── propose_upgrade ───────────────────────────────────────────────────

    #[test]
    fn test_propose_upgrade_succeeds_for_governance_key() {
        let mut proxy = initialised_proxy();
        proxy
            .propose_upgrade("newwasm1234", "2.0.0", GOV1, 2_000_000)
            .unwrap();
        let pending = proxy.pending_upgrade().unwrap().unwrap();
        assert_eq!(pending.target_version, "2.0.0");
        assert_eq!(pending.proposed_by, GOV1);
    }

    #[test]
    fn test_propose_upgrade_rejected_for_stranger() {
        let mut proxy = initialised_proxy();
        let err = proxy
            .propose_upgrade("newwasm", "2.0.0", STRANGER, 2_000_000)
            .unwrap_err();
        assert!(matches!(err, ProxyError::Unauthorized(_)));
    }

    #[test]
    fn test_propose_already_active_version_fails() {
        let mut proxy = initialised_proxy();
        let err = proxy
            .propose_upgrade("samewasm", "1.0.0", GOV1, 2_000_000)
            .unwrap_err();
        assert!(matches!(err, ProxyError::AlreadyDeployed(_)));
    }

    // ── execute_upgrade ───────────────────────────────────────────────────

    #[test]
    fn test_execute_upgrade_activates_new_wasm() {
        let mut proxy = initialised_proxy();
        proxy
            .propose_upgrade("v2wasm", "2.0.0", GOV1, 2_000_000)
            .unwrap();
        let new_record = proxy.execute_upgrade(GOV2, 2_100_000).unwrap();
        assert_eq!(new_record.version, "2.0.0");
        assert_eq!(proxy.current_wasm().unwrap().version, "2.0.0");
    }

    #[test]
    fn test_execute_upgrade_snapshots_prev_wasm() {
        let mut proxy = initialised_proxy();
        proxy
            .propose_upgrade("v2wasm", "2.0.0", GOV1, 2_000_000)
            .unwrap();
        proxy.execute_upgrade(GOV2, 2_100_000).unwrap();
        let prev = proxy.prev_wasm().unwrap().unwrap();
        assert_eq!(prev.version, "1.0.0");
    }

    #[test]
    fn test_execute_upgrade_increments_counter() {
        let mut proxy = initialised_proxy();
        proxy
            .propose_upgrade("v2wasm", "2.0.0", GOV1, 2_000_000)
            .unwrap();
        proxy.execute_upgrade(GOV2, 2_100_000).unwrap();
        assert_eq!(proxy.upgrade_count().unwrap(), 1);
    }

    #[test]
    fn test_execute_upgrade_clears_pending() {
        let mut proxy = initialised_proxy();
        proxy
            .propose_upgrade("v2wasm", "2.0.0", GOV1, 2_000_000)
            .unwrap();
        proxy.execute_upgrade(GOV2, 2_100_000).unwrap();
        assert!(proxy.pending_upgrade().unwrap().is_none());
    }

    #[test]
    fn test_execute_without_pending_fails() {
        let mut proxy = initialised_proxy();
        let err = proxy.execute_upgrade(GOV1, 2_100_000).unwrap_err();
        assert!(matches!(err, ProxyError::NoPendingUpgrade));
    }

    #[test]
    fn test_execute_upgrade_rejected_for_stranger() {
        let mut proxy = initialised_proxy();
        proxy
            .propose_upgrade("v2wasm", "2.0.0", GOV1, 2_000_000)
            .unwrap();
        let err = proxy.execute_upgrade(STRANGER, 2_100_000).unwrap_err();
        assert!(matches!(err, ProxyError::Unauthorized(_)));
    }

    // ── rollback_upgrade ──────────────────────────────────────────────────

    #[test]
    fn test_rollback_restores_previous_version() {
        let mut proxy = initialised_proxy();
        proxy
            .propose_upgrade("v2wasm", "2.0.0", GOV1, 2_000_000)
            .unwrap();
        proxy.execute_upgrade(GOV2, 2_100_000).unwrap();
        proxy.rollback_upgrade(GOV1).unwrap();
        assert_eq!(proxy.current_wasm().unwrap().version, "1.0.0");
    }

    #[test]
    fn test_rollback_decrements_counter() {
        let mut proxy = initialised_proxy();
        proxy
            .propose_upgrade("v2wasm", "2.0.0", GOV1, 2_000_000)
            .unwrap();
        proxy.execute_upgrade(GOV2, 2_100_000).unwrap();
        assert_eq!(proxy.upgrade_count().unwrap(), 1);
        proxy.rollback_upgrade(GOV1).unwrap();
        assert_eq!(proxy.upgrade_count().unwrap(), 0);
    }

    #[test]
    fn test_rollback_without_prev_fails() {
        let mut proxy = initialised_proxy();
        let err = proxy.rollback_upgrade(GOV1).unwrap_err();
        assert!(matches!(err, ProxyError::NoRollbackTarget));
    }

    #[test]
    fn test_rollback_rejected_for_stranger() {
        let mut proxy = initialised_proxy();
        proxy
            .propose_upgrade("v2wasm", "2.0.0", GOV1, 2_000_000)
            .unwrap();
        proxy.execute_upgrade(GOV2, 2_100_000).unwrap();
        let err = proxy.rollback_upgrade(STRANGER).unwrap_err();
        assert!(matches!(err, ProxyError::Unauthorized(_)));
    }

    // ── State persistence across upgrade versions ─────────────────────────

    #[test]
    fn test_state_persists_across_upgrade_v1_to_v2() {
        // Simulates a v1→v2 upgrade: governance keys set in v1 must still
        // be readable after the WASM record is swapped to v2.
        let mut proxy = initialised_proxy();

        // v1 state: governance keys are set
        let v1_keys = proxy.governance_keys().unwrap();
        assert_eq!(v1_keys.len(), 2);

        // Perform upgrade to v2
        proxy
            .propose_upgrade("v2wasm", "2.0.0", GOV1, 2_000_000)
            .unwrap();
        proxy.execute_upgrade(GOV2, 2_100_000).unwrap();

        // Governance keys (gov: prefix) must be unchanged post-upgrade
        let v2_keys = proxy.governance_keys().unwrap();
        assert_eq!(v2_keys, v1_keys, "governance keys must survive upgrade");

        // Admin key must also be unchanged
        let admin = proxy.storage.get_admin_key().unwrap().unwrap();
        assert_eq!(admin, ADMIN);
    }

    #[test]
    fn test_layout_collision_impossible() {
        // Proves that impl: and gov: keys with the same suffix are distinct.
        let mut proxy = initialised_proxy();
        // Force-write a value to impl:admin_key (if such a key existed) — it must not
        // be visible through gov:admin_key.
        proxy.storage.set("impl:admin_key", &"IMPOSTER".to_string()).unwrap();
        let gov_admin = proxy.storage.get_admin_key().unwrap().unwrap();
        assert_eq!(gov_admin, ADMIN, "impl:admin_key must not shadow gov:admin_key");
    }

    // ── add_governance_key ────────────────────────────────────────────────

    #[test]
    fn test_add_governance_key_by_admin() {
        let mut proxy = initialised_proxy();
        proxy.add_governance_key("GNEW_KEY", ADMIN).unwrap();
        assert!(proxy
            .governance_keys()
            .unwrap()
            .contains(&"GNEW_KEY".to_string()));
    }

    #[test]
    fn test_add_governance_key_rejected_for_non_admin() {
        let mut proxy = initialised_proxy();
        let err = proxy
            .add_governance_key("GNEW_KEY", GOV1)
            .unwrap_err();
        assert!(matches!(err, ProxyError::Unauthorized(_)));
    }

    // ── Two full upgrade cycles ───────────────────────────────────────────

    #[test]
    fn test_two_sequential_upgrades() {
        let mut proxy = initialised_proxy();

        // v1 → v2
        proxy.propose_upgrade("v2", "2.0.0", GOV1, 1_000).unwrap();
        proxy.execute_upgrade(GOV2, 1_001).unwrap();
        assert_eq!(proxy.upgrade_count().unwrap(), 1);
        assert_eq!(proxy.current_wasm().unwrap().version, "2.0.0");

        // v2 → v3
        proxy.propose_upgrade("v3", "3.0.0", GOV2, 2_000).unwrap();
        proxy.execute_upgrade(GOV1, 2_001).unwrap();
        assert_eq!(proxy.upgrade_count().unwrap(), 2);
        assert_eq!(proxy.current_wasm().unwrap().version, "3.0.0");

        // Rollback: v3 → v2
        proxy.rollback_upgrade(GOV1).unwrap();
        assert_eq!(proxy.upgrade_count().unwrap(), 1);
        assert_eq!(proxy.current_wasm().unwrap().version, "2.0.0");
    }
}
