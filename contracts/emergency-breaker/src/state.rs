//! # Emergency Circuit Breaker — State & Storage
//!
//! Defines all persistent state types and the single-lookup pause check used by
//! [`crate::lib`].
//!
//! ## Storage layout
//!
//! All keys are stored in a flat `HashMap<StorageKey, StorageValue>` (simulated
//! here with Rust `std::collections::HashMap` — in a real Soroban host this
//! maps 1-to-1 with `env.storage().instance()`).
//!
//! | Key                       | Value                      | Purpose                                  |
//! |---------------------------|----------------------------|------------------------------------------|
//! | `StorageKey::Threshold`   | `u8`                       | Required signature count M               |
//! | `StorageKey::Operators`   | `Vec<[u8;32]>` (pub keys)  | The N authorised operator public keys    |
//! | `StorageKey::FreezeScope` | `FreezeScope` bitmask      | Which operations are frozen              |
//! | `StorageKey::UnpauseAt`   | `u64` (Unix seconds)       | Earliest timestamp for unpause           |
//! | `StorageKey::PausedBy`    | `Vec<[u8;32]>` (pub keys)  | Signers of the current freeze proposal   |

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Freeze scope bitmask
// ---------------------------------------------------------------------------

/// Granular operation classes that can be frozen independently.
///
/// Stored as a `u8` bitmask so the pause check is a single bit-AND — O(1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FreezeScope(pub u8);

impl FreezeScope {
    /// No operations are frozen.
    pub const NONE: Self = FreezeScope(0b0000_0000);
    /// Deposits are frozen.
    pub const DEPOSITS: Self = FreezeScope(0b0000_0001);
    /// Withdrawals are frozen.
    pub const WITHDRAWALS: Self = FreezeScope(0b0000_0010);
    /// Governance (voting, proposals) is frozen.
    pub const GOVERNANCE: Self = FreezeScope(0b0000_0100);
    /// All state-changing operations are frozen.
    pub const ALL: Self = FreezeScope(0b1111_1111);

    /// Returns `true` if all bits of `other` are set in `self`.
    #[inline]
    pub fn contains(self, other: FreezeScope) -> bool {
        self.0 & other.0 == other.0
    }

    /// Merge another scope into this one (bitwise OR).
    #[inline]
    pub fn merge(&mut self, other: FreezeScope) {
        self.0 |= other.0;
    }
}

// ---------------------------------------------------------------------------
// Storage key / value
// ---------------------------------------------------------------------------

/// Typed storage keys — each variant maps to exactly one storage slot.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StorageKey {
    /// Required signature threshold M.
    Threshold,
    /// List of authorised operator public keys (Ed25519, 32 bytes each).
    Operators,
    /// Current freeze scope bitmask.
    FreezeScope,
    /// Unix timestamp after which an unpause is permitted.
    UnpauseAt,
    /// Operator public keys that have already signed the current freeze proposal.
    PausedBy,
}

/// Typed storage values.
#[derive(Debug, Clone)]
pub enum StorageValue {
    U8(u8),
    U64(u64),
    PublicKeys(Vec<[u8; 32]>),
    Scope(FreezeScope),
}

// ---------------------------------------------------------------------------
// In-memory state store (simulates Soroban instance storage)
// ---------------------------------------------------------------------------

/// Simulated Soroban instance storage.
///
/// In production this would be backed by `env.storage().instance()`.  The API
/// surface mirrors what a Soroban contract would call, making it straightforward
/// to swap in the real host SDK.
#[derive(Debug, Default)]
pub struct StateStore {
    inner: HashMap<StorageKey, StorageValue>,
}

