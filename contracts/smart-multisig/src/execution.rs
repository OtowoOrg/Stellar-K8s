//! Execution engine for the smart-multisig wallet.
//!
//! # Responsibility
//!
//! This module is the single choke-point through which every outbound token
//! transfer must pass.  The pipeline is:
//!
//! ```text
//! 1. assert_policy_pre_check()   ← fee guard + TWAP oracle cross-contract call
//!        ↓ (reverts immediately on any violation — no signature work wasted)
//! 2. assert_threshold_met()      ← count collected approvals vs. threshold
//!        ↓
//! 3. dispatch_transfer()         ← token::Client cross-contract transfer
//! ```
//!
//! Step 1 is intentionally ordered **before** step 2 so that a network
//! congestion or oracle-deviation event causes a cheap revert without
//! iterating the signer set.
//!
//! # TWAP oracle interface
//!
//! The oracle contract is expected to expose a single function:
//!
//! ```text
//! fn get_twap(env: Env) -> i128
//! ```
//!
//! The price is assumed to be scaled to 7 decimal places (same as XLM
//! stroops).  The oracle address is stored inside [`ExecutionPolicy`] and
//! validated every time a transfer is executed.

use soroban_sdk::{token::Client as TokenClient, Address, Env, Symbol, Val, Vec};

use crate::policy::{
    assert_fee_within_policy, assert_twap_within_policy, load_policy, load_signatures,
    DataKey, ExecutionPolicy, PendingTx, PolicyError,
};
// ---------------------------------------------------------------------------
// TWAP oracle cross-contract call
// ---------------------------------------------------------------------------

/// Invoke the external TWAP oracle contract and return the current price.
///
/// The oracle must expose `fn get_twap(env: Env) -> i128`.  The call is made
/// with an empty argument list; the oracle derives all state from its own
/// ledger storage.
///
/// Returns [`PolicyError::NotInitialized`] if the oracle call returns an
/// unexpected type (should never happen with a compliant oracle, but
/// defensive coding prevents a bad oracle from bricking the wallet).
pub fn fetch_twap_price(env: &Env, oracle: &Address) -> Result<i128, PolicyError> {
    let args: Vec<Val> = Vec::new(env);
    let price: i128 = env.invoke_contract(oracle, &Symbol::new(env, "get_twap"), args);
    Ok(price)
}

// ---------------------------------------------------------------------------
// Policy pre-check  (must run BEFORE signature loop)
// ---------------------------------------------------------------------------

/// Run all [`ExecutionPolicy`] guards before touching the signer set.
///
/// Order of checks:
/// 1. **Fee guard** — compares `submitted_fee_stroops` against the policy
///    ceiling.  This is a pure comparison with no cross-contract calls, so
///    it fails cheapest.
/// 2. **TWAP guard** — cross-contract call to `policy.twap_oracle`.
///
/// Both checks must pass or the function returns an error immediately,
/// saving the CPU cost of signature iteration on a doomed transaction.
///
/// # Arguments
///
/// * `submitted_fee_stroops` — the current network base fee (in stroops) as
///   reported by the transaction submitter.
///
/// # Errors
///
/// * [`PolicyError::FeeTooHigh`] — submitted fee exceeds policy ceiling.
/// * [`PolicyError::TwapDeviationTooLarge`] — oracle price deviates beyond
///   the policy's `max_price_deviation_bps` from the stored reference.
/// * [`PolicyError::NotInitialized`] — policy has not been set.
pub fn assert_policy_pre_check(
    env: &Env,
    submitted_fee_stroops: u32,
) -> Result<ExecutionPolicy, PolicyError> {
    let policy = load_policy(env)?;

    // 1. Fee guard — pure comparison, no I/O, cheapest check first.
    assert_fee_within_policy(submitted_fee_stroops, &policy)?;

    // 2. TWAP guard — cross-contract call to external oracle.
    let live_price = fetch_twap_price(env, &policy.twap_oracle)?;
    assert_twap_within_policy(&policy, live_price)?;

    Ok(policy)
}

// ---------------------------------------------------------------------------
// Signature threshold enforcement
// ---------------------------------------------------------------------------

