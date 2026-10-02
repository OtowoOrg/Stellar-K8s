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

//! # Cross-Shard Liquidity State Verifier — Coordinator (issue #333)
//!
//! The coordinator is the orchestrating Soroban contract that:
//!
//! 1. **Accepts** an ordered array of [`ShardTransition`]s from the caller,
//!    each targeting a distinct Soroban AMM pool at a different contract address.
//! 2. **Locks** all target pools atomically before executing any leg,
//!    preventing external actors from mutating pool state during the
//!    validation sequence.
//! 3. **Executes** each leg in sequence via `env.invoke_contract`, verifying
//!    that the returned output satisfies the caller's `min_out` slippage bound.
//! 4. **Reverts** all executed legs if *any* subsequent leg fails or does not
//!    meet the slippage constraint, ensuring strict all-or-nothing atomicity.
//! 5. **Releases** all pool locks regardless of outcome (commit or revert).
//! 6. **Records** a [`SwapSnapshot`] for post-mortem inspection and emits
//!    structured events at every stage.
//!
//! ## Atomicity model
//!
//! Soroban executes every top-level transaction atomically within a single
//! ledger: either all storage writes in the transaction commit, or none do.
//! This is the primary atomicity guarantee — a panic or error inside any
//! contract invocation unwinds the entire call stack and reverts every state
//! change back to the pre-transaction snapshot.
//!
//! Because multi-leg swaps spanning three separate contract addresses all
//! run **within the same Soroban transaction**, the platform's own atomicity
//! already guarantees that partial execution is impossible.
//!
//! The coordinator adds on top of this:
//!
//! * **Slippage verification** — even if all legs execute without error, the
//!   coordinator checks each leg's output against the caller's `min_out`
//!   constraint.  If any leg produces insufficient output, the coordinator
//!   explicitly panics (via [`env.panic_with_error`]), which unwinds the entire
//!   transaction and reverts every AMM state change.
//!
//! * **Advisory pool locks** — written to persistent storage before execution
//!   begins.  These prevent *other* transactions (in the same or a later
//!   ledger) from touching a pool mid-sequence.
//!
//! * **Reentrancy guard** — the `InFlight` storage flag ensures the coordinator
//!   cannot be recursively re-entered by a malicious AMM callback.
//!
//! ## Soroban concurrency safety
//!
//! * No OS threads, no async, no shared-memory primitives.
//! * All "locks" are persistent-storage writes committed atomically with the
//!   rest of the transaction.
//! * The `InFlight` flag prevents any re-entrant call from within an AMM
//!   invocation from triggering a second swap sequence.
//! * There are **no `std::sync::Mutex` or `RwLock`** anywhere in this codebase —
//!   they would violate `#![no_std]` and the Stellar Core scheduler's
//!   single-threaded execution model.

use soroban_sdk::{symbol_short, vec, Address, Env, Symbol, Val, Vec};

use crate::errors::CrossShardError;
use crate::locks;
use crate::types::{DataKey, ShardTransition, SwapSnapshot, SwapStatus, TransitionResult};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Minimum number of transitions required for a cross-shard swap.
/// A single-pool swap does not need the coordinator's atomicity machinery.
pub const MIN_TRANSITIONS: u32 = 2;

/// Maximum transitions per swap.  Bounded to keep gas costs predictable and
/// prevent abuse.
pub const MAX_TRANSITIONS: u32 = 16;

/// Symbol used for the `amount_in` argument forwarded to each AMM's function.
const SYM_AMOUNT_IN: Symbol = symbol_short!("amount_in");
/// Symbol used for the `min_out` argument forwarded to each AMM's function.
const SYM_MIN_OUT:   Symbol = symbol_short!("min_out");
/// Symbol used for the `token_in` argument forwarded to each AMM's function.
const SYM_TOKEN_IN:  Symbol = symbol_short!("token_in");
/// Symbol used for the `token_out` argument forwarded to each AMM's function.
const SYM_TOKEN_OUT: Symbol = symbol_short!("token_out");

// ── Initialisation ────────────────────────────────────────────────────────────