impl StateStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, key: StorageKey, value: StorageValue) {
        self.inner.insert(key, value);
    }

    pub fn get(&self, key: &StorageKey) -> Option<&StorageValue> {
        self.inner.get(key)
    }

    // -- Typed convenience getters --

    pub fn threshold(&self) -> u8 {
        match self.get(&StorageKey::Threshold) {
            Some(StorageValue::U8(v)) => *v,
            _ => 0,
        }
    }

    pub fn operators(&self) -> Vec<[u8; 32]> {
        match self.get(&StorageKey::Operators) {
            Some(StorageValue::PublicKeys(v)) => v.clone(),
            _ => vec![],
        }
    }

    pub fn freeze_scope(&self) -> FreezeScope {
        match self.get(&StorageKey::FreezeScope) {
            Some(StorageValue::Scope(s)) => *s,
            _ => FreezeScope::NONE,
        }
    }

    pub fn unpause_at(&self) -> u64 {
        match self.get(&StorageKey::UnpauseAt) {
            Some(StorageValue::U64(v)) => *v,
            _ => 0,
        }
    }

    pub fn paused_by(&self) -> Vec<[u8; 32]> {
        match self.get(&StorageKey::PausedBy) {
            Some(StorageValue::PublicKeys(v)) => v.clone(),
            _ => vec![],
        }
    }

    // -- Typed convenience setters --

    pub fn set_threshold(&mut self, m: u8) {
        self.set(StorageKey::Threshold, StorageValue::U8(m));
    }

    pub fn set_operators(&mut self, ops: Vec<[u8; 32]>) {
        self.set(StorageKey::Operators, StorageValue::PublicKeys(ops));
    }

    pub fn set_freeze_scope(&mut self, scope: FreezeScope) {
        self.set(StorageKey::FreezeScope, StorageValue::Scope(scope));
    }

    pub fn set_unpause_at(&mut self, ts: u64) {
        self.set(StorageKey::UnpauseAt, StorageValue::U64(ts));
    }

    pub fn set_paused_by(&mut self, signers: Vec<[u8; 32]>) {
        self.set(StorageKey::PausedBy, StorageValue::PublicKeys(signers));
    }

    /// Fast O(1) pause check — a single bit-AND on the stored bitmask.
    ///
    /// This is the hot path called on every state-changing transaction.
    #[inline]
    pub fn is_frozen(&self, op: FreezeScope) -> bool {
        self.freeze_scope().contains(op)
    }
}

// ---------------------------------------------------------------------------
// Freeze lifecycle state machine
// ---------------------------------------------------------------------------

/// High-level states of the circuit breaker lifecycle.
///
/// ```text
///  ┌──────────┐   freeze()   ┌──────────┐   timelock expires   ┌──────────────┐
///  │  ACTIVE  │─────────────▶│  FROZEN  │──────────────────────▶│ PENDING_THAW │
///  └──────────┘              └──────────┘                        └──────────────┘
///       ▲                                                               │
///       └───────────────────── unfreeze() ────────────────────────────┘
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerState {
    /// No freeze active; all operations run normally.
    Active,
    /// A freeze is in force; matching operations revert immediately.
    Frozen,
    /// The timelock has expired but `unfreeze()` has not yet been called.
    PendingThaw,
}

impl BreakerState {
    /// Derive the current state from raw storage values.
    pub fn from_store(store: &StateStore, now: u64) -> Self {
        let scope = store.freeze_scope();
        if scope == FreezeScope::NONE {
            BreakerState::Active
        } else if now >= store.unpause_at() && store.unpause_at() > 0 {
            BreakerState::PendingThaw
        } else {
            BreakerState::Frozen
        }
    }
}

#[cfg(test)]
mod state_tests {
    use super::*;

    #[test]
    fn freeze_scope_contains() {
        let all = FreezeScope::ALL;
        assert!(all.contains(FreezeScope::DEPOSITS));
        assert!(all.contains(FreezeScope::WITHDRAWALS));
        assert!(all.contains(FreezeScope::GOVERNANCE));

        let deposits_only = FreezeScope::DEPOSITS;
        assert!(!deposits_only.contains(FreezeScope::WITHDRAWALS));
    }

    #[test]
    fn freeze_scope_merge() {
        let mut scope = FreezeScope::DEPOSITS;
        scope.merge(FreezeScope::WITHDRAWALS);
        assert!(scope.contains(FreezeScope::DEPOSITS));
        assert!(scope.contains(FreezeScope::WITHDRAWALS));
        assert!(!scope.contains(FreezeScope::GOVERNANCE));
    }

    #[test]
    fn state_store_is_frozen_single_lookup() {
        let mut store = StateStore::new();
        store.set_freeze_scope(FreezeScope::DEPOSITS);

        assert!(store.is_frozen(FreezeScope::DEPOSITS));
        assert!(!store.is_frozen(FreezeScope::WITHDRAWALS));
    }

    #[test]
    fn breaker_state_transitions() {
        let mut store = StateStore::new();

        // Active when no scope set
        assert_eq!(BreakerState::from_store(&store, 1000), BreakerState::Active);

        // Frozen while timelock is in the future
        store.set_freeze_scope(FreezeScope::ALL);
        store.set_unpause_at(2000);
        assert_eq!(BreakerState::from_store(&store, 1000), BreakerState::Frozen);

        // PendingThaw after timelock expires
        assert_eq!(BreakerState::from_store(&store, 2001), BreakerState::PendingThaw);
    }
}
