//! # Governance Execution Queue & Proposal Registry
//!
//! A Soroban smart contract that implements an on-chain governance pipeline for
//! the Stellar-K8s operator.  It allows community participants to:
//!
//! 1. **Submit proposals** — specify a target contract, function, and calldata.
//! 2. **Vote** — cast yes/no votes; once the quorum threshold is reached the
//!    proposal moves to `Queued` state.
//! 3. **Wait** — the contract enforces a mandatory time delay (in ledgers) before
//!    execution is permitted.
//! 4. **Execute** — any caller may trigger execution after the delay elapses.
//!    If the target contract call fails the proposal is marked `Failed` without
//!    corrupting any other queue state.
//! 5. **Cancel** — emergency multi-sig guardians may cancel a `Created` or
//!    `Queued` proposal by collecting enough signatures.
//!
//! ## Storage layout
//!
//! | Key | Type | TTL |
//! |-----|------|-----|
//! | `Admin` | `Address` | Persistent |
//! | `Guardians` | `Vec<Address>` | Persistent |
//! | `GuardianThreshold` | `u32` | Persistent |
//! | `ExecutionDelay` | `u32` (ledgers) | Persistent |
//! | `VoteQuorum` | `u64` | Persistent |
//! | `ProposalCounter` | `u64` | Persistent |
//! | `Proposal(id)` | `Proposal` | Persistent |
//! | `Vote(id, addr)` | `bool` | Persistent |
//! | `CancelSigs(id)` | `Vec<Address>` | Temporary |

#![no_std]

mod execution;
mod types;

#[cfg(test)]
mod test;

use execution::{
    assert_delay_elapsed, clear_guardian_sigs, dispatch_proposal, load_proposal,
    record_guardian_sig, save_proposal, transition_to_cancelled, transition_to_executed,
    transition_to_failed, transition_to_queued,
};
use types::{DataKey, GovError, Proposal, ProposalStatus, events};

use soroban_sdk::{
    contract, contractimpl, contracttype, log, symbol_short,
    Address, Bytes, Env, Symbol, Vec,
};

// ---------------------------------------------------------------------------
// Contract declaration
// ---------------------------------------------------------------------------

#[contract]
pub struct GovQueueContract;

// ---------------------------------------------------------------------------
// Public interface
// ---------------------------------------------------------------------------

#[contractimpl]
impl GovQueueContract {
    // -----------------------------------------------------------------------
    // Initialisation
    // -----------------------------------------------------------------------