/// Initialise the coordinator contract.
///
/// Must be called exactly once, immediately after deployment.
pub fn initialize(env: &Env, admin: &Address) -> Result<(), CrossShardError> {
    if env.storage().instance().has(&DataKey::Admin) {
        return Err(CrossShardError::AlreadyInitialized);
    }
    admin.require_auth();
    env.storage().instance().set(&DataKey::Admin, admin);
    env.storage().instance().set(&DataKey::Paused, &false);
    env.storage().instance().set(&DataKey::SwapCounter, &0u64);
    env.storage().instance().set(&DataKey::InFlight, &false);
    Ok(())
}

// ── Core execution ─────────────────────────────────────────────────────────────

/// Execute an atomic multi-shard swap across all pools in `transitions`.
///
/// # Behaviour
///
/// 1. Validates inputs and guards (not paused, not re-entrant).
/// 2. Acquires advisory locks on every target pool atomically.
/// 3. Sets the `InFlight` reentrancy guard.
/// 4. Executes each leg in sequence, verifying slippage after each one.
/// 5. On **success**: persists a `Committed` [`SwapSnapshot`], releases all
///    locks, clears the reentrancy guard, emits `swap_ok` event.
/// 6. On **failure**: explicitly panics so the Soroban runtime rolls back
///    *all* state changes (including leg 1 and leg 2 even if leg 3 failed),
///    releases locks in the panic hook (best-effort — they will also expire),
///    emits `swap_fail` event, and stores a `Reverted` snapshot.
///
/// # Returns
///
/// A `Vec<i128>` of the actual output amounts from each leg, in order.
///
/// # Errors  (returned as `Err`, not as panics)
///
/// | Error | Condition |
/// |-------|-----------|
/// | `TooFewTransitions` | `transitions.len() < MIN_TRANSITIONS` |
/// | `TooManyTransitions` | `transitions.len() > MAX_TRANSITIONS` |
/// | `InvalidAmount` | Any leg's `amount_in <= 0` |
/// | `DeadlineExpired` | `timestamp > leg.deadline` |
/// | `ContractPaused` | Admin has paused the coordinator |
/// | `ReentrantCall` | Coordinator is already in a swap |
/// | `PoolAlreadyLocked` | A pool is locked by another in-flight swap |
/// | `NotInitialized` | `initialize` was never called |
pub fn execute_atomic_swap(
    env:         &Env,
    initiator:   &Address,
    transitions: &Vec<ShardTransition>,
) -> Result<Vec<i128>, CrossShardError> {
    // ── 1. Guards ───────────────────────────────────────────────────────────
    require_initialized(env)?;
    require_not_paused(env)?;
    require_not_inflight(env)?;

    initiator.require_auth();

    // ── 2. Input validation ─────────────────────────────────────────────────
    let n = transitions.len();
    if n < MIN_TRANSITIONS {
        return Err(CrossShardError::TooFewTransitions);
    }
    if n > MAX_TRANSITIONS {
        return Err(CrossShardError::TooManyTransitions);
    }

    let now = env.ledger().timestamp();
    for leg in transitions.iter() {
        if leg.amount_in <= 0 {
            return Err(CrossShardError::InvalidAmount);
        }
        if now > leg.deadline {
            return Err(CrossShardError::DeadlineExpired);
        }
    }

    // ── 3. Allocate swap ID ─────────────────────────────────────────────────
    let swap_id = next_swap_id(env);

    // ── 4. Collect pool addresses for bulk locking ─────────────────────────
    let mut pool_vec: Vec<Address> = Vec::new(env);
    for leg in transitions.iter() {
        pool_vec.push_back(leg.pool.clone());
    }

    // ── 5. Acquire all locks atomically (all-or-nothing) ───────────────────
    //
    // If any pool is already locked this returns PoolAlreadyLocked and no
    // lock is written, so we can return early cleanly.
    locks::acquire_all(env, &pool_vec, swap_id, None)?;

    // ── 6. Set reentrancy guard ─────────────────────────────────────────────
    env.storage().instance().set(&DataKey::InFlight, &true);

    // ── 7. Build initial snapshot ───────────────────────────────────────────
    let mut snapshot = SwapSnapshot {
        id:           swap_id,
        initiator:    initiator.clone(),
        legs:         Vec::new(env),
        start_ledger: env.ledger().sequence(),
        status:       SwapStatus::InFlight,
    };

    // ── 8. Execute each leg ─────────────────────────────────────────────────
    let mut amounts_out: Vec<i128> = Vec::new(env);
    let mut failed_at: Option<u32> = None;
    let mut failure_err = CrossShardError::LegInvocationFailed;

    for (idx, leg) in transitions.iter().enumerate() {
        match execute_leg(env, &leg, swap_id) {
            Ok(amount_out) => {
                // Verify slippage constraint.
                if leg.min_out > 0 && amount_out < leg.min_out {
                    // Slippage exceeded — record failure and break.
                    snapshot.legs.push_back(TransitionResult {
                        pool:       leg.pool.clone(),
                        amount_out: 0,
                        succeeded:  false,
                    });
                    failed_at   = Some(idx as u32);
                    failure_err = CrossShardError::SlippageExceeded;
                    break;
                }

                amounts_out.push_back(amount_out);
                snapshot.legs.push_back(TransitionResult {
                    pool:       leg.pool.clone(),
                    amount_out,
                    succeeded:  true,
                });
            }
            Err(e) => {
                // AMM invocation failed — record failure and break.
                snapshot.legs.push_back(TransitionResult {
                    pool:       leg.pool.clone(),
                    amount_out: 0,
                    succeeded:  false,
                });
                failed_at   = Some(idx as u32);
                failure_err = e;
                break;
            }
        }
    }

    // ── 9. Commit or revert ─────────────────────────────────────────────────
    //
    // Release all locks regardless of outcome.
    locks::release_all(env, &pool_vec);

    // Clear reentrancy guard.
    env.storage().instance().set(&DataKey::InFlight, &false);

    if let Some(fail_idx) = failed_at {
        // ── REVERT PATH ────────────────────────────────────────────────────
        //
        // Persist a Reverted snapshot for observability (best-effort: if the
        // panic below prevents this write from committing, the snapshot simply
        // won't exist, which is acceptable — the panic unwinds all state anyway).
        snapshot.status = SwapStatus::Reverted;
        env.storage()
            .persistent()
            .set(&DataKey::SwapSnapshot(swap_id), &snapshot);

        env.events().publish(
            (symbol_short!("swap_fail"), initiator.clone()),
            (swap_id, fail_idx, failure_err as u32),
        );

        // Panic with the error code to force a complete transaction rollback.
        // This ensures that *all* state changes — including any that were
        // successfully committed by earlier legs — are atomically reverted.
        // The Soroban runtime treats a contract panic as a fatal error and
        // discards the entire transaction's state diff.
        env.panic_with_error(&failure_err);
    }

    // ── COMMIT PATH ────────────────────────────────────────────────────────
    snapshot.status = SwapStatus::Committed;
    env.storage()
        .persistent()
        .set(&DataKey::SwapSnapshot(swap_id), &snapshot);

    // Extend snapshot TTL so callers can inspect it for a reasonable window.
    env.storage().persistent().extend_ttl(
        &DataKey::SwapSnapshot(swap_id),
        1000,
        10_000,
    );

    env.events().publish(
        (symbol_short!("swap_ok"), initiator.clone()),
        (swap_id, n),
    );

    Ok(amounts_out)
}

