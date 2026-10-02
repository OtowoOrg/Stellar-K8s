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

//! # Cross-Shard Liquidity State Verifier (issue #333)
//!
//! A Soroban coordinating contract that enables atomic cross-shard arbitrage
//! trades across multiple independent AMM pool contracts.
//!
//! ## Problem
//!
//! As decentralised exchanges expand, liquidity fragments across many isolated
//! Soroban AMM pools.  A naive multi-hop trade that calls Pool A, then Pool B,
//! then Pool C in separate transactions risks partial execution: Pool A's swap
//! may succeed while Pool C's swap fails, leaving the trader with an unwanted
//! intermediate token position and no recourse.
//!
//! ## Solution
//!
//! This contract coordinates an ordered sequence of [`ShardTransition`]s in a
//! **single Soroban transaction**, leveraging the platform's own atomicity:
//!
//! * All three pool invocations occur in one transaction.
//! * If the third pool fails (e.g. insufficient liquidity, deadline expired,
//!   slippage exceeded), the coordinator calls `env.panic_with_error()`.
//! * Soroban's runtime reverts the *entire transaction's* state diff,
//!   including the successful legs against Pool A and Pool B.
//! * The trader is never left holding an unwanted intermediate token.
//!
//! ## Key modules
//!
//! | Module | Purpose |
//! |--------|---------|
//! | [`coordinator`] | Orchestration logic: validate inputs, lock pools, execute legs, revert on failure |
//! | [`locks`]       | Advisory pool-lock state machine (acquire, release, force-release, TTL) |
//! | [`types`]       | Shared XDR-serialisable types: [`ShardTransition`], [`SwapSnapshot`], [`PoolLock`] |
//! | [`errors`]      | [`CrossShardError`] discriminant enum |
//!
//! ## Usage
//!
//! ```text
//! // 1. Deploy and initialise the coordinator.
//! coordinator.initialize(admin);
//!
//! // 2. Build an array of desired state transitions.
//! let transitions = vec![
//!     ShardTransition { pool: amm_a, function: symbol!("swap"), amount_in: 1000, min_out: 990, ... },
//!     ShardTransition { pool: amm_b, function: symbol!("swap"), amount_in: 990,  min_out: 980, ... },
//!     ShardTransition { pool: amm_c, function: symbol!("swap"), amount_in: 980,  min_out: 970, ... },
//! ];
//!
//! // 3. Execute atomically — either all three commit or all three revert.
//! let amounts_out = coordinator.execute_atomic_swap(initiator, transitions);
//! ```
//!
//! ## Security properties
//!
//! * **All-or-nothing atomicity** — enforced by Soroban's transaction model
//!   plus an explicit `env.panic_with_error()` on any leg failure.
//! * **Slippage protection** — each leg carries a `min_out` bound verified
//!   before the next leg executes.
//! * **Deadline enforcement** — each leg carries a Unix timestamp deadline.
//! * **Reentrancy guard** — the `InFlight` storage flag prevents malicious
//!   AMM callbacks from triggering a nested swap.
//! * **Advisory pool locks** — prevent concurrent transactions (in the same
//!   or a subsequent ledger) from modifying locked pools during the sequence.
//! * **Zero multi-threading** — no OS threads, no async, no `Mutex`/`RwLock`;
//!   fully compatible with Stellar Core's single-threaded scheduler.

#![no_std]

pub mod coordinator;
pub mod errors;
pub mod locks;
pub mod types;

#[cfg(test)]
mod test;

// Re-export the most commonly used public types.
pub use errors::CrossShardError;
pub use types::{PoolLock, ShardTransition, SwapSnapshot, SwapStatus, TransitionResult};

use soroban_sdk::{contract, contractimpl, Address, Env, Vec};

// ── Contract declaration ──────────────────────────────────────────────────────

/// The Cross-Shard Liquidity State Verifier contract.
///
/// Deploy once; then call [`initialize`] to set the admin address.
/// All subsequent multi-shard swaps go through [`execute_atomic_swap`].
#[contract]
pub struct CrossShardCoordinator;

