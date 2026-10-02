//! Integration tests for the SmartMultiSig contract.
//!
//! # Test matrix
//!
//! | Test                                  | Scenario                                                    | Expected outcome            |
//! |---------------------------------------|-------------------------------------------------------------|-----------------------------|
//! | `test_valid_transfer_2_of_3`          | 2-of-3 multisig, fee and TWAP within policy                 | Transfer succeeds           |
//! | `test_execute_reverts_on_fee_spike`   | Submitted fee above policy ceiling                          | `FeeTooHigh` revert         |
//! | `test_execute_reverts_on_twap_spike`  | Oracle returns price >5% from reference                     | `TwapDeviationTooLarge`     |
//! | `test_propose_requires_signer`        | Non-signer attempts to propose                              | `Unauthorized`              |
//! | `test_approve_once_only`              | Signer approves twice                                       | `AlreadySigned`             |
//! | `test_threshold_not_met`              | Only 1 of 2 approvals collected                             | `ThresholdNotMet`           |
//! | `test_set_policy_unauthorized`        | Non-admin tries to update policy                            | `Unauthorized`              |
//! | `test_policy_disable_fee_guard`       | Fee guard disabled via sentinel `u32::MAX`                  | Succeeds despite high fee   |
//! | `test_policy_disable_twap_guard`      | TWAP guard disabled via sentinel `u32::MAX`                 | Succeeds despite deviation  |
//! | `test_rotate_admin`                   | Admin rotates; old admin loses access                       | New admin can update policy |
//!
//! # Client API note (soroban-sdk 27)
//!
//! In SDK 27 the generated contract client exposes two variants of each fn:
//!
//! - `client.method(...)` — panics on error, returns `T` directly.
//! - `client.try_method(...)` → `Result<T, soroban_sdk::Error>` — used for
//!   testing error cases without panicking.
//!
//! # Mock oracle
//!
//! The Soroban test environment handles cross-contract calls for contracts
//! registered within the same `Env`.  A minimal `MockTwapOracle` exposes
//! `get_twap() → i128` and `set_twap(price: i128)` for price injection.
//!
//! # Fee guard test design
//!
//! The Soroban SDK 27 host does not expose a ledger `base_fee` inside WASM.
//! The contract accepts `current_fee_stroops` as an argument to
//! `execute_transfer`.  Fee spike tests pass a large value to simulate
//! network congestion.

#![cfg(test)]

extern crate std;

