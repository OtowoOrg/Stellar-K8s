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

//! Integration tests for the Cross-Shard Liquidity State Verifier (issue #333).
//!
//! # Test structure
//!
//! 1. A minimal `MockAmm` Soroban contract that acts as a fake AMM pool.
//!    It records incoming swaps and can be configured to either succeed
//!    (return `amount_in * rate / 100`) or deliberately fail with a panic.
//!
//! 2. The full three-pool validation scenario required by issue #333:
//!    > Attempt an atomic swap requiring liquidity from three separate AMM
//!    > contracts; intentionally induce a failure in the third contract and
//!    > assert that the state changes in the first two strictly revert.
//!
//! # Atomicity verification
//!
//! After the third pool fails, we verify that:
//! * The coordinator's `SwapSnapshot` records `SwapStatus::Reverted` (or does
//!   not exist at all if the panic prevented the write from committing).
//! * All advisory pool locks have been released.
//! * The coordinator's `InFlight` flag is cleared.
//! * The mock AMMs' swap counts have NOT incremented (because the entire
//!   transaction was rolled back by the platform).

#![allow(unused)]

extern crate std;

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short,
    testutils::{Address as _, Ledger},
    vec, Address, Env, Symbol, Val,
};

use crate::{
    coordinator,
    errors::CrossShardError,
    locks,
    types::{DataKey, ShardTransition, SwapStatus},
    CrossShardCoordinator, CrossShardCoordinatorClient,
};

// ── Mock AMM contract ────────────────────────────────────────────────────────

/// Storage keys for the mock AMM.
#[contracttype]
enum AmmKey {
    /// Exchange rate: output = amount_in * Rate / 100
    Rate,
    /// Whether the next swap call should panic.
    ShouldFail,
    /// Count of swap calls that have completed (not panicked).
    SwapCount,
    /// Cumulative input tokens received.
    TotalIn,
}

/// A minimal AMM pool stub for testing.
///
/// * `set_rate(rate)` — configure exchange rate (default 95 = 95%).
/// * `set_should_fail(true)` — makes the next `swap` call panic.
/// * `swap(amount_in, min_out, token_in, token_out)` — returns
///   `amount_in * rate / 100`, or panics if `should_fail` is set.
/// * `swap_count()` — number of successful swaps.
/// * `total_in()` — cumulative input token volume.
#[contract]
pub struct MockAmm;

#[contractimpl]
impl MockAmm {
    pub fn set_rate(env: Env, rate: i128) {
        env.storage().instance().set(&AmmKey::Rate, &rate);
    }

    pub fn set_should_fail(env: Env, fail: bool) {
        env.storage().instance().set(&AmmKey::ShouldFail, &fail);
    }

    /// Simulate a swap.  Panics if `should_fail` is true.
    /// Returns `amount_in * rate / 100`.
    pub fn swap(
        env:       Env,
        amount_in: i128,
        min_out:   i128,
        token_in:  Address,
        token_out: Address,
    ) -> i128 {
        // Deliberate failure path — simulates insufficient liquidity / other error.
        let should_fail: bool = env
            .storage()
            .instance()
            .get(&AmmKey::ShouldFail)
            .unwrap_or(false);
        if should_fail {
            panic!("MockAmm: insufficient liquidity (simulated failure)");
        }

        let rate: i128 = env
            .storage()
            .instance()
            .get(&AmmKey::Rate)
            .unwrap_or(95);

        let amount_out = amount_in
            .checked_mul(rate)
            .unwrap()
            .checked_div(100)
            .unwrap();

        // Verify slippage internally (AMM-side check mirrors coordinator check).
        assert!(
            amount_out >= min_out,
            "MockAmm: slippage exceeded ({} < {})",
            amount_out,
            min_out
        );

        // Increment counters (these writes are reverted if the transaction rolls back).
        let count: u64 = env
            .storage()
            .instance()
            .get(&AmmKey::SwapCount)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&AmmKey::SwapCount, &(count + 1));