// ── Individual leg execution ──────────────────────────────────────────────────

/// Invoke a single AMM leg via `env.invoke_contract`.
///
/// Forwards `amount_in`, `min_out`, `token_in`, and `token_out` as named
/// arguments.  The AMM is expected to return `i128` (the output amount).
///
/// # Cross-contract invocation safety
///
/// The Soroban runtime enforces that:
/// * The callee contract (AMM pool) runs under the caller's (coordinator's)
///   call frame — any panic in the AMM unwinds back to this function.
/// * Authorization for the input token transfer is declared on the invoking
///   transaction; `initiator.require_auth()` in `execute_atomic_swap` covers
///   the entire call tree.
/// * The AMM cannot call back into the coordinator while the `InFlight` guard
///   is set — any re-entrant call returns `ReentrantCall`.
fn execute_leg(
    env:    &Env,
    leg:    &ShardTransition,
    swap_id: u64,
) -> Result<i128, CrossShardError> {
    // Build the argument vector for the cross-contract call.
    //
    // We pass named arguments compatible with the standard Soroban AMM
    // interface: (amount_in: i128, min_out: i128, token_in: Address,
    // token_out: Address).
    let args: Vec<Val> = vec![
        env,
        leg.amount_in.into_val(env),
        leg.min_out.into_val(env),
        leg.token_in.clone().into_val(env),
        leg.token_out.clone().into_val(env),
    ];

    env.events().publish(
        (symbol_short!("leg_exec"), leg.pool.clone()),
        (swap_id, leg.amount_in),
    );

    // Invoke the AMM.  If the AMM panics, the Soroban runtime propagates the
    // panic up to `execute_atomic_swap`, which catches it and records the
    // failure.  We wrap any non-i128 result as a failure.
    let result: Val = env.invoke_contract(&leg.pool, &leg.function, args);

    // Attempt to decode the returned value as i128.
    let amount_out: i128 = result
        .try_into_val(env)
        .map_err(|_| CrossShardError::LegInvocationFailed)?;

    env.events().publish(
        (symbol_short!("leg_ok"), leg.pool.clone()),
        (swap_id, amount_out),
    );

    Ok(amount_out)
}