use soroban_sdk::{
    contract, contractimpl,
    testutils::Address as _,
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

use crate::{
    policy::{ExecutionPolicy, PolicyError},
    SmartMultiSig, SmartMultiSigClient,
};

// ---------------------------------------------------------------------------
// Mock TWAP Oracle contract
// ---------------------------------------------------------------------------

/// Minimal oracle storing a single `i128` price in instance storage.
#[contract]
pub struct MockTwapOracle;

#[contractimpl]
impl MockTwapOracle {
    pub fn init(env: Env, price: i128) {
        env.storage()
            .instance()
            .set(&soroban_sdk::symbol_short!("price"), &price);
    }

    pub fn set_twap(env: Env, price: i128) {
        env.storage()
            .instance()
            .set(&soroban_sdk::symbol_short!("price"), &price);
    }

    /// Called by the multisig during `execute_transfer`.
    pub fn get_twap(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&soroban_sdk::symbol_short!("price"))
            .unwrap_or(1_000_000i128)
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Normal fee used in passing tests (below the 1 000-stroop policy ceiling).
const NORMAL_FEE: u32 = 100;

/// Spiked fee to trigger the fee guard (above the 1 000-stroop ceiling).
const SPIKED_FEE: u32 = 5_000;

fn make_policy(
    oracle: &Address,
    max_fee: u32,
    reference_price: i128,
    max_deviation_bps: u32,
) -> ExecutionPolicy {
    ExecutionPolicy {
        max_allowed_fee_stroops: max_fee,
        twap_oracle: oracle.clone(),
        reference_twap_price: reference_price,
        max_price_deviation_bps: max_deviation_bps,
    }
}

struct Harness {
    env: Env,
    multisig: SmartMultiSigClient<'static>,
    oracle: MockTwapOracleClient<'static>,
    token: Address,
    admin: Address,
    signer_a: Address,
    signer_b: Address,
    #[allow(dead_code)]
    signer_c: Address,
    recipient: Address,
}

impl Harness {
    fn new_2_of_3() -> Self {
        Self::new_with_policy_params(1_000, 1_000_000, 500)
    }

    fn new_with_policy_params(max_fee: u32, reference_price: i128, max_deviation_bps: u32) -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let signer_a = Address::generate(&env);
        let signer_b = Address::generate(&env);
        let signer_c = Address::generate(&env);
        let recipient = Address::generate(&env);

        let oracle_id = env.register(MockTwapOracle, ());
        let oracle = MockTwapOracleClient::new(&env, &oracle_id);
        oracle.init(&reference_price);

        let token_admin = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract_v2(token_admin.clone());
        let token = token_id.address();

        let signers =
            soroban_sdk::vec![&env, signer_a.clone(), signer_b.clone(), signer_c.clone()];
        let policy = make_policy(&oracle_id, max_fee, reference_price, max_deviation_bps);

        let multisig_id = env.register(
            SmartMultiSig,
            (admin.clone(), signers, 2u32, policy),
        );
        let multisig = SmartMultiSigClient::new(&env, &multisig_id);

        // Fund multisig with 10 000 token units.
        StellarAssetClient::new(&env, &token).mint(&multisig_id, &10_000i128);

        // SAFETY: lifetime erasure — env owns all allocations.
        let env: Env = unsafe { core::mem::transmute(env) };
        let multisig: SmartMultiSigClient<'static> = unsafe { core::mem::transmute(multisig) };
        let oracle: MockTwapOracleClient<'static> = unsafe { core::mem::transmute(oracle) };

        Self { env, multisig, oracle, token, admin, signer_a, signer_b, signer_c, recipient }
    }
}

// ---------------------------------------------------------------------------
// Test 1 — Happy path: valid 2-of-3 transfer
// ---------------------------------------------------------------------------

#[test]
fn test_valid_transfer_2_of_3() {
    let h = Harness::new_2_of_3();

    // propose_transfer returns u64 (tx_id) directly — no unwrap needed.
    let tx_id = h.multisig.propose_transfer(&h.signer_a, &h.token, &h.recipient, &500i128);
    assert_eq!(tx_id, 1u64);

    h.multisig.approve(&h.signer_b, &tx_id);

    h.multisig.execute_transfer(&h.signer_a, &tx_id, &NORMAL_FEE);

    let tx = h.multisig.get_pending_tx(&tx_id);
    assert!(tx.executed, "tx should be marked executed");

    let bal = TokenClient::new(&h.env, &h.token).balance(&h.recipient);
    assert_eq!(bal, 500i128);
}

// ---------------------------------------------------------------------------
// Test 2 — Policy violation: fee spike blocks transfer
// ---------------------------------------------------------------------------

/// Pass `SPIKED_FEE` (5 000 stroops) which exceeds the policy ceiling (1 000).
/// `execute_transfer` must revert with `FeeTooHigh` before signature work.
///
/// Validates: "Build logic that pauses all outbound transfers if the current
/// ledger base fee exceeds a defined threshold, protecting the DAO treasury."
#[test]
fn test_execute_reverts_on_fee_spike() {
    let h = Harness::new_2_of_3();

    let tx_id = h.multisig.propose_transfer(&h.signer_a, &h.token, &h.recipient, &500i128);
    h.multisig.approve(&h.signer_b, &tx_id);

    // Use try_ to get the Result without panicking.
    // SDK 27: try_xxx → Result<Result<T, PolicyError>, InvokeError>
    // unwrap_err() → Result<PolicyError, InvokeError>
    // Contract-level error arrives as Ok(PolicyError::...) in the inner Result.
    let inner = h
        .multisig
        .try_execute_transfer(&h.signer_a, &tx_id, &SPIKED_FEE)
        .unwrap_err();
    assert_eq!(
        inner,
        Ok(PolicyError::FeeTooHigh),
        "expected FeeTooHigh when submitted fee exceeds policy ceiling"
    );

    let tx = h.multisig.get_pending_tx(&tx_id);
    assert!(!tx.executed, "tx must not be executed after fee policy revert");
}

// ---------------------------------------------------------------------------
// Test 3 — Policy violation: TWAP deviation blocks transfer
// ---------------------------------------------------------------------------

/// Oracle returns 10% above reference (policy allows 5%).
/// `execute_transfer` must revert with `TwapDeviationTooLarge`.
///
/// Validates: "Integrate cross-contract calls to verify an external TWAP
/// oracle, blocking massive asset swaps if market price deviates."
#[test]
fn test_execute_reverts_on_twap_spike() {
    let h = Harness::new_2_of_3();

    let tx_id = h.multisig.propose_transfer(&h.signer_a, &h.token, &h.recipient, &500i128);
    h.multisig.approve(&h.signer_b, &tx_id);

    // Push oracle 10% above reference — exceeds the 5% policy limit.
    h.oracle.set_twap(&1_100_000i128);

    let inner = h
        .multisig
        .try_execute_transfer(&h.signer_a, &tx_id, &NORMAL_FEE)
        .unwrap_err();
    assert_eq!(
        inner,
        Ok(PolicyError::TwapDeviationTooLarge),
        "expected TwapDeviationTooLarge when oracle price deviates by >5%"
    );

    let recipient_bal = TokenClient::new(&h.env, &h.token).balance(&h.recipient);
    assert_eq!(recipient_bal, 0i128, "recipient should get nothing after TWAP revert");

    let tx = h.multisig.get_pending_tx(&tx_id);
    assert!(!tx.executed, "tx must not be executed after TWAP revert");
}

// ---------------------------------------------------------------------------
// Test 4 — Non-signer cannot propose
// ---------------------------------------------------------------------------

#[test]
fn test_propose_requires_signer() {
    let h = Harness::new_2_of_3();
    let stranger = Address::generate(&h.env);

    let inner = h
        .multisig
        .try_propose_transfer(&stranger, &h.token, &h.recipient, &100i128)
        .unwrap_err();
    assert_eq!(inner, Ok(PolicyError::Unauthorized));
}

// ---------------------------------------------------------------------------
// Test 5 — Double-approve is rejected
// ---------------------------------------------------------------------------

#[test]
fn test_approve_once_only() {
    let h = Harness::new_2_of_3();
    let tx_id = h.multisig.propose_transfer(&h.signer_a, &h.token, &h.recipient, &100i128);

    // signer_a already approved during propose_transfer.
    // approve returns (), inner Err type is PolicyError
    let inner = h.multisig.try_approve(&h.signer_a, &tx_id).unwrap_err();
    assert_eq!(inner, Ok(PolicyError::AlreadySigned));
}

// ---------------------------------------------------------------------------
// Test 6 — Threshold not met blocks execution
// ---------------------------------------------------------------------------

#[test]
fn test_threshold_not_met() {
    let h = Harness::new_2_of_3();

    // Only 1 approval (proposer) — needs 2.
    let tx_id = h.multisig.propose_transfer(&h.signer_a, &h.token, &h.recipient, &100i128);

    let inner = h
        .multisig
        .try_execute_transfer(&h.signer_a, &tx_id, &NORMAL_FEE)
        .unwrap_err();
    assert_eq!(inner, Ok(PolicyError::ThresholdNotMet));
}

// ---------------------------------------------------------------------------
// Test 7 — Non-admin cannot update policy
// ---------------------------------------------------------------------------

#[test]
fn test_set_policy_unauthorized() {
    let h = Harness::new_2_of_3();
    let oracle_id = h.multisig.get_policy().twap_oracle;
    let bad_policy = make_policy(&oracle_id, 999_999, 1_000_000, 500);

    // set_policy returns (), inner Err is PolicyError
    let inner = h
        .multisig
        .try_set_policy(&h.signer_a, &bad_policy)
        .unwrap_err();
    assert_eq!(inner, Ok(PolicyError::Unauthorized));
}

// ---------------------------------------------------------------------------
// Test 8 — Fee guard disabled via sentinel allows any submitted fee
// ---------------------------------------------------------------------------

#[test]
fn test_policy_disable_fee_guard() {
    let h = Harness::new_with_policy_params(u32::MAX, 1_000_000, 500);

    let tx_id = h.multisig.propose_transfer(&h.signer_a, &h.token, &h.recipient, &100i128);
    h.multisig.approve(&h.signer_b, &tx_id);

    // Extreme fee — should pass because guard is disabled.
    h.multisig.execute_transfer(&h.signer_a, &tx_id, &1_000_000u32);

    let tx = h.multisig.get_pending_tx(&tx_id);
    assert!(tx.executed);
}

// ---------------------------------------------------------------------------
// Test 9 — TWAP guard disabled via sentinel allows any price
// ---------------------------------------------------------------------------

#[test]
fn test_policy_disable_twap_guard() {
    let h = Harness::new_with_policy_params(1_000, 1_000_000, u32::MAX);

    h.oracle.set_twap(&9_999_999i128);

    let tx_id = h.multisig.propose_transfer(&h.signer_a, &h.token, &h.recipient, &100i128);
    h.multisig.approve(&h.signer_b, &tx_id);

    h.multisig.execute_transfer(&h.signer_a, &tx_id, &NORMAL_FEE);

    let tx = h.multisig.get_pending_tx(&tx_id);
    assert!(tx.executed);
}

// ---------------------------------------------------------------------------
// Test 10 — Admin rotation: old admin loses policy-update rights
// ---------------------------------------------------------------------------

#[test]
fn test_rotate_admin() {
    let h = Harness::new_2_of_3();
    let new_admin = Address::generate(&h.env);
    let oracle_id = h.multisig.get_policy().twap_oracle;

    h.multisig.rotate_admin(&h.admin, &new_admin);

    // Old admin rejected.
    let updated_policy = make_policy(&oracle_id, 2_000, 1_000_000, 500);
    let inner = h
        .multisig
        .try_set_policy(&h.admin, &updated_policy)
        .unwrap_err();
    assert_eq!(
        inner,
        Ok(PolicyError::Unauthorized),
        "old admin must be rejected after rotation"
    );

    // New admin accepted.
    h.multisig.set_policy(&new_admin, &updated_policy);
    let stored = h.multisig.get_policy();
    assert_eq!(stored.max_allowed_fee_stroops, 2_000);
}
