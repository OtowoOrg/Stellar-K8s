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

//! Shared types for the Cross-Shard Liquidity State Verifier (issue #333).
//!
//! All types derive `#[contracttype]` so they are XDR-serialisable and can be
//! stored in Soroban persistent/instance storage and passed across contract
//! invocation boundaries.

use soroban_sdk::{contracttype, Address, Symbol, Vec};

// ── Storage keys ─────────────────────────────────────────────────────────────

/// Top-level storage keys for the coordinator contract.
///
/// Every variant is prefixed with a short namespace so it cannot collide with
/// any guest contract that links this crate as a library.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// The privileged address that controls contract configuration.
    Admin,
    /// Whether the entire coordinator is paused.
    Paused,
    /// Running count of swap operations ever initiated (used for nonce / event IDs).
    SwapCounter,
    /// Per-pool lock entry: `PoolLock(pool_address)`.
    PoolLock(Address),
    /// Per-swap state snapshot keyed by the initiating swap ID.
    SwapSnapshot(u64),
    /// Boolean flag indicating whether the current execution is inside the
    /// atomic validation sequence.  Acts as a reentrancy guard.
    InFlight,
}

// ── Core domain types ─────────────────────────────────────────────────────────

/// Describes a single desired state transition targeting one Soroban AMM pool.
///
/// The coordinator collects an ordered `Vec<ShardTransition>` from the caller
/// and either commits all of them or reverts all of them atomically.
///
/// # Fields
///
/// * `pool`        — Address of the target AMM pool contract.
/// * `function`    — Soroban function name to invoke on `pool` (e.g. `swap`).
/// * `amount_in`   — Token amount the initiator supplies to this pool.
/// * `min_out`     — Minimum output the initiator requires from this pool.
///                   Acts as slippage protection; set to 0 to disable.
/// * `token_in`    — SAC address of the input token for this leg.
/// * `token_out`   — SAC address of the output token for this leg.
/// * `deadline`    — Unix timestamp after which this transition must not execute.
///                   Use `u64::MAX` to disable.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShardTransition {
    pub pool:      Address,
    pub function:  Symbol,
    pub amount_in: i128,
    pub min_out:   i128,
    pub token_in:  Address,
    pub token_out: Address,
    pub deadline:  u64,
}

/// Outcome record for a single shard transition, stored per-leg inside a
/// [`SwapSnapshot`].
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransitionResult {
    /// Pool that was targeted.
    pub pool:       Address,
    /// Actual output tokens received (0 if not executed).
    pub amount_out: i128,
    /// Whether this transition completed successfully.
    pub succeeded:  bool,
}

/// Snapshot of the full swap state stored in persistent storage so that the
/// revert path can reconstruct what happened if a later leg fails.
///
/// Stored under `DataKey::SwapSnapshot(id)` for post-mortem inspection.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwapSnapshot {
    /// Monotonically increasing swap identifier.
    pub id:           u64,
    /// Who initiated this swap.
    pub initiator:    Address,
    /// Ordered list of leg results (populated as execution proceeds).
    pub legs:         Vec<TransitionResult>,
    /// Ledger sequence at which this swap was initiated.
    pub start_ledger: u32,
    /// Overall status of this swap.
    pub status:       SwapStatus,
}

/// High-level lifecycle status of a multi-shard swap.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SwapStatus {
    /// Execution is currently in progress.
    InFlight  = 0,
    /// All legs committed successfully.
    Committed = 1,
    /// One or more legs failed; all changes reverted.
    Reverted  = 2,
}

// ── Lock types (also used by locks.rs) ───────────────────────────────────────

/// Describes the lock state of a single AMM pool during the validation
/// sequence.
///
/// The lock is an advisory mutex: the coordinator writes it before invoking
/// the target pool and removes it after the atomic sequence completes.
/// No external actor (other than the coordinator itself) should mutate the
/// pool while a `PoolLock` for that address is held — any external
/// modification during the window constitutes a sequencing violation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolLock {
    /// Address of the locked pool.
    pub pool:         Address,
    /// Coordinator's own contract address — used to verify lock ownership.
    pub held_by:      Address,
    /// Ledger sequence when the lock was acquired.
    pub acquired_at:  u32,
    /// Maximum ledger by which the lock must be released.
    /// After this ledger any actor can force-release a stale lock.
    pub expires_at:   u32,
    /// Swap ID this lock belongs to.
    pub swap_id:      u64,
}
