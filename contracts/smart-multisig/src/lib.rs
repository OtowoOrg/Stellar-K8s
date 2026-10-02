//! # Smart Multi-Sig Wallet with Programmable On-Chain ExecutionPolicy
//!
//! A Soroban smart contract that extends a standard M-of-N multi-signature
//! wallet with **dynamic, state-verifying execution rules** evaluated against
//! real-time network parameters before any signature validation occurs.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────┐
//! │                  SmartMultiSig                       │
//! │                                                     │
//! │  Instance Storage                                   │
//! │  ├── Admin          Address                         │
//! │  ├── Signers        Vec<Address>                    │
//! │  ├── Threshold      u32                             │
//! │  ├── Policy         ExecutionPolicy  ◄──────────┐  │
//! │  ├── TxCounter      u64                          │  │
//! │  ├── PendingTx(id)  PendingTx                    │  │
//! │  └── Signatures(id) Vec<Address>                 │  │
//! │                                                  │  │
//! │  execute_transfer pipeline:                      │  │
//! │  1. assert_policy_pre_check()  ←─ fee guard ─────┘  │
//! │                                ←─ TWAP oracle ──────►│ External oracle
//! │  2. assert_threshold_met()                          │
//! │  3. dispatch_transfer()  ──────────────────────────►│ Token contract
//! └─────────────────────────────────────────────────────┘
//! ```
//!
//! ## Public interface
//!
//! | Function              | Who can call      | Description                                  |
//! |-----------------------|-------------------|----------------------------------------------|
//! | `__constructor`       | deployer (once)   | Initialise signers, threshold, policy        |
//! | `propose_transfer`    | any signer        | Create a pending outbound transfer           |
//! | `approve`             | any signer        | Add an approval to a pending transfer        |
//! | `execute_transfer`    | any signer        | Execute once threshold & policy pass         |
//! | `set_policy`          | admin only        | Update the ExecutionPolicy                   |
//! | `get_policy`          | anyone            | Read the current ExecutionPolicy             |
//! | `get_pending_tx`      | anyone            | Inspect a pending transaction                |
//! | `get_signers`         | anyone            | Return the current signer set                |
//! | `get_threshold`       | anyone            | Return the current threshold                 |
//! | `rotate_admin`        | admin only        | Transfer admin rights                        |
//!
//! ## Fee guard design note
//!
//! The Soroban SDK 27 host does not expose the ledger base fee inside WASM.
//! The submitting signer passes `current_fee_stroops` (the network base fee
//! they observed) into `execute_transfer`.  The contract compares this value
//! against `policy.max_allowed_fee_stroops`.  This is the canonical approach
//! for fee-aware Soroban contracts on Stellar.

#![no_std]

pub mod execution;
pub mod policy;

#[cfg(test)]
mod test;

use execution::{
    assert_policy_pre_check, assert_threshold_met, dispatch_transfer, has_approved, is_signer,
    load_admin, load_signers, load_threshold, next_tx_id, record_approval,
};
use policy::{
    load_pending_tx, refresh_ttl, save_pending_tx, save_policy, DataKey, ExecutionPolicy,
    PendingTx, PolicyError,
};

use soroban_sdk::{contract, contractevent, contractimpl, Address, Env, Vec};

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Emitted when a new pending transfer is proposed.
#[contractevent(topics = ["multisig", "proposed"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransferProposed {
    pub tx_id: u64,
    pub proposer: Address,
    pub token: Address,
    pub to: Address,
    pub amount: i128,
}

/// Emitted when a signer approves a pending transfer.
#[contractevent(topics = ["multisig", "approved"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransferApproved {
    pub tx_id: u64,
    pub signer: Address,
}

/// Emitted when a pending transfer is executed successfully.
#[contractevent(topics = ["multisig", "executed"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransferExecuted {
    pub tx_id: u64,
    pub token: Address,
    pub to: Address,
    pub amount: i128,
}

/// Emitted when the ExecutionPolicy is updated.
#[contractevent(topics = ["multisig", "policy"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyUpdated {
    pub max_allowed_fee_stroops: u32,
    pub max_price_deviation_bps: u32,
}

// ---------------------------------------------------------------------------
// Contract declaration
// ---------------------------------------------------------------------------

#[contract]
pub struct SmartMultiSig;

// ---------------------------------------------------------------------------
// Contract implementation
// ---------------------------------------------------------------------------