        let total_in: i128 = env
            .storage()
            .instance()
            .get(&AmmKey::TotalIn)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&AmmKey::TotalIn, &(total_in + amount_in));

        amount_out
    }

    pub fn swap_count(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&AmmKey::SwapCount)
            .unwrap_or(0)
    }

    pub fn total_in(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&AmmKey::TotalIn)
            .unwrap_or(0)
    }
}

// ── Test helpers ─────────────────────────────────────────────────────────────

/// Register the coordinator contract and three AMM pool stubs.
///
/// Returns `(env, coordinator_client, amm_a_addr, amm_b_addr, amm_c_addr, admin, trader)`.
fn setup() -> (
    Env,
    CrossShardCoordinatorClient<'static>,
    Address, // amm_a
    Address, // amm_b
    Address, // amm_c
    Address, // admin
    Address, // trader
) {
    let env = Env::default();
    env.mock_all_auths();

    // Register the coordinator.
    let coord_id = env.register(CrossShardCoordinator, ());
    let coord = CrossShardCoordinatorClient::new(&env, &coord_id);

    // Register three independent AMM pool stubs.
    let amm_a = env.register(MockAmm, ());
    let amm_b = env.register(MockAmm, ());
    let amm_c = env.register(MockAmm, ());

    let amm_a_client = MockAmmClient::new(&env, &amm_a);
    let amm_b_client = MockAmmClient::new(&env, &amm_b);
    let amm_c_client = MockAmmClient::new(&env, &amm_c);

    // Configure AMMs with 95% exchange rate by default.
    amm_a_client.set_rate(&95i128);
    amm_b_client.set_rate(&95i128);
    amm_c_client.set_rate(&95i128);

    let admin  = Address::generate(&env);
    let trader = Address::generate(&env);

    coord.initialize(&admin);

    // Set a ledger timestamp far in the future so deadlines don't expire.
    env.ledger().with_mut(|l| {
        l.timestamp       = 1_000_000;
        l.sequence_number = 100;
    });

    (env, coord, amm_a, amm_b, amm_c, admin, trader)
}

/// Build a `ShardTransition` with sensible defaults.
fn make_transition(
    env:       &Env,
    pool:       Address,
    amount_in:  i128,
    min_out:    i128,
) -> ShardTransition {
    ShardTransition {
        pool,
        function:  Symbol::new(env, "swap"),
        amount_in,
        min_out,
        token_in:  Address::generate(env),
        token_out: Address::generate(env),
        deadline:  u64::MAX,
    }
}

// ── Core tests ────────────────────────────────────────────────────────────────

/// Happy path: all three AMMs succeed; verify commit and output amounts.
#[test]
fn three_pool_swap_all_succeed() {
    let (env, coord, amm_a, amm_b, amm_c, _admin, trader) = setup();

    // Leg 1: 1000 in → 950 out (95%)
    // Leg 2: 950 in  → 902 out (95%)
    // Leg 3: 902 in  → 856 out (95%)
    let transitions = vec![
        &env,
        make_transition(&env, amm_a.clone(), 1000, 900),
        make_transition(&env, amm_b.clone(), 950,  850),
        make_transition(&env, amm_c.clone(), 902,  800),
    ];

    let amounts_out = coord
        .execute_atomic_swap(&trader, &transitions)
        .expect("three-pool swap should succeed");

    assert_eq!(amounts_out.len(), 3);
    assert_eq!(amounts_out.get(0).unwrap(), 950);  // 1000 * 95 / 100
    assert_eq!(amounts_out.get(1).unwrap(), 902);  // 950  * 95 / 100
    assert_eq!(amounts_out.get(2).unwrap(), 856);  // 902  * 95 / 100

    // Coordinator should have issued swap ID 1.
    assert_eq!(coord.last_swap_id(), 1);

    // Snapshot should show Committed status.
    let snap = coord
        .get_swap_snapshot(&1u64)
        .expect("snapshot should exist after commit");
    assert_eq!(snap.status, SwapStatus::Committed);
    assert_eq!(snap.legs.len(), 3);
    assert!(snap.legs.get(0).unwrap().succeeded);
    assert!(snap.legs.get(1).unwrap().succeeded);
    assert!(snap.legs.get(2).unwrap().succeeded);

    // All locks must be released after a successful swap.
    assert!(!coord.pool_is_locked(&amm_a));
    assert!(!coord.pool_is_locked(&amm_b));
    assert!(!coord.pool_is_locked(&amm_c));

    // Reentrancy guard must be cleared.
    assert!(!coord.is_inflight());
}