/// Verify that the number of collected approvals for `tx` meets the wallet's
/// configured signature threshold.
///
/// # Arguments
///
/// * `signers`   — the wallet's authorised signer set (from Instance Storage).
/// * `threshold` — minimum approval count required.
/// * `tx`        — the pending transaction whose approval list is checked.
///
/// # Errors
///
/// * [`PolicyError::ThresholdNotMet`] — fewer valid approvals than `threshold`.
pub fn assert_threshold_met(
    env: &Env,
    signers: &soroban_sdk::Vec<Address>,
    threshold: u32,
    tx: &PendingTx,
) -> Result<(), PolicyError> {
    let collected = load_signatures(env, tx.id);

    // Count only approvals from addresses that are still in the signer set.
    // This handles the edge case where a signer was removed after approving.
    let valid_count = collected
        .iter()
        .filter(|addr| signers.contains(addr))
        .count() as u32;

    if valid_count < threshold {
        return Err(PolicyError::ThresholdNotMet);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Transfer dispatch
// ---------------------------------------------------------------------------

/// Execute the token transfer encoded in `tx`.
///
/// Uses `token::Client` (the canonical SEP-41 / Stellar token interface) to
/// transfer `tx.amount` of `tx.token` from the multisig contract's own
/// address to `tx.to`.
///
/// # Errors
///
/// * [`PolicyError::InvalidAmount`] — `tx.amount` is zero or negative.
///
/// Any failure inside the token contract itself will propagate as a panic
/// (the Soroban host unwinds the whole transaction), which is the correct
/// behaviour: a failed transfer should never silently mark the tx as executed.
pub fn dispatch_transfer(env: &Env, tx: &PendingTx) -> Result<(), PolicyError> {
    if tx.amount <= 0 {
        return Err(PolicyError::InvalidAmount);
    }
    TokenClient::new(env, &tx.token).transfer(
        &env.current_contract_address(),
        &tx.to,
        &tx.amount,
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Signer helpers
// ---------------------------------------------------------------------------

/// Return `true` if `addr` is in the wallet's signer set.
pub fn is_signer(signers: &soroban_sdk::Vec<Address>, addr: &Address) -> bool {
    signers.contains(addr)
}

/// Record a signer's approval for `tx_id`.
///
/// Appends `signer` to the approval list stored under [`DataKey::Signatures`].
/// The caller must have already verified the signer is authorised and has not
/// already signed.
pub fn record_approval(env: &Env, tx_id: u64, signer: &Address) {
    let mut sigs = crate::policy::load_signatures(env, tx_id);
    sigs.push_back(signer.clone());
    crate::policy::save_signatures(env, tx_id, &sigs);
}

/// Return `true` if `signer` has already approved `tx_id`.
pub fn has_approved(env: &Env, tx_id: u64, signer: &Address) -> bool {
    let sigs = crate::policy::load_signatures(env, tx_id);
    sigs.contains(signer)
}

// ---------------------------------------------------------------------------
// Admin / signer-set helpers
// ---------------------------------------------------------------------------

/// Load the admin address from Instance Storage.
pub fn load_admin(env: &Env) -> Result<Address, PolicyError> {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(PolicyError::NotInitialized)
}

/// Load the signer set from Instance Storage.
pub fn load_signers(env: &Env) -> Result<soroban_sdk::Vec<Address>, PolicyError> {
    env.storage()
        .instance()
        .get(&DataKey::Signers)
        .ok_or(PolicyError::NotInitialized)
}

/// Load the signature threshold from Instance Storage.
pub fn load_threshold(env: &Env) -> Result<u32, PolicyError> {
    env.storage()
        .instance()
        .get(&DataKey::Threshold)
        .ok_or(PolicyError::NotInitialized)
}

/// Increment and return the next pending-tx counter.
pub fn next_tx_id(env: &Env) -> u64 {
    let current: u64 = env
        .storage()
        .instance()
        .get(&DataKey::TxCounter)
        .unwrap_or(0u64);
    let next = current.saturating_add(1);
    env.storage()
        .instance()
        .set(&DataKey::TxCounter, &next);
    next
}