// ── Public interface ─────────────────────────────────────────────────────────

#[contractimpl]
impl CrossShardCoordinator {
    // ── Lifecycle ─────────────────────────────────────────────────────────────

    /// Initialise the coordinator with an `admin` address.
    ///
    /// Must be called exactly once, immediately after deployment.
    /// Returns [`CrossShardError::AlreadyInitialized`] if called again.
    pub fn initialize(env: Env, admin: Address) -> Result<(), CrossShardError> {
        coordinator::initialize(&env, &admin)
    }

    // ── Core operation ────────────────────────────────────────────────────────

    /// Execute an atomic multi-shard swap across all pools listed in
    /// `transitions`.
    ///
    /// All legs must succeed (and satisfy their `min_out` slippage bounds)
    /// for any state change to be committed.  If any leg fails or produces
    /// insufficient output, all state changes from every preceding leg are
    /// atomically reverted.
    ///
    /// # Arguments
    ///
    /// * `initiator`   — Address authorising the entire swap sequence.
    /// * `transitions` — Ordered list of per-pool state transitions.
    ///                   Must contain at least 2 and at most 16 entries.
    ///
    /// # Returns
    ///
    /// `Ok(Vec<i128>)` — actual output amounts from each leg, in order.
    ///
    /// # Errors
    ///
    /// See [`CrossShardError`] for the full list of error conditions.
    pub fn execute_atomic_swap(
        env:         Env,
        initiator:   Address,
        transitions: Vec<ShardTransition>,
    ) -> Result<Vec<i128>, CrossShardError> {
        coordinator::execute_atomic_swap(&env, &initiator, &transitions)
    }

    // ── Admin ─────────────────────────────────────────────────────────────────

    /// Pause or unpause the coordinator (admin only).
    ///
    /// While paused, `execute_atomic_swap` returns `ContractPaused`.
    pub fn set_paused(env: Env, caller: Address, paused: bool) -> Result<(), CrossShardError> {
        coordinator::set_paused(&env, &caller, paused)
    }

    /// Transfer admin control to `new_admin` (current admin only).
    pub fn transfer_admin(
        env:       Env,
        caller:    Address,
        new_admin: Address,
    ) -> Result<(), CrossShardError> {
        coordinator::transfer_admin(&env, &caller, &new_admin)
    }

    // ── Lock management ───────────────────────────────────────────────────────

    /// Force-release an expired advisory lock on `pool`.
    ///
    /// Any caller may invoke this after the lock's `expires_at` ledger has
    /// passed, to clean up locks left behind by failed/abandoned swaps.
    ///
    /// Returns `LockStillValid` if the lock has not yet expired.
    pub fn force_release_lock(env: Env, pool: Address) -> Result<(), CrossShardError> {
        locks::force_release(&env, &pool)
    }

    // ── View functions ────────────────────────────────────────────────────────

    /// Return the current admin address.
    pub fn get_admin(env: Env) -> Result<Address, CrossShardError> {
        coordinator::get_admin(&env)
    }

    /// Return `true` if the coordinator is currently paused.
    pub fn is_paused(env: Env) -> bool {
        coordinator::is_paused(&env)
    }

    /// Return `true` if a swap is currently in-flight.
    pub fn is_inflight(env: Env) -> bool {
        coordinator::is_inflight(&env)
    }

    /// Return the most recently issued swap ID.
    pub fn last_swap_id(env: Env) -> u64 {
        coordinator::last_swap_id(&env)
    }

    /// Retrieve a stored [`SwapSnapshot`] by swap ID.
    pub fn get_swap_snapshot(env: Env, swap_id: u64) -> Option<SwapSnapshot> {
        coordinator::get_swap_snapshot(&env, swap_id)
    }

    /// Return the lock record for `pool`, if one exists.
    pub fn get_pool_lock(env: Env, pool: Address) -> Option<PoolLock> {
        locks::read_lock(&env, &pool)
    }

    /// Return `true` if `pool` is currently held under a non-expired lock.
    pub fn pool_is_locked(env: Env, pool: Address) -> bool {
        locks::is_locked(&env, &pool)
    }
}