/// Issue #333 validation scenario:
/// Induce a failure in the third AMM and assert that legs 1 and 2 strictly revert.
#[test]
fn three_pool_swap_third_fails_all_revert() {
    let (env, coord, amm_a, amm_b, amm_c, _admin, trader) = setup();

    // Configure AMM C to fail.
    let amm_c_client = MockAmmClient::new(&env, &amm_c);
    amm_c_client.set_should_fail(&true);

    let amm_a_client = MockAmmClient::new(&env, &amm_a);
    let amm_b_client = MockAmmClient::new(&env, &amm_b);

    let transitions = vec![
        &env,
        make_transition(&env, amm_a.clone(), 1000, 900),
        make_transition(&env, amm_b.clone(), 950,  850),
        make_transition(&env, amm_c.clone(), 902,  800), // ← this leg will fail
    ];

    // The swap must return an error (or panic, caught by the test harness).
    let result = coord.execute_atomic_swap(&trader, &transitions);
    assert!(
        result.is_err(),
        "swap must fail when the third AMM panics"
    );

    // ── Atomicity verification ──────────────────────────────────────────────
    //
    // Because Soroban rolls back the *entire transaction* on panic, every
    // state change made during the swap — including AMM A's and AMM B's
    // swap_count increments — must be reverted.

    // AMM A and B swap counters must remain at zero (their writes reverted).
    assert_eq!(
        amm_a_client.swap_count(),
        0,
        "AMM A swap_count must revert to 0 after third-pool failure"
    );
    assert_eq!(
        amm_b_client.swap_count(),
        0,
        "AMM B swap_count must revert to 0 after third-pool failure"
    );

    // Total input on AMM A and B must also be zero.
    assert_eq!(
        amm_a_client.total_in(),
        0,
        "AMM A total_in must revert to 0"
    );
    assert_eq!(
        amm_b_client.total_in(),
        0,
        "AMM B total_in must revert to 0"
    );

    // All advisory locks must have been released (or were reverted).
    assert!(!coord.pool_is_locked(&amm_a), "lock on AMM A must be released");
    assert!(!coord.pool_is_locked(&amm_b), "lock on AMM B must be released");
    assert!(!coord.pool_is_locked(&amm_c), "lock on AMM C must be released");

    // The reentrancy guard must be cleared.
    assert!(
        !coord.is_inflight(),
        "InFlight flag must be cleared after rollback"
    );

    // Swap counter: because the transaction rolled back, the coordinator's
    // SwapCounter increment is also reverted, so it stays at 0.
    assert_eq!(
        coord.last_swap_id(),
        0,
        "swap counter must revert to 0 after rollback"
    );
}

