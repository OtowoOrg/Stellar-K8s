//! Data model and pure (non-storage) logic for the time-locked governance
//! voting sub-contract.
//!
//! This module owns the on-chain representations of a governance proposal, an
//! individual voter's ballot, and the deterministic tally/quorum rules. It is
//! kept free of any storage or environment access so the state-machine rules
//! can be unit-tested in isolation and reasoned about deterministically.

use soroban_sdk::{contracttype, Address, Symbol};

/// The direction of a single vote.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Vote {
    /// Vote in favour of adopting the proposed parameter change.
    For,
    /// Vote against adopting the proposed parameter change.
    Against,
    /// Vote abstention. An abstention still counts towards quorum
    /// participation but does not tilt the outcome toward either side.
    Abstain,
}

/// The lifecycle state of a proposal.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProposalState {
    /// Voting is open (current ledger is within `[start_ledger, deadline)`).
    Active,
    /// Quorum was met and `for > against`; waiting out the execution
    /// timelock before the parameter change may be applied.
    Passed,
    /// The proposal was rejected: it failed quorum, or the `against` tally
    /// met or exceeded `for`.
    Rejected,
    /// The parameter change was applied on-chain after the timelock elapsed.
    Executed,
    /// The proposal was abandoned/expired gracefully (e.g. left `Active`
    /// far beyond its voting window without being settled).
    Expired,
}

/// A governance proposal under consideration.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Proposal {
    /// Monotonic proposal identifier (starts at 1).
    pub id: u32,
    /// Address that submitted the proposal.
    pub proposer: Address,
    /// Human readable short title / description.
    pub title: Symbol,
    /// The governance parameter subject to change.
    pub parameter: Symbol,
    /// The value the parameter should take if the proposal is executed.
    pub new_value: i128,
    /// Ledger sequence at which the proposal was created (voting opens).
    pub start_ledger: u32,
    /// Length of the voting window in ledgers. Voting closes at
    /// `start_ledger + voting_duration`.
    pub voting_duration: u32,
    /// Mandatory timelock in ledgers between passing and execution.
    pub execution_delay: u32,
    /// Quorum as basis points (1/10000) of `total_weight` that must
    /// participate for the outcome to be binding (e.g. 60% == 6000).
    pub quorum_percent_bps: u32,
    /// Cumulative weight voting `for`.
    pub for_weight: i128,
    /// Cumulative weight voting `against`.
    pub against_weight: i128,
    /// Cumulative weight abstaining.
    pub abstain_weight: i128,
    /// `for + against + abstain` participation weight.
    pub total_voted_weight: i128,
    /// Current lifecycle state.
    pub state: ProposalState,
    /// Ledger at which the proposal passed; the timelock is measured from it.
    pub pass_ledger: Option<u32>,
}

impl Proposal {
    /// Ledger at which voting closes.
    pub fn deadline(&self) -> u32 {
        self.start_ledger.saturating_add(self.voting_duration)
    }

    /// Returns `true` once the voting window has elapsed.
    pub fn voting_closed(&self, current_ledger: u32) -> bool {
        current_ledger >= self.deadline()
    }

    /// Returns `true` if the execution timelock has elapsed.
    pub fn timelock_elapsed(&self, current_ledger: u32) -> bool {
        match self.pass_ledger {
            Some(pass) => current_ledger >= pass.saturating_add(self.execution_delay),
            None => false,
        }
    }
}

/// A recorded ballot for a given voter against a given proposal.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Ballot {
    /// The direction the voter currently casts.
    pub vote: Vote,
    /// The staked/participating weight backing this vote.
    pub weight: i128,
}

/// Basis points scaling factor (10000 bps == 100%).
pub const BPS_SCALE: i128 = 10_000;

/// Compute the dynamic quorum threshold (in absolute weight) for a given
/// total staked/participating weight and quorum ratio (in basis points).
///
/// The threshold is rounded up so that a strictly-greater-or-equal comparison
/// is exact with integer arithmetic, which lets us assert "failing quorum by
/// less than 1%" deterministically.
pub fn quorum_threshold(total_weight: i128, quorum_percent_bps: u32) -> i128 {
    let w = total_weight.max(0);
    let bps = quorum_percent_bps as i128;
    if bps >= BPS_SCALE {
        return w;
    }
    // ceil(w * bps / 10000)
    (w * bps).div_euclid(BPS_SCALE) + if (w * bps) % BPS_SCALE != 0 { 1 } else { 0 }
}

/// Whether the required participation quorum has been reached.
pub fn quorum_met(total_voted_weight: i128, total_weight: i128, quorum_percent_bps: u32) -> bool {
    total_voted_weight >= quorum_threshold(total_weight, quorum_percent_bps)
}

/// Cast (or re-cast) a ballot into a proposal's running tally.
///
/// If `previous` is `Some`, the voter is flipping/reweighting: the weight of
/// the prior ballot is first subtracted from the appropriate tally before the
/// new weight is added. Returns `(participation_delta, for_delta, against_delta,
/// abstain_delta)` expressed as deltas to apply to the running totals, plus the
/// new total contribution.
pub fn apply_ballot(previous: Option<&Ballot>, new_vote: Vote, new_weight: i128) -> BallotDeltas {
    let mut d = BallotDeltas {
        total_voted: new_weight,
        for_weight: 0,
        against_weight: 0,
        abstain_weight: 0,
    };

    // Remove the previous vote's contribution first so vote flips are exact.
    if let Some(prev) = previous {
        d.total_voted -= prev.weight;
        match prev.vote {
            Vote::For => d.for_weight -= prev.weight,
            Vote::Against => d.against_weight -= prev.weight,
            Vote::Abstain => d.abstain_weight -= prev.weight,
        }
    }

    match new_vote {
        Vote::For => d.for_weight += new_weight,
        Vote::Against => d.against_weight += new_weight,
        Vote::Abstain => d.abstain_weight += new_weight,
    }

    d
}

/// Result-of-application weight deltas used to update a proposal's tallies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BallotDeltas {
    pub total_voted: i128,
    pub for_weight: i128,
    pub against_weight: i128,
    pub abstain_weight: i128,
}

/// The deterministic settlement outcome of a proposal once voting closes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tally {
    /// Quorum reached and `for > against`.
    Passed,
    /// Participation did not reach the dynamic quorum threshold.
    FailedQuorum,
    /// Quorum reached but the `against` tally met or exceeded `for`.
    Defeated,
}