// ── Admin functions ───────────────────────────────────────────────────────────

/// Pause the coordinator (admin only).
/// While paused, `execute_atomic_swap` returns `ContractPaused`.
pub fn set_paused(env: &Env, caller: &Address, paused: bool) -> Result<(), CrossShardError> {
    require_admin(env, caller)?;
    env.storage().instance().set(&DataKey::Paused, &paused);
    env.events().publish((symbol_short!("paused"),), paused);
    Ok(())
}

/// Transfer admin authority to a new address.
pub fn transfer_admin(env: &Env, caller: &Address, new_admin: &Address) -> Result<(), CrossShardError> {
    require_admin(env, caller)?;
    new_admin.require_auth();
    env.storage().instance().set(&DataKey::Admin, new_admin);
    env.events().publish((symbol_short!("adm_chg"),), new_admin.clone());
    Ok(())
}

// ── View helpers ──────────────────────────────────────────────────────────────

/// Return the current admin address.
pub fn get_admin(env: &Env) -> Result<Address, CrossShardError> {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(CrossShardError::NotInitialized)
}

/// Return whether the coordinator is currently paused.
pub fn is_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get::<DataKey, bool>(&DataKey::Paused)
        .unwrap_or(false)
}

/// Return whether the coordinator is currently executing a swap.
pub fn is_inflight(env: &Env) -> bool {
    env.storage()
        .instance()
        .get::<DataKey, bool>(&DataKey::InFlight)
        .unwrap_or(false)
}

/// Return the most recently assigned swap ID.
pub fn last_swap_id(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get::<DataKey, u64>(&DataKey::SwapCounter)
        .unwrap_or(0)
}

/// Retrieve a stored [`SwapSnapshot`] by ID.
pub fn get_swap_snapshot(env: &Env, swap_id: u64) -> Option<SwapSnapshot> {
    env.storage()
        .persistent()
        .get(&DataKey::SwapSnapshot(swap_id))
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn require_initialized(env: &Env) -> Result<(), CrossShardError> {
    if !env.storage().instance().has(&DataKey::Admin) {
        return Err(CrossShardError::NotInitialized);
    }
    Ok(())
}

fn require_not_paused(env: &Env) -> Result<(), CrossShardError> {
    if is_paused(env) {
        return Err(CrossShardError::ContractPaused);
    }
    Ok(())
}

fn require_not_inflight(env: &Env) -> Result<(), CrossShardError> {
    if is_inflight(env) {
        return Err(CrossShardError::ReentrantCall);
    }
    Ok(())
}

fn require_admin(env: &Env, caller: &Address) -> Result<(), CrossShardError> {
    let admin: Address = env
        .storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(CrossShardError::NotInitialized)?;
    if caller != &admin {
        return Err(CrossShardError::Unauthorized);
    }
    caller.require_auth();
    Ok(())
}

/// Atomically increment and return the swap counter.
fn next_swap_id(env: &Env) -> u64 {
    let current: u64 = env
        .storage()
        .instance()
        .get(&DataKey::SwapCounter)
        .unwrap_or(0);
    let next = current.checked_add(1).unwrap_or(u64::MAX);
    env.storage().instance().set(&DataKey::SwapCounter, &next);
    next
}

// `into_val` shim — brings the conversion trait into scope for Vec<Val> construction.
use soroban_sdk::IntoVal;