/// Slippage guard: even if the AMM returns a value, if it's below min_out the
/// coordinator must revert all legs.
#[test]
fn slippage_on_second_leg_reverts_all() {
    let (env, coord, amm_a, amm_b, amm_c, _admin, trader) = setup();

    let amm_a_client = MockAmmClient::new(&env, &amm_a);
    let amm_b_client = MockAmmClient::new(&env, &amm_b);

    // AMM B produces 902 (95% of 950), but min_out for leg 2 is set to 999 (impossible).
    let transitions = vec![
        &env,
        make_transition(&env, amm_a.clone(), 1000, 900),
        ShardTransition {
            pool:      amm_b.clone(),
            function:  Symbol::new(&env, "swap"),
            amount_in: 950,
            min_out:   999, // ← impossible: 950 * 0.95 = 902 < 999
            token_in:  Address::generate(&env),
            token_out: Address::generate(&env),
            deadline:  u64::MAX,
        },
        make_transition(&env, amm_c.clone(), 902, 800),
    ];

    let result = coord.execute_atomic_swap(&trader, &transitions);
    assert!(result.is_err(), "slippage on leg 2 must cause overall failure");

    // Leg 1 (AMM A) must have its writes reverted.
    assert_eq!(
        amm_a_client.swap_count(),
        0,
        "AMM A must be reverted after slippage failure on leg 2"
    );
    assert!(!coord.pool_is_locked(&amm_a));
    assert!(!coord.pool_is_locked(&amm_b));
    assert!(!coord.is_inflight());
}

/// Deadline enforcement: a leg with an expired deadline must be rejected
/// before any pool invocation occurs.
#[test]
fn expired_deadline_rejected_before_execution() {
    let (env, coord, amm_a, amm_b, amm_c, _admin, trader) = setup();

    let amm_a_client = MockAmmClient::new(&env, &amm_a);

    // Set ledger timestamp to 2_000_000; transition deadlines are at 1_500_000 (expired).
    env.ledger().with_mut(|l| {
        l.timestamp = 2_000_000;
    });

    let transitions = vec![
        &env,
        ShardTransition {
            pool:      amm_a.clone(),
            function:  Symbol::new(&env, "swap"),
            amount_in: 1000,
            min_out:   900,
            token_in:  Address::generate(&env),
            token_out: Address::generate(&env),
            deadline:  1_500_000, // ← expired
        },
        make_transition(&env, amm_b.clone(), 950, 850),
        make_transition(&env, amm_c.clone(), 902, 800),
    ];

    let result = coord.execute_atomic_swap(&trader, &transitions);
    assert!(result.is_err(), "expired deadline must reject the swap");

    // No AMM was touched; AMM A's counter stays at zero.
    assert_eq!(amm_a_client.swap_count(), 0);
    assert!(!coord.pool_is_locked(&amm_a));
    assert!(!coord.is_inflight());
}

/// TooFewTransitions: only one pool supplied.
#[test]
fn too_few_transitions_rejected() {
    let (env, coord, amm_a, _amm_b, _amm_c, _admin, trader) = setup();

    let transitions = vec![
        &env,
        make_transition(&env, amm_a.clone(), 1000, 900),
    ];

    let result = coord.execute_atomic_swap(&trader, &transitions);
    assert!(result.is_err(), "single-pool swap must be rejected");
}

/// Admin pause: swaps are rejected while the coordinator is paused.
#[test]
fn swap_rejected_when_paused() {
    let (env, coord, amm_a, amm_b, amm_c, admin, trader) = setup();

    coord.set_paused(&admin, &true).unwrap();
    assert!(coord.is_paused());

    let transitions = vec![
        &env,
        make_transition(&env, amm_a.clone(), 1000, 900),
        make_transition(&env, amm_b.clone(), 950,  850),
        make_transition(&env, amm_c.clone(), 902,  800),
    ];

    let result = coord.execute_atomic_swap(&trader, &transitions);
    assert!(result.is_err(), "swap must fail when coordinator is paused");

    // Unpause and verify swaps work again.
    coord.set_paused(&admin, &false).unwrap();
    assert!(!coord.is_paused());
    let result2 = coord.execute_atomic_swap(&trader, &transitions);
    assert!(result2.is_ok(), "swap must succeed after unpause");
}

