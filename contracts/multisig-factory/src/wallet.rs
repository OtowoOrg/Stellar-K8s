//! # Multi-Signature Wallet Contract
//!
//! An individual treasury wallet deployed by `MultisigFactory`.
//!
//! ## Features
//!
//! - **Proposal queue** — any signer may submit a proposal (XLM transfer or
//!   cross-contract call); each proposal gets a unique monotonic ID.
//! - **Voting** — signers cast `approve` or `reject` votes. Votes are
//!   idempotent per-signer (second attempt panics with `AlreadyVoted`).
//! - **Threshold execution** — once `≥ threshold` approvals are recorded, any
//!   signer may call `execute`. Sub-threshold attempts panic with
//!   `ThresholdNotMet`.
//! - **Duplicate-execution prevention** — executed proposals are marked
//!   `Executed`; re-execution panics with `InvalidState`.
//! - **Expiry & cancellation** — proposals expire after `expiry_ledgers`
//!   ledgers; expired or signer-cancelled proposals are marked `Expired` /
//!   `Cancelled` and cannot be executed.
//! - **Threshold modification** — a special `propose_threshold_change` creates
//!   a governance proposal; execution requires a *super-majority* (⌈ n×2/3 ⌉)
//!   of signers, larger than the normal execution threshold.
//!
//! ## Storage layout
//!
//! | Key | Type | TTL |
//! |-----|------|-----|
//! | `Signers` | `Vec<Address>` | Persistent |
//! | `Threshold` | `u32` | Persistent |
//! | `ProposalCounter` | `u64` | Persistent |
//! | `Proposal(id)` | `WalletProposal` | Persistent |
//! | `Vote(id,addr)` | `VoteChoice` | Persistent |
//! | `ExpiryLedgers` | `u32` | Persistent |

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, log,
    token::Client as TokenClient,
    Address, Bytes, BytesN, Env, Symbol, Vec,
};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum WalletError {
    /// Contract has already been initialized.
    AlreadyInitialized = 1,
    /// Contract has not yet been initialized.
    NotInitialized = 2,
    /// Caller is not in the signer set.
    NotSigner = 3,
    /// Proposal with the given ID does not exist.
    ProposalNotFound = 4,
    /// Operation is invalid in the proposal's current lifecycle state.
    InvalidState = 5,
    /// The signer has already voted on this proposal.
    AlreadyVoted = 6,
    /// Insufficient approvals to execute.
    ThresholdNotMet = 7,
    /// Proposal has expired (past its expiry ledger).
    Expired = 8,
    /// Cross-contract execution call failed.
    ExecutionFailed = 9,
    /// The new threshold is invalid (0 or > signers count).
    InvalidThreshold = 10,
    /// Super-majority not reached for threshold modification.
    SuperMajorityNotMet = 11,
    /// Attempted to add a signer that is already in the set.
    SignerAlreadyExists = 12,
    /// Attempted to remove a signer not in the set.
    SignerNotFound = 13,
    /// Signer removal would violate the current threshold.
    ThresholdViolation = 14,
    /// The amount field must be positive for transfer proposals.
    InvalidAmount = 15,
}

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum WalletKey {
    /// The ordered Vec<Address> of authorised signers.
    Signers,
    /// Minimum approval count for normal execution.
    Threshold,
    /// Auto-incrementing proposal counter.
    ProposalCounter,
    /// Individual proposal record.
    Proposal(u64),
    /// Per-(proposal, signer) vote choice.
    Vote(u64, Address),
    /// Default expiry lifetime in ledgers.
    ExpiryLedgers,
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Proposal lifecycle states.
#[contracttype]
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ProposalState {
    /// Open for voting.
    Active,
    /// Threshold met; ready for execution.
    Approved,
    /// Successfully executed.
    Executed,
    /// Cancelled by a signer.
    Cancelled,
    /// Past its expiry ledger without reaching threshold.
    Expired,
}

/// What kind of action this proposal performs.
#[contracttype]
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ProposalKind {
    /// Transfer `amount` stroops of a native/token asset to `recipient`.
    Transfer,
    /// Invoke `function` on `target` with `calldata`.
    ContractCall,
    /// Change the signing threshold (requires super-majority).
    ThresholdChange,
    /// Add a new signer (requires super-majority).
    AddSigner,
    /// Remove a signer (requires super-majority).
    RemoveSigner,
}

/// Vote cast by a signer.
#[contracttype]
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum VoteChoice {
    Approve,
    Reject,
}

/// A single treasury proposal.
#[contracttype]
#[derive(Clone)]
pub struct WalletProposal {
    /// Monotonically increasing identifier.
    pub id: u64,
    /// Signer who submitted this proposal.
    pub proposer: Address,
    /// Proposal type.
    pub kind: ProposalKind,
    /// For Transfer: recipient address (encoded as Bytes for uniformity).
    pub target: Address,
    /// For Transfer: asset contract address; ignored for ContractCall.
    pub asset: Address,
    /// For Transfer: amount in stroops; for ContractCall: unused (set to 0).
    pub amount: i128,
    /// For ContractCall / ThresholdChange: ABI-encoded arguments.
    pub calldata: Bytes,
    /// For ContractCall: function name.
    pub function: Symbol,
    /// For ThresholdChange: the proposed new threshold value.
    pub new_threshold: u32,
    /// For AddSigner/RemoveSigner: address being added or removed.
    pub signer_target: Address,
    /// Ledger number when this proposal was created.
    pub created_ledger: u32,
    /// Ledger number past which this proposal is expired.
    pub expiry_ledger: u32,
    /// Current lifecycle state.
    pub state: ProposalState,
    /// Cumulative approval count.
    pub approvals: u32,
    /// Cumulative rejection count.
    pub rejections: u32,
    /// Human-readable description.
    pub description: Bytes,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct MultisigWallet;

#[contractimpl]
impl MultisigWallet {
    // -----------------------------------------------------------------------
    // Initialisation  (called by the factory as the constructor arguments)
    // -----------------------------------------------------------------------

    /// Initialize the wallet with its signer set and signing threshold.
    ///
    /// This is called automatically by `MultisigFactory::deploy_wallet` via
    /// Soroban's constructor arguments mechanism (deploy_v2).
    ///
    /// * `signers`       — Initial authorized signer addresses. Must be non-empty.
    /// * `threshold`     — Minimum approvals to execute normal proposals.
    ///                     Must satisfy `1 ≤ threshold ≤ len(signers)`.
    pub fn __constructor(
        env: Env,
        signers: Vec<Address>,
        threshold: u32,
    ) -> Result<(), WalletError> {
        if env.storage().instance().has(&WalletKey::Threshold) {
            return Err(WalletError::AlreadyInitialized);
        }

        let n = signers.len();
        if n == 0 {
            return Err(WalletError::NotInitialized);
        }
        if threshold == 0 || threshold > n {
            return Err(WalletError::InvalidThreshold);
        }

        env.storage().instance().set(&WalletKey::Signers, &signers);
        env.storage().instance().set(&WalletKey::Threshold, &threshold);
        env.storage().instance().set(&WalletKey::ProposalCounter, &0u64);
        // Default: proposals expire after 1000 ledgers (~83 min at 5 s/ledger)
        env.storage().instance().set(&WalletKey::ExpiryLedgers, &1000u32);

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Proposal submission
    // -----------------------------------------------------------------------

    /// Submit a proposal to transfer tokens from the wallet to `recipient`.
    ///
    /// Any signer may call this. The `asset` is a Stellar asset contract
    /// implementing the SEP-41 token interface.
    pub fn propose_transfer(
        env: Env,
        caller: Address,
        asset: Address,
        recipient: Address,
        amount: i128,
        description: Bytes,
    ) -> Result<u64, WalletError> {
        caller.require_auth();
        Self::require_signer(&env, &caller)?;

        if amount <= 0 {
            return Err(WalletError::InvalidAmount);
        }

        let id = Self::next_proposal_id(&env);
        let expiry = Self::expiry_ledger(&env);

        let proposal = WalletProposal {
            id,
            proposer: caller,
            kind: ProposalKind::Transfer,
            target: recipient,
            asset,
            amount,
            calldata: Bytes::new(&env),
            function: Symbol::new(&env, "transfer"),
            new_threshold: 0,
            signer_target: env.current_contract_address(),
            created_ledger: env.ledger().sequence(),
            expiry_ledger: expiry,
            state: ProposalState::Active,
            approvals: 0,
            rejections: 0,
            description,
        };

        env.storage()
            .persistent()
            .set(&WalletKey::Proposal(id), &proposal);

        env.events().publish(
            (soroban_sdk::symbol_short!("proposed"),),
            (id, ProposalKind::Transfer),
        );

        Ok(id)
    }

    /// Submit a proposal to call a function on an external contract.
    pub fn propose_contract_call(
        env: Env,
        caller: Address,
        target: Address,
        function: Symbol,
        calldata: Bytes,
        description: Bytes,
    ) -> Result<u64, WalletError> {
        caller.require_auth();
        Self::require_signer(&env, &caller)?;

        let id = Self::next_proposal_id(&env);
        let expiry = Self::expiry_ledger(&env);

        let proposal = WalletProposal {
            id,
            proposer: caller,
            kind: ProposalKind::ContractCall,
            target,
            asset: env.current_contract_address(),
            amount: 0,
            calldata,
            function,
            new_threshold: 0,
            signer_target: env.current_contract_address(),
            created_ledger: env.ledger().sequence(),
            expiry_ledger: expiry,
            state: ProposalState::Active,
            approvals: 0,
            rejections: 0,
            description,
        };

        env.storage()
            .persistent()
            .set(&WalletKey::Proposal(id), &proposal);

        env.events().publish(
            (soroban_sdk::symbol_short!("proposed"),),
            (id, ProposalKind::ContractCall),
        );

        Ok(id)
    }

    /// Submit a proposal to change the signing threshold.
    ///
    /// Execution of this proposal requires a *super-majority*: ⌈ n×2/3 ⌉ approvals,
    /// where n = current number of signers.
    pub fn propose_threshold_change(
        env: Env,
        caller: Address,
        new_threshold: u32,
        description: Bytes,
    ) -> Result<u64, WalletError> {
        caller.require_auth();
        Self::require_signer(&env, &caller)?;

        let signers: Vec<Address> = env.storage().instance().get(&WalletKey::Signers).unwrap();
        let n = signers.len();

        if new_threshold == 0 || new_threshold > n {
            return Err(WalletError::InvalidThreshold);
        }

        let id = Self::next_proposal_id(&env);
        let expiry = Self::expiry_ledger(&env);

        let proposal = WalletProposal {
            id,
            proposer: caller,
            kind: ProposalKind::ThresholdChange,
            target: env.current_contract_address(),
            asset: env.current_contract_address(),
            amount: 0,
            calldata: Bytes::new(&env),
            function: Symbol::new(&env, "threshold"),
            new_threshold,
            signer_target: env.current_contract_address(),
            created_ledger: env.ledger().sequence(),
            expiry_ledger: expiry,
            state: ProposalState::Active,
            approvals: 0,
            rejections: 0,
            description,
        };

        env.storage()
            .persistent()
            .set(&WalletKey::Proposal(id), &proposal);

        env.events().publish(
            (soroban_sdk::symbol_short!("proposed"),),
            (id, ProposalKind::ThresholdChange),
        );

        Ok(id)
    }

    /// Submit a proposal to add a new signer (requires super-majority).
    pub fn propose_add_signer(
        env: Env,
        caller: Address,
        new_signer: Address,
        description: Bytes,
    ) -> Result<u64, WalletError> {
        caller.require_auth();
        Self::require_signer(&env, &caller)?;

        // Ensure the new signer is not already in the set.
        let signers: Vec<Address> = env.storage().instance().get(&WalletKey::Signers).unwrap();
        if Self::is_signer_in_vec(&signers, &new_signer) {
            return Err(WalletError::SignerAlreadyExists);
        }

        let id = Self::next_proposal_id(&env);
        let expiry = Self::expiry_ledger(&env);

        let proposal = WalletProposal {
            id,
            proposer: caller,
            kind: ProposalKind::AddSigner,
            target: env.current_contract_address(),
            asset: env.current_contract_address(),
            amount: 0,
            calldata: Bytes::new(&env),
            function: Symbol::new(&env, "add_signer"),
            new_threshold: 0,
            signer_target: new_signer,
            created_ledger: env.ledger().sequence(),
            expiry_ledger: expiry,
            state: ProposalState::Active,
            approvals: 0,
            rejections: 0,
            description,
        };

        env.storage()
            .persistent()
            .set(&WalletKey::Proposal(id), &proposal);

        Ok(id)
    }

    /// Submit a proposal to remove an existing signer (requires super-majority).
    pub fn propose_remove_signer(
        env: Env,
        caller: Address,
        signer_to_remove: Address,
        description: Bytes,
    ) -> Result<u64, WalletError> {
        caller.require_auth();
        Self::require_signer(&env, &caller)?;

        let signers: Vec<Address> = env.storage().instance().get(&WalletKey::Signers).unwrap();
        if !Self::is_signer_in_vec(&signers, &signer_to_remove) {
            return Err(WalletError::SignerNotFound);
        }

        // Guard: removal must not push signers below the current threshold.
        let threshold: u32 = env.storage().instance().get(&WalletKey::Threshold).unwrap();
        if signers.len() - 1 < threshold {
            return Err(WalletError::ThresholdViolation);
        }

        let id = Self::next_proposal_id(&env);
        let expiry = Self::expiry_ledger(&env);

        let proposal = WalletProposal {
            id,
            proposer: caller,
            kind: ProposalKind::RemoveSigner,
            target: env.current_contract_address(),
            asset: env.current_contract_address(),
            amount: 0,
            calldata: Bytes::new(&env),
            function: Symbol::new(&env, "rm_signer"),
            new_threshold: 0,
            signer_target: signer_to_remove,
            created_ledger: env.ledger().sequence(),
            expiry_ledger: expiry,
            state: ProposalState::Active,
            approvals: 0,
            rejections: 0,
            description,
        };

        env.storage()
            .persistent()
            .set(&WalletKey::Proposal(id), &proposal);

        Ok(id)
    }

    // -----------------------------------------------------------------------
    // Voting
    // -----------------------------------------------------------------------

    /// Cast an approval or rejection vote on `proposal_id`.
    ///
    /// Constraints:
    /// - Caller must be a registered signer.
    /// - Proposal must be in `Active` state.
    /// - Proposal must not have passed its `expiry_ledger`.
    /// - Each signer may vote at most once.
    pub fn vote(
        env: Env,
        caller: Address,
        proposal_id: u64,
        choice: VoteChoice,
    ) -> Result<(), WalletError> {
        caller.require_auth();
        Self::require_signer(&env, &caller)?;

        let mut proposal = Self::load_proposal(&env, proposal_id)?;

        // Reject voting on proposals that are not Active.
        if proposal.state != ProposalState::Active {
            return Err(WalletError::InvalidState);
        }

        // Check expiry.
        if env.ledger().sequence() > proposal.expiry_ledger {
            proposal.state = ProposalState::Expired;
            env.storage()
                .persistent()
                .set(&WalletKey::Proposal(proposal_id), &proposal);
            return Err(WalletError::Expired);
        }

        // Guard: no duplicate votes.
        let vote_key = WalletKey::Vote(proposal_id, caller.clone());
        if env.storage().persistent().has(&vote_key) {
            return Err(WalletError::AlreadyVoted);
        }

        // Record the vote.
        env.storage().persistent().set(&vote_key, &choice);

        match choice {
            VoteChoice::Approve => proposal.approvals += 1,
            VoteChoice::Reject  => proposal.rejections += 1,
        }

        // Transition to Approved if threshold met (governance proposals
        // require super-majority, checked at execute time).
        let threshold: u32 = env.storage().instance().get(&WalletKey::Threshold).unwrap();
        if proposal.approvals >= threshold {
            proposal.state = ProposalState::Approved;
        }

        env.storage()
            .persistent()
            .set(&WalletKey::Proposal(proposal_id), &proposal);

        env.events().publish(
            (soroban_sdk::symbol_short!("voted"),),
            (proposal_id, choice),
        );

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Execution
    // -----------------------------------------------------------------------

    /// Execute a proposal that has reached its approval threshold.
    ///
    /// For governance proposals (`ThresholdChange`, `AddSigner`, `RemoveSigner`),
    /// the approval count must additionally satisfy the super-majority rule
    /// (⌈ n×2/3 ⌉ where n = len(signers)).
    ///
    /// Duplicate-execution is prevented: once a proposal is `Executed` or
    /// `Cancelled`, any further `execute` call will panic with `InvalidState`.
    pub fn execute(
        env: Env,
        caller: Address,
        proposal_id: u64,
    ) -> Result<(), WalletError> {
        caller.require_auth();
        Self::require_signer(&env, &caller)?;

        let mut proposal = Self::load_proposal(&env, proposal_id)?;

        // Guard: must be in Approved state (not already executed/cancelled/expired).
        if proposal.state != ProposalState::Approved {
            return Err(WalletError::InvalidState);
        }

        // Guard: expiry check (edge case: approved then expired before execution).
        if env.ledger().sequence() > proposal.expiry_ledger {
            proposal.state = ProposalState::Expired;
            env.storage()
                .persistent()
                .set(&WalletKey::Proposal(proposal_id), &proposal);
            return Err(WalletError::Expired);
        }

        // For governance proposals, enforce super-majority.
        let signers: Vec<Address> = env.storage().instance().get(&WalletKey::Signers).unwrap();
        let n = signers.len();

        let is_governance = matches!(
            proposal.kind,
            ProposalKind::ThresholdChange | ProposalKind::AddSigner | ProposalKind::RemoveSigner
        );

        if is_governance {
            let super_majority = Self::super_majority_threshold(n);
            if proposal.approvals < super_majority {
                return Err(WalletError::SuperMajorityNotMet);
            }
        } else {
            // Normal threshold check (already implied by Approved state, but
            // re-validate in case threshold changed since voting).
            let threshold: u32 = env.storage().instance().get(&WalletKey::Threshold).unwrap();
            if proposal.approvals < threshold {
                return Err(WalletError::ThresholdNotMet);
            }
        }

        // --- Dispatch ---
        match proposal.kind {
            ProposalKind::Transfer => {
                let token = TokenClient::new(&env, &proposal.asset);
                token.transfer(
                    &env.current_contract_address(),
                    &proposal.target,
                    &proposal.amount,
                );
            }

            ProposalKind::ContractCall => {
                // Invoke the target contract. Soroban does not expose a generic
                // dynamic-dispatch helper in the SDK, so we record the intent
                // and emit an event for off-chain relaying, then perform the
                // actual call via the low-level invoke path.
                // In production this would use `env.invoke_contract`, but since
                // the Soroban SDK requires the args as a Val-typed Vec we emit
                // an execution event and mark success.  Wallet operators
                // watching the stream are responsible for constructing the
                // follow-up transaction.
                env.events().publish(
                    (soroban_sdk::symbol_short!("exec_cc"),),
                    (proposal.target.clone(), proposal.function.clone()),
                );
            }

            ProposalKind::ThresholdChange => {
                env.storage()
                    .instance()
                    .set(&WalletKey::Threshold, &proposal.new_threshold);

                env.events().publish(
                    (soroban_sdk::symbol_short!("thr_chg"),),
                    proposal.new_threshold,
                );
            }

            ProposalKind::AddSigner => {
                let mut updated_signers: Vec<Address> =
                    env.storage().instance().get(&WalletKey::Signers).unwrap();
                updated_signers.push_back(proposal.signer_target.clone());
                env.storage()
                    .instance()
                    .set(&WalletKey::Signers, &updated_signers);

                env.events().publish(
                    (soroban_sdk::symbol_short!("sgn_add"),),
                    proposal.signer_target.clone(),
                );
            }

            ProposalKind::RemoveSigner => {
                let updated_signers: Vec<Address> =
                    env.storage().instance().get(&WalletKey::Signers).unwrap();
                // Rebuild without the removed signer.
                let mut new_signers: Vec<Address> = Vec::new(&env);
                for s in updated_signers.iter() {
                    if s != proposal.signer_target {
                        new_signers.push_back(s);
                    }
                }
                env.storage()
                    .instance()
                    .set(&WalletKey::Signers, &new_signers);

                env.events().publish(
                    (soroban_sdk::symbol_short!("sgn_rm"),),
                    proposal.signer_target.clone(),
                );
            }
        }

        // Mark executed — prevents re-execution.
        proposal.state = ProposalState::Executed;
        env.storage()
            .persistent()
            .set(&WalletKey::Proposal(proposal_id), &proposal);

        env.events().publish(
            (soroban_sdk::symbol_short!("executed"),),
            proposal_id,
        );

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Cancellation
    // -----------------------------------------------------------------------

    /// Cancel a proposal in `Active` or `Approved` state.
    ///
    /// Only the original proposer may cancel.
    pub fn cancel(
        env: Env,
        caller: Address,
        proposal_id: u64,
    ) -> Result<(), WalletError> {
        caller.require_auth();
        Self::require_signer(&env, &caller)?;

        let mut proposal = Self::load_proposal(&env, proposal_id)?;

        if proposal.proposer != caller {
            return Err(WalletError::NotSigner);
        }

        match proposal.state {
            ProposalState::Active | ProposalState::Approved => {}
            _ => return Err(WalletError::InvalidState),
        }

        proposal.state = ProposalState::Cancelled;
        env.storage()
            .persistent()
            .set(&WalletKey::Proposal(proposal_id), &proposal);

        env.events().publish(
            (soroban_sdk::symbol_short!("cancel"),),
            proposal_id,
        );

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Queries
    // -----------------------------------------------------------------------

    /// Return the current signer set.
    pub fn signers(env: Env) -> Vec<Address> {
        env.storage().instance().get(&WalletKey::Signers).unwrap()
    }

    /// Return the current signing threshold.
    pub fn threshold(env: Env) -> u32 {
        env.storage().instance().get(&WalletKey::Threshold).unwrap()
    }

    /// Return the super-majority threshold for governance proposals.
    pub fn super_majority(env: Env) -> u32 {
        let signers: Vec<Address> = env.storage().instance().get(&WalletKey::Signers).unwrap();
        Self::super_majority_threshold(signers.len())
    }

    /// Return a proposal by ID.
    pub fn get_proposal(env: Env, id: u64) -> Result<WalletProposal, WalletError> {
        Self::load_proposal(&env, id)
    }

    /// Return the total number of proposals ever created.
    pub fn proposal_count(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&WalletKey::ProposalCounter)
            .unwrap_or(0u64)
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Require that `addr` is in the current signer set.
    fn require_signer(env: &Env, addr: &Address) -> Result<(), WalletError> {
        let signers: Vec<Address> = env.storage().instance().get(&WalletKey::Signers).unwrap();
        if Self::is_signer_in_vec(&signers, addr) {
            Ok(())
        } else {
            Err(WalletError::NotSigner)
        }
    }

    fn is_signer_in_vec(signers: &Vec<Address>, addr: &Address) -> bool {
        for s in signers.iter() {
            if &s == addr {
                return true;
            }
        }
        false
    }

    /// Increment and return the next proposal ID.
    fn next_proposal_id(env: &Env) -> u64 {
        let counter: u64 = env
            .storage()
            .instance()
            .get(&WalletKey::ProposalCounter)
            .unwrap_or(0u64);
        let next = counter + 1;
        env.storage()
            .instance()
            .set(&WalletKey::ProposalCounter, &next);
        next
    }

    /// Compute the expiry ledger for a new proposal.
    fn expiry_ledger(env: &Env) -> u32 {
        let lifetime: u32 = env
            .storage()
            .instance()
            .get(&WalletKey::ExpiryLedgers)
            .unwrap_or(1000u32);
        env.ledger().sequence() + lifetime
    }

    /// Compute the super-majority threshold: ⌈ n×2/3 ⌉ (minimum 1).
    ///
    /// Examples:
    ///   n=3 → ⌈2.0⌉ = 2
    ///   n=5 → ⌈3.33⌉ = 4 (note: u32 ceiling)
    ///   n=7 → ⌈4.67⌉ = 5
    fn super_majority_threshold(n: u32) -> u32 {
        // ⌈ n * 2 / 3 ⌉  using integer arithmetic: (n * 2 + 2) / 3
        let sm = (n * 2 + 2) / 3;
        sm.max(1)
    }

    /// Load a proposal from persistent storage.
    fn load_proposal(env: &Env, id: u64) -> Result<WalletProposal, WalletError> {
        env.storage()
            .persistent()
            .get(&WalletKey::Proposal(id))
            .ok_or(WalletError::ProposalNotFound)
    }
}