#[contractimpl]
impl SmartMultiSig {
    // -----------------------------------------------------------------------
    // Construction
    // -----------------------------------------------------------------------

    /// Initialise the smart-multisig wallet.
    ///
    /// Called atomically at deploy time via `__constructor` so that the
    /// admin/signer assignment cannot be front-run.
    ///
    /// # Arguments
    ///
    /// * `admin`     — Address that can update the `ExecutionPolicy` and rotate itself.
    /// * `signers`   — Initial set of authorised signers (must be non-empty).
    /// * `threshold` — Minimum approvals required to execute a transfer (1 ≤ threshold ≤ len(signers)).
    /// * `policy`    — Initial `ExecutionPolicy` governing execution conditions.
    pub fn __constructor(
        env: Env,
        admin: Address,
        signers: Vec<Address>,
        threshold: u32,
        policy: ExecutionPolicy,
    ) -> Result<(), PolicyError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(PolicyError::AlreadyInitialized);
        }
        if signers.is_empty() {
            return Err(PolicyError::EmptySignerSet);
        }
        if threshold == 0 || threshold > signers.len() {
            return Err(PolicyError::InvalidThreshold);
        }

        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Signers, &signers);
        env.storage().instance().set(&DataKey::Threshold, &threshold);
        env.storage().instance().set(&DataKey::TxCounter, &0u64);
        save_policy(&env, &policy);
        refresh_ttl(&env);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Transfer lifecycle
    // -----------------------------------------------------------------------

    /// Propose a new outbound token transfer.
    ///
    /// Any authorised signer may propose a transfer.  The proposal is stored as
    /// a [`PendingTx`] in Instance Storage and automatically counts as the
    /// proposer's own approval.
    ///
    /// Returns the unique `tx_id` assigned to this pending transaction.
    pub fn propose_transfer(
        env: Env,
        proposer: Address,
        token: Address,
        to: Address,
        amount: i128,
    ) -> Result<u64, PolicyError> {
        proposer.require_auth();

        let signers = load_signers(&env)?;
        if !is_signer(&signers, &proposer) {
            return Err(PolicyError::Unauthorized);
        }
        if amount <= 0 {
            return Err(PolicyError::InvalidAmount);
        }

        let tx_id = next_tx_id(&env);
        let tx = PendingTx {
            id: tx_id,
            token: token.clone(),
            to: to.clone(),
            amount,
            proposer: proposer.clone(),
            executed: false,
        };
        save_pending_tx(&env, &tx);

        // The proposer's own signature is counted immediately.
        record_approval(&env, tx_id, &proposer);

        TransferProposed {
            tx_id,
            proposer,
            token,
            to,
            amount,
        }
        .publish(&env);

        refresh_ttl(&env);
        Ok(tx_id)
    }

    /// Approve a pending transfer.
    ///
    /// Any authorised signer that has not yet approved may add their approval.
    pub fn approve(env: Env, signer: Address, tx_id: u64) -> Result<(), PolicyError> {
        signer.require_auth();

        let signers = load_signers(&env)?;
        if !is_signer(&signers, &signer) {
            return Err(PolicyError::Unauthorized);
        }

        let tx = load_pending_tx(&env, tx_id)?;
        if tx.executed {
            return Err(PolicyError::TxAlreadyExecuted);
        }
        if has_approved(&env, tx_id, &signer) {
            return Err(PolicyError::AlreadySigned);
        }

        record_approval(&env, tx_id, &signer);

        TransferApproved { tx_id, signer }.publish(&env);

        refresh_ttl(&env);
        Ok(())
    }

    /// Execute a pending transfer after all execution policy guards pass and
    /// the signature threshold is met.
    ///
    /// # Execution order
    ///
    /// 1. **Policy pre-check** — fee guard (using `current_fee_stroops`) +
    ///    TWAP oracle cross-contract call.  Reverts immediately on violation.
    /// 2. **Threshold check** — count valid approvals vs. configured threshold.
    /// 3. **Transfer dispatch** — `token::Client` cross-contract transfer.
    ///
    /// # Arguments
    ///
    /// * `caller`              — Any authorised signer triggering execution.
    /// * `tx_id`               — ID of the pending transaction to execute.
    /// * `current_fee_stroops` — The current network base fee (stroops) as
    ///   observed by the submitting relayer.  Compared against the policy
    ///   ceiling to guard against gas-spike depletion.
    pub fn execute_transfer(
        env: Env,
        caller: Address,
        tx_id: u64,
        current_fee_stroops: u32,
    ) -> Result<(), PolicyError> {
        caller.require_auth();

        let signers = load_signers(&env)?;
        if !is_signer(&signers, &caller) {
            return Err(PolicyError::Unauthorized);
        }

        let mut tx = load_pending_tx(&env, tx_id)?;
        if tx.executed {
            return Err(PolicyError::TxAlreadyExecuted);
        }

        // ----------------------------------------------------------------
        // STEP 1: Policy pre-check — runs BEFORE signature loop.
        // Any network-condition violation reverts here cheaply.
        // ----------------------------------------------------------------
        assert_policy_pre_check(&env, current_fee_stroops)?;

        // ----------------------------------------------------------------
        // STEP 2: Signature threshold check.
        // ----------------------------------------------------------------
        let threshold = load_threshold(&env)?;
        assert_threshold_met(&env, &signers, threshold, &tx)?;

        // ----------------------------------------------------------------
        // STEP 3: Mark as executed before external call (checks-effects-
        // interactions pattern) to prevent re-entrancy.
        // ----------------------------------------------------------------
        tx.executed = true;
        save_pending_tx(&env, &tx);

        // ----------------------------------------------------------------
        // STEP 4: Dispatch the token transfer.
        // ----------------------------------------------------------------
        dispatch_transfer(&env, &tx)?;

        TransferExecuted {
            tx_id,
            token: tx.token.clone(),
            to: tx.to.clone(),
            amount: tx.amount,
        }
        .publish(&env);

        refresh_ttl(&env);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Policy management (admin only)
    // -----------------------------------------------------------------------

    /// Replace the current `ExecutionPolicy`.
    ///
    /// Only the `admin` may update the policy.  The admin must `require_auth`.
    ///
    /// **Lockout prevention guarantee**: policy guards can always be disabled
    /// by the admin:
    /// - Set `max_allowed_fee_stroops = u32::MAX` to disable the fee guard.
    /// - Set `max_price_deviation_bps = u32::MAX` to disable the TWAP guard.
    /// This ensures the admin can never be permanently locked out by a
    /// misconfigured policy.
    pub fn set_policy(
        env: Env,
        caller: Address,
        new_policy: ExecutionPolicy,
    ) -> Result<(), PolicyError> {
        caller.require_auth();

        let admin = load_admin(&env)?;
        if caller != admin {
            return Err(PolicyError::Unauthorized);
        }

        PolicyUpdated {
            max_allowed_fee_stroops: new_policy.max_allowed_fee_stroops,
            max_price_deviation_bps: new_policy.max_price_deviation_bps,
        }
        .publish(&env);

        save_policy(&env, &new_policy);
        refresh_ttl(&env);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Admin rotation
    // -----------------------------------------------------------------------

    /// Transfer admin rights to `new_admin`.
    ///
    /// The existing admin must authorise this call.
    pub fn rotate_admin(env: Env, caller: Address, new_admin: Address) -> Result<(), PolicyError> {
        caller.require_auth();

        let admin = load_admin(&env)?;
        if caller != admin {
            return Err(PolicyError::Unauthorized);
        }

        env.storage().instance().set(&DataKey::Admin, &new_admin);
        refresh_ttl(&env);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Read-only views
    // -----------------------------------------------------------------------

    /// Return the current `ExecutionPolicy`.
    pub fn get_policy(env: Env) -> Result<ExecutionPolicy, PolicyError> {
        policy::load_policy(&env)
    }

    /// Return a pending transaction by ID.
    pub fn get_pending_tx(env: Env, tx_id: u64) -> Result<PendingTx, PolicyError> {
        load_pending_tx(&env, tx_id)
    }

    /// Return the current authorised signer set.
    pub fn get_signers(env: Env) -> Result<Vec<Address>, PolicyError> {
        load_signers(&env)
    }

    /// Return the current signature threshold.
    pub fn get_threshold(env: Env) -> Result<u32, PolicyError> {
        load_threshold(&env)
    }

    /// Return the current admin address.
    pub fn get_admin(env: Env) -> Result<Address, PolicyError> {
        load_admin(&env)
    }

    /// Return the number of valid approvals collected for `tx_id`.
    pub fn approval_count(env: Env, tx_id: u64) -> u32 {
        let signers = load_signers(&env).unwrap_or_else(|_| Vec::new(&env));
        let sigs = policy::load_signatures(&env, tx_id);
        sigs.iter()
            .filter(|addr| signers.contains(addr))
            .count() as u32
    }
}