/// Double-initialisation must be rejected.
#[test]
fn double_initialize_rejected() {
    let (env, coord, _amm_a, _amm_b, _amm_c, admin, _trader) = setup();
    // Already initialized in setup(); second call must fail.
    let result = coord.initialize(&admin);
    assert!(result.is_err(), "double initialize must return AlreadyInitialized");
}

/// Force-releasing a lock that hasn't expired must be rejected.
#[test]
fn force_release_unexpired_lock_rejected() {
    let (env, coord, amm_a, _amm_b, _amm_c, _admin, trader) = setup();

    // Manually insert a valid (non-expired) lock.
    let lock = crate::types::PoolLock {
        pool:        amm_a.clone(),
        held_by:     Address::generate(&env),
        acquired_at: env.ledger().sequence(),
        expires_at:  env.ledger().sequence() + 100,
        swap_id:     1,
    };
    env.as_contract(&coord.address, || {
        env.storage()
            .persistent()
            .set(&DataKey::PoolLock(amm_a.clone()), &lock);
    });

    let result = coord.force_release_lock(&amm_a);
    assert!(
        result.is_err(),
        "force_release on a valid lock must fail with LockStillValid"
    );
}

/// Successful two-pool swap (minimum valid count).
#[test]
fn two_pool_swap_succeeds() {
    let (env, coord, amm_a, amm_b, _amm_c, _admin, trader) = setup();

    let transitions = vec![
        &env,
        make_transition(&env, amm_a.clone(), 1000, 900),
        make_transition(&env, amm_b.clone(), 950,  850),
    ];

    let amounts_out = coord
        .execute_atomic_swap(&trader, &transitions)
        .expect("two-pool swap should succeed");

    assert_eq!(amounts_out.len(), 2);
    assert_eq!(amounts_out.get(0).unwrap(), 950);
    assert_eq!(amounts_out.get(1).unwrap(), 902);
}

/// Sequential swaps increment the swap counter correctly.
#[test]
fn sequential_swaps_increment_counter() {
    let (env, coord, amm_a, amm_b, amm_c, _admin, trader) = setup();

    let t = || {
        vec![
            &env,
            make_transition(&env, amm_a.clone(), 1000, 900),
            make_transition(&env, amm_b.clone(), 950,  850),
            make_transition(&env, amm_c.clone(), 902,  800),
        ]
    };

    coord.execute_atomic_swap(&trader, &t()).unwrap();
    assert_eq!(coord.last_swap_id(), 1);

    coord.execute_atomic_swap(&trader, &t()).unwrap();
    assert_eq!(coord.last_swap_id(), 2);

    coord.execute_atomic_swap(&trader, &t()).unwrap();
    assert_eq!(coord.last_swap_id(), 3);
}

/// Pool locking: if Pool A is already locked, a second swap involving Pool A
/// must be rejected with PoolAlreadyLocked.
#[test]
fn concurrent_swap_rejected_if_pool_locked() {
    let (env, coord, amm_a, amm_b, amm_c, _admin, trader) = setup();

    // Manually insert a valid lock on amm_a as if a prior swap is in-flight.
    let valid_lock = crate::types::PoolLock {
        pool:        amm_a.clone(),
        held_by:     coord.address.clone(),
        acquired_at: env.ledger().sequence(),
        expires_at:  env.ledger().sequence() + 100,
        swap_id:     99,
    };
    env.as_contract(&coord.address, || {
        env.storage()
            .persistent()
            .set(&DataKey::PoolLock(amm_a.clone()), &valid_lock);
    });

    let transitions = vec![
        &env,
        make_transition(&env, amm_a.clone(), 1000, 900), // ← locked
        make_transition(&env, amm_b.clone(), 950,  850),
        make_transition(&env, amm_c.clone(), 902,  800),
    ];

    let result = coord.execute_atomic_swap(&trader, &transitions);
    assert!(
        result.is_err(),
        "swap must fail with PoolAlreadyLocked when a pool is locked"
    );
}
