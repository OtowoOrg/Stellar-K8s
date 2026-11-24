//! Shared data types for the governance execution queue.
//!
//! All types are `#[contracttype]` so Soroban serialises them into ledger storage
//! and they appear correctly in XDR event payloads.

use soroban_sdk::{contracttype, Address, Bytes, Symbol, Vec};

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

/// Top-level storage key enum used for all persistent storage reads/writes.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Admin address (owner of the contract, can set config).
    Admin,
    /// Multi-sig guardian set (Vec<Address>).
    Guardians,
    /// Minimum number of guardian signatures required to cancel a proposal.
    GuardianThreshold,
    /// Mandatory time delay (in ledgers) before a queued proposal may execute.
    ExecutionDelay,
    /// Auto-incrementing proposal ID counter.
    ProposalCounter,
    /// Individual proposal record keyed by its u64 ID.
    Proposal(u64),
    /// Vote record: (proposal_id, voter_address) → bool (true = yes, false = no).
    Vote(u64, Address),
    /// Quorum — minimum yes-vote count needed to move to Queued state.
    VoteQuorum,
    /// Guardian cancel signatures collected for a proposal.
    CancelSigs(u64),
}

// ---------------------------------------------------------------------------
// Proposal state machine
// ---------------------------------------------------------------------------

/// All possible states a proposal can occupy.
///
/// ```
/// Created → Voting → Queued → Executed
///                 ↓         ↓
///              Cancelled  Cancelled
/// ```
#[contracttype]
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ProposalStatus {
    /// Newly submitted; voting is open.
    Created,
    /// Vote quorum reached; waiting for the execution delay to pass.
    Queued,
    /// Successfully executed on-chain.
    Executed,
    /// Cancelled — either by guardians (before execution) or by the proposer
    /// (only while still in `Created`).
    Cancelled,
    /// Execution attempted but the target call failed; queue state preserved.
    Failed,
}

// ---------------------------------------------------------------------------
// Core proposal record
// ---------------------------------------------------------------------------

/// A governance proposal stored in the registry.
#[contracttype]
#[derive(Clone)]
pub struct Proposal {
    /// Unique identifier assigned at submission time.
    pub id: u64,
    /// The address of the contract to invoke when this proposal executes.
    pub target: Address,
    /// Name of the function to call on `target`.
    pub function: Symbol,
    /// ABI-encoded arguments to pass to `function`.
    pub calldata: Bytes,
    /// Account that submitted this proposal.
    pub proposer: Address,
    /// Ledger sequence number when the proposal was submitted.
    pub created_ledger: u32,
    /// Ledger sequence number at which this proposal became `Queued`.
    /// Zero while still in `Created` state.
    pub queued_ledger: u32,
    /// Ledger sequence number after which execution is permitted
    /// (= `queued_ledger` + `execution_delay`).
    pub earliest_execution_ledger: u32,
    /// Current lifecycle state.
    pub status: ProposalStatus,
    /// Total yes votes accumulated.
    pub yes_votes: u64,
    /// Total no votes accumulated.
    pub no_votes: u64,
    /// Human-readable description (max ~256 bytes recommended).
    pub description: Bytes,
}

// ---------------------------------------------------------------------------
// Event topics
// ---------------------------------------------------------------------------

/// Standard event topics emitted by the contract.
/// Stored as string literals and converted to `Symbol` at call sites.
pub mod events {
    pub const PROPOSAL_CREATED: &str  = "proposal_created";
    pub const VOTE_CAST: &str         = "vote_cast";
    pub const PROPOSAL_QUEUED: &str   = "proposal_queued";
    pub const PROPOSAL_EXECUTED: &str = "proposal_executed";
    pub const PROPOSAL_FAILED: &str   = "proposal_failed";
    pub const PROPOSAL_CANCELLED: &str= "proposal_cancelled";
    pub const GUARDIAN_ADDED: &str    = "guardian_added";
    pub const DELAY_UPDATED: &str     = "delay_updated";
}

// ---------------------------------------------------------------------------
// Error codes
// ---------------------------------------------------------------------------

/// All error conditions the contract can surface.
#[contracttype]
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GovError {
    /// Caller is not the contract admin.
    NotAdmin              = 1,
    /// Proposal with the given ID does not exist.
    ProposalNotFound      = 2,
    /// Operation not valid in the proposal's current state.
    InvalidState          = 3,
    /// The execution delay has not yet elapsed.
    DelayNotElapsed       = 4,
    /// Voter has already cast a vote on this proposal.
    AlreadyVoted          = 5,
    /// Caller is not a guardian.
    NotGuardian           = 6,
    /// Not enough guardian signatures to cancel.
    InsufficientGuardians = 7,
    /// Guardian has already signed the cancellation.
    AlreadySigned         = 8,
    /// Quorum has not been reached; cannot queue yet.
    QuorumNotReached      = 9,
    /// Caller is not the original proposer.
    NotProposer           = 10,
    /// Arithmetic overflow in vote counter.
    Overflow              = 11,
    /// The target contract call returned an error.
    ExecutionFailed       = 12,
    /// Guardian set is empty (must have at least one).
    EmptyGuardianSet      = 13,
}

impl From<GovError> for soroban_sdk::Error {
    fn from(e: GovError) -> Self {
        soroban_sdk::Error::from_contract_error(e as u32)
    }
}