    /// Initialise the contract.
    ///
    /// Must be called exactly once by the deployer.
    ///
    /// * `admin`            — address that controls configuration.
    /// * `guardians`        — initial set of emergency guardian addresses.
    /// * `guardian_threshold` — minimum guardian signatures required to cancel.
    /// * `execution_delay`  — mandatory delay in ledgers between a proposal
    ///                         reaching quorum and being executable.
    /// * `vote_quorum`      — minimum yes-vote count to advance to `Queued`.
    pub fn initialize(
        env: Env,
        admin: Address,
        guardians: Vec<Address>,
        guardian_threshold: u32,
        execution_delay: u32,
        vote_quorum: u64,
    ) -> Result<(), GovError> {
        // Prevent re-initialisation.
        if env.storage().persistent().has(&DataKey::Admin) {
            return Err(GovError::NotAdmin);
        }

        if guardians.is_empty() {
            return Err(GovError::EmptyGuardianSet);
        }

        admin.require_auth();

        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().set(&DataKey::Guardians, &guardians);
        env.storage().persistent().set(&DataKey::GuardianThreshold, &guardian_threshold);
        env.storage().persistent().set(&DataKey::ExecutionDelay, &execution_delay);
        env.storage().persistent().set(&DataKey::VoteQuorum, &vote_quorum);
        env.storage().persistent().set(&DataKey::ProposalCounter, &0u64);

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Configuration (admin only)
    // -----------------------------------------------------------------------

    /// Update the mandatory execution delay (in ledgers).
    pub fn set_execution_delay(env: Env, delay: u32) -> Result<(), GovError> {
        Self::require_admin(&env)?;
        env.storage().persistent().set(&DataKey::ExecutionDelay, &delay);
        env.events().publish(
            (Symbol::new(&env, events::DELAY_UPDATED),),
            delay,
        );
        Ok(())
    }

    /// Replace the entire guardian set.
    pub fn set_guardians(
        env: Env,
        guardians: Vec<Address>,
        threshold: u32,
    ) -> Result<(), GovError> {
        Self::require_admin(&env)?;
        if guardians.is_empty() {
            return Err(GovError::EmptyGuardianSet);
        }
        env.storage().persistent().set(&DataKey::Guardians, &guardians);
        env.storage().persistent().set(&DataKey::GuardianThreshold, &threshold);
        env.events().publish(
            (Symbol::new(&env, events::GUARDIAN_ADDED),),
            (guardians, threshold),
        );
        Ok(())
    }

    /// Update the minimum yes-vote quorum.
    pub fn set_vote_quorum(env: Env, quorum: u64) -> Result<(), GovError> {
        Self::require_admin(&env)?;
        env.storage().persistent().set(&DataKey::VoteQuorum, &quorum);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Proposal lifecycle
    // -----------------------------------------------------------------------

    /// Submit a new governance proposal.
    ///
    /// Returns the new proposal's ID.
    ///
    /// * `proposer`     — must authorise the call.
    /// * `target`       — contract address to invoke on execution.
    /// * `function`     — function name to call on `target`.
    /// * `calldata`     — ABI-encoded arguments for `function`.
    /// * `description`  — human-readable summary (stored on-chain).
    pub fn submit_proposal(
        env: Env,
        proposer: Address,
        target: Address,
        function: Symbol,
        calldata: Bytes,
        description: Bytes,
    ) -> Result<u64, GovError> {
        proposer.require_auth();

        // Assign a new ID.
        let id: u64 = env
            .storage()
            .persistent()
            .get::<DataKey, u64>(&DataKey::ProposalCounter)
            .unwrap_or(0);
        let next_id = id.checked_add(1).ok_or(GovError::Overflow)?;
        env.storage().persistent().set(&DataKey::ProposalCounter, &next_id);

        let proposal = Proposal {
            id: next_id,
            target,
            function,
            calldata,
            proposer,
            created_ledger: env.ledger().sequence(),
            queued_ledger: 0,
            earliest_execution_ledger: 0,
            status: ProposalStatus::Created,
            yes_votes: 0,
            no_votes: 0,
            description,
        };

        save_proposal(&env, &proposal);

        env.events().publish(
            (Symbol::new(&env, events::PROPOSAL_CREATED),),
            next_id,
        );

        log!(&env, "proposal {} submitted by {}", next_id, proposal.proposer);
        Ok(next_id)
    }

    /// Cast a yes (`support = true`) or no (`support = false`) vote on a proposal.
    ///
    /// Each address may vote exactly once.  Once the yes-vote count reaches the
    /// configured quorum the proposal automatically transitions to `Queued`.
    pub fn cast_vote(
        env: Env,
        voter: Address,
        proposal_id: u64,
        support: bool,
    ) -> Result<(), GovError> {
        voter.require_auth();

        let mut proposal = load_proposal(&env, proposal_id)?;

        // Voting is only open while the proposal is in `Created` state.
        if proposal.status != ProposalStatus::Created {
            return Err(GovError::InvalidState);
        }

        // Each address votes at most once.
        let vote_key = DataKey::Vote(proposal_id, voter.clone());
        if env.storage().persistent().has(&vote_key) {
            return Err(GovError::AlreadyVoted);
        }
        env.storage().persistent().set(&vote_key, &support);

        // Tally.
        if support {
            proposal.yes_votes = proposal.yes_votes.checked_add(1).ok_or(GovError::Overflow)?;
        } else {
            proposal.no_votes = proposal.no_votes.checked_add(1).ok_or(GovError::Overflow)?;
        }

        env.events().publish(
            (Symbol::new(&env, events::VOTE_CAST),),
            (proposal_id, voter.clone(), support),
        );

        log!(&env, "vote cast on proposal {}: support={}", proposal_id, support);

        // Check if quorum is reached — if so, advance to Queued.
        let quorum: u64 = env
            .storage()
            .persistent()
            .get::<DataKey, u64>(&DataKey::VoteQuorum)
            .unwrap_or(1);

        if proposal.yes_votes >= quorum {
            transition_to_queued(&env, &mut proposal);
            env.events().publish(
                (Symbol::new(&env, events::PROPOSAL_QUEUED),),
                (proposal_id, proposal.earliest_execution_ledger),
            );
            log!(
                &env,
                "proposal {} queued; executable after ledger {}",
                proposal_id,
                proposal.earliest_execution_ledger
            );
        } else {
            save_proposal(&env, &proposal);
        }

        Ok(())
    }

    /// Manually queue a proposal that has reached quorum but has not yet been
    /// queued (e.g. if `cast_vote` was not called by the voter who pushed it
    /// over quorum — defensive path).
    pub fn queue_proposal(env: Env, proposal_id: u64) -> Result<(), GovError> {
        let mut proposal = load_proposal(&env, proposal_id)?;

        if proposal.status != ProposalStatus::Created {
            return Err(GovError::InvalidState);
        }

        let quorum: u64 = env
            .storage()
            .persistent()
            .get::<DataKey, u64>(&DataKey::VoteQuorum)
            .unwrap_or(1);

        if proposal.yes_votes < quorum {
            return Err(GovError::QuorumNotReached);
        }

        transition_to_queued(&env, &mut proposal);

        env.events().publish(
            (Symbol::new(&env, events::PROPOSAL_QUEUED),),
            (proposal_id, proposal.earliest_execution_ledger),
        );

        Ok(())
    }

    /// Execute a queued proposal.
    ///
    /// Anyone may call this function once:
    /// - The proposal is in `Queued` state.
    /// - `current_ledger >= proposal.earliest_execution_ledger`.
    ///
    /// If the target contract call fails the proposal is marked `Failed` and
    /// the queue state is preserved intact.
    pub fn execute_proposal(env: Env, proposal_id: u64) -> Result<(), GovError> {
        let mut proposal = load_proposal(&env, proposal_id)?;

        // Must be in Queued state.
        if proposal.status != ProposalStatus::Queued {
            return Err(GovError::InvalidState);
        }

        // Enforce the time delay — this is the critical gate.
        assert_delay_elapsed(&env, &proposal)?;

        // Attempt dispatch.  If the call panics / errors we mark as Failed.
        match dispatch_proposal(&env, &proposal) {
            Ok(_) => {
                transition_to_executed(&env, &mut proposal);
                env.events().publish(
                    (Symbol::new(&env, events::PROPOSAL_EXECUTED),),
                    proposal_id,
                );
                log!(&env, "proposal {} executed successfully", proposal_id);
            }
            Err(_) => {
                transition_to_failed(&env, &mut proposal);
                env.events().publish(
                    (Symbol::new(&env, events::PROPOSAL_FAILED),),
                    proposal_id,
                );
                log!(&env, "proposal {} execution failed; marked Failed", proposal_id);
                return Err(GovError::ExecutionFailed);
            }
        }

        Ok(())
    }

    /// Cancel a proposal.
    ///
    /// The proposer may cancel their own proposal while it is still in `Created`
    /// state (before quorum is reached).
    ///
    /// Guardians may cancel a `Created` or `Queued` proposal by each calling
    /// this function; once `guardian_threshold` unique guardian signatures are
    /// accumulated the cancellation proceeds.
    pub fn cancel_proposal(env: Env, caller: Address, proposal_id: u64) -> Result<(), GovError> {
        caller.require_auth();

        let mut proposal = load_proposal(&env, proposal_id)?;

        // Only Created or Queued proposals can be cancelled.
        if proposal.status != ProposalStatus::Created
            && proposal.status != ProposalStatus::Queued
        {
            return Err(GovError::InvalidState);
        }

        // Path 1: proposer self-cancels (only while still in Created).
        if caller == proposal.proposer && proposal.status == ProposalStatus::Created {
            transition_to_cancelled(&env, &mut proposal);
            env.events().publish(
                (Symbol::new(&env, events::PROPOSAL_CANCELLED),),
                (proposal_id, caller),
            );
            log!(&env, "proposal {} cancelled by proposer", proposal_id);
            return Ok(());
        }

        // Path 2: guardian multi-sig cancellation.
        let threshold_reached = record_guardian_sig(&env, proposal_id, &caller)?;

        if threshold_reached {
            clear_guardian_sigs(&env, proposal_id);
            transition_to_cancelled(&env, &mut proposal);
            env.events().publish(
                (Symbol::new(&env, events::PROPOSAL_CANCELLED),),
                (proposal_id, caller),
            );
            log!(&env, "proposal {} cancelled by guardians", proposal_id);
        } else {
            log!(
                &env,
                "guardian signature recorded for proposal {}; threshold not yet reached",
                proposal_id
            );
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Read-only queries
    // -----------------------------------------------------------------------

    /// Return the full proposal record for `id`.
    pub fn get_proposal(env: Env, id: u64) -> Result<Proposal, GovError> {
        load_proposal(&env, id)
    }

    /// Return the current status of proposal `id`.
    pub fn get_proposal_status(env: Env, id: u64) -> Result<ProposalStatus, GovError> {
        let p = load_proposal(&env, id)?;
        Ok(p.status)
    }

    /// Return the current execution delay in ledgers.
    pub fn get_execution_delay(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get::<DataKey, u32>(&DataKey::ExecutionDelay)
            .unwrap_or(0)
    }

    /// Return the current vote quorum.
    pub fn get_vote_quorum(env: Env) -> u64 {
        env.storage()
            .persistent()
            .get::<DataKey, u64>(&DataKey::VoteQuorum)
            .unwrap_or(1)
    }

    /// Return the current guardian set.
    pub fn get_guardians(env: Env) -> Vec<Address> {
        env.storage()
            .persistent()
            .get::<DataKey, Vec<Address>>(&DataKey::Guardians)
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Return the highest issued proposal ID (0 = none yet).
    pub fn proposal_count(env: Env) -> u64 {
        env.storage()
            .persistent()
            .get::<DataKey, u64>(&DataKey::ProposalCounter)
            .unwrap_or(0)
    }

    /// Check whether `voter` has already voted on `proposal_id`.
    pub fn has_voted(env: Env, proposal_id: u64, voter: Address) -> bool {
        env.storage()
            .persistent()
            .has(&DataKey::Vote(proposal_id, voter))
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Verify that the caller is the contract admin.
    fn require_admin(env: &Env) -> Result<(), GovError> {
        let admin: Address = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::Admin)
            .ok_or(GovError::NotAdmin)?;
        admin.require_auth();
        Ok(())
    }
}
