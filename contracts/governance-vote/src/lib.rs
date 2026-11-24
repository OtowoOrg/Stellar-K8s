//! Time-Locked Governance Voting Sub-Contract with Quorum Enforcement.
//!
//! This contract lets protocol stakeholders propose, vote on, and execute
//! parameter changes under a transparent, deterministic governance process
//! featuring:
//!
//! - **Proposal lifecycle**: creation, voting (`For` / `Against` / `Abstain`),
//!   automated settlement, execution and graceful expiration.
//! - **Dynamic quorum**: the participation threshold is derived from a
//!   configurable `total_weight` (total staked / participating weight) and a
//!   quorum ratio expressed in basis points, so the required quorum adapts to
//!   the total weight in the system and can be made arbitrarily precise.
//! - **Execution timelock**: a `Passed` proposal can only be executed once a
//!   mandatory number of ledger sequences has elapsed since it passed. The
//!   timelock is strictly enforced with `ledger.sequence()` checks.
//! - **Instance TTL extension**: proposal state lives in instance storage and
//!   its TTL is extended on every transition so that it cannot expire while a
//!   vote is still open or a timelock is pending.
//!
//! The data model and the pure tally/quorum rules live in [`proposal`].

pub mod proposal;

use proposal::{
    apply_ballot, quorum_met, quorum_threshold, Ballot, Proposal, ProposalState, Tally, Vote,
};
use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, symbol_short, Address, Env, Symbol,
};

const INSTANCE_ADMIN: Symbol = symbol_short!("_admin");
const INSTANCE_TOTAL_WEIGHT: Symbol = symbol_short!("_total");
const INSTANCE_QUORUM_BPS: Symbol = symbol_short!("_quorum");
const INSTANCE_HORIZON: Symbol = symbol_short!("_horizon");
const INSTANCE_COUNT: Symbol = symbol_short!("_count");

const KEY_PROPOSAL: Symbol = symbol_short!("PROP");
const KEY_SETTING: Symbol = symbol_short!("SETTING");
const KEY_BALLOT: Symbol = symbol_short!("BALLOT");

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GovernanceError {
    Unauthorized = 1,
    ProposalNotFound = 2,
    ProposalNotActive = 3,
    VotingClosed = 4,
    CannotFinalizeEarly = 5,
    AlreadyFinalized = 6,
    NotPassed = 7,
    TimelockNotElapsed = 8,
    AlreadyExecuted = 9,
    CannotExecuteFailed = 10,
    CannotExpireYet = 11,
    InvalidParameter = 12,
    InvalidVotingDuration = 13,
    InvalidExecutionDelay = 14,
    InvalidQuorum = 15,
    InvalidTotalWeight = 16,
    InvalidVoteWeight = 17,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProposalCreated {
    #[topic]
    pub id: u32,
    pub proposer: Address,
    pub parameter: Symbol,
    pub new_value: i128,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoteCast {
    #[topic]
    pub id: u32,
    pub voter: Address,
    pub vote: Vote,
    pub weight: i128,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProposalFinalized {
    #[topic]
    pub id: u32,
    pub state: ProposalState,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProposalExecuted {
    #[topic]
    pub id: u32,
    pub parameter: Symbol,
    pub value: i128,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProposalExpired {
    #[topic]
    pub id: u32,
}

#[contract]
pub struct GovernanceVote;

#[contractimpl]
impl GovernanceVote {
    /// Initialise the governance contract.
    ///
    /// - `total_weight`: total staked/participating weight used to derive the
    ///   dynamic quorum threshold.
    /// - `quorum_percent_bps`: required participation ratio in basis points
    ///   (e.g. `6000` == 60%).
    /// - `expiration_horizon`: number of ledgers past the voting deadline after
    ///   which an abandoned `Active` proposal may be expired gracefully.
    pub fn __constructor(
        env: Env,
        admin: Address,
        total_weight: i128,
        quorum_percent_bps: u32,
        expiration_horizon: u32,
    ) {
        if total_weight <= 0 {
            env.panic_with_error(&GovernanceError::InvalidTotalWeight);
        }
        if quorum_percent_bps == 0 || quorum_percent_bps > 10_000 {
            env.panic_with_error(&GovernanceError::InvalidQuorum);
        }
        let store = env.storage().instance();
        store.set(&INSTANCE_ADMIN, &admin);
        store.set(&INSTANCE_TOTAL_WEIGHT, &total_weight);
        store.set(&INSTANCE_QUORUM_BPS, &quorum_percent_bps);
        store.set(&INSTANCE_HORIZON, &expiration_horizon);
        store.set(&INSTANCE_COUNT, &0u32);
        refresh_instance_ttl(&env);
    }

    // ------------------------------------------------------------------
    // Admin configuration
    // ------------------------------------------------------------------

    pub fn set_admin(env: Env, new_admin: Address) {
        admin(&env).require_auth();
        env.storage().instance().set(&INSTANCE_ADMIN, &new_admin);
        refresh_instance_ttl(&env);
    }

    /// Update the total staked/participating weight. This changes the dynamic
    /// quorum threshold for proposals that have not yet been settled.
    pub fn set_total_weight(env: Env, new_total_weight: i128) {
        admin(&env).require_auth();
        if new_total_weight <= 0 {
            env.panic_with_error(&GovernanceError::InvalidTotalWeight);
        }
        env.storage().instance().set(&INSTANCE_TOTAL_WEIGHT, &new_total_weight);
        refresh_instance_ttl(&env);
    }

    /// Update the quorum participation ratio (basis points, 1/10000).
    pub fn set_quorum_percent_bps(env: Env, new_bps: u32) {
        admin(&env).require_auth();
        if new_bps == 0 || new_bps > 10_000 {
            env.panic_with_error(&GovernanceError::InvalidQuorum);
        }
        env.storage().instance().set(&INSTANCE_QUORUM_BPS, &new_bps);
        refresh_instance_ttl(&env);
    }

    /// Update the abandonment horizon used for graceful expiration.
    pub fn set_expiration_horizon(env: Env, new_horizon: u32) {
        admin(&env).require_auth();
        env.storage().instance().set(&INSTANCE_HORIZON, &new_horizon);
        refresh_instance_ttl(&env);
    }

    // ------------------------------------------------------------------
    // Proposal lifecycle
    // ------------------------------------------------------------------

    /// Create a new `Active` proposal to change `parameter` to `new_value`.
    ///
    /// Voting is open for `voting_duration` ledgers from the current ledger;
    /// a successful proposal becomes executable only after `execution_delay`
    /// further ledgers. Returns the new proposal id.
    pub fn propose(
        env: Env,
        proposer: Address,
        title: Symbol,
        parameter: Symbol,
        new_value: i128,
        voting_duration: u32,
        execution_delay: u32,
    ) -> u32 {
        proposer.require_auth();
        if voting_duration == 0 {
            env.panic_with_error(&GovernanceError::InvalidVotingDuration);
        }
        let store = env.storage().instance();
        let count: u32 = store.get(&INSTANCE_COUNT).unwrap_or(0);
        let id = count + 1;
        store.set(&INSTANCE_COUNT, &id);

        let proposal = Proposal {
            id,
            proposer: proposer.clone(),
            title,
            parameter: parameter.clone(),
            new_value,
            start_ledger: env.ledger().sequence(),
            voting_duration,
            execution_delay,
            quorum_percent_bps: store.get(&INSTANCE_QUORUM_BPS).unwrap(),
            for_weight: 0,
            against_weight: 0,
            abstain_weight: 0,
            total_voted_weight: 0,
            state: ProposalState::Active,
            pass_ledger: None,
        };

        store.set(&(KEY_PROPOSAL, id), &proposal);
        refresh_instance_ttl(&env);
        ProposalCreated {
            id,
            proposer: proposer.clone(),
            parameter,
            new_value,
        }
        .publish(&env);
        id    }

    /// Cast or re-cast a vote on an open proposal.
    ///
    /// `weight` is the staked/participating weight backing this voter. Ballots
    /// are tracked per voter, so re-voting ("vote flips") any number of times
    /// is supported and always updates the running tallies exactly.
    pub fn vote(env: Env, voter: Address, proposal_id: u32, vote: Vote, weight: i128) {
        voter.require_auth();
        if weight <= 0 {
            env.panic_with_error(&GovernanceError::InvalidVoteWeight);
        }

        let store = env.storage().instance();
        let mut proposal: Proposal = load_proposal(&env, proposal_id);
        if proposal.state != ProposalState::Active {
            env.panic_with_error(&GovernanceError::ProposalNotActive);
        }
        let current = env.ledger().sequence();
        if proposal.voting_closed(current) {
            env.panic_with_error(&GovernanceError::VotingClosed);
        }

        let ballot_key = (KEY_BALLOT, proposal_id, voter.clone());
        let previous: Option<Ballot> = env.storage().persistent().get(&ballot_key);
        let deltas = apply_ballot(previous.as_ref(), vote, weight);

        proposal.for_weight += deltas.for_weight;
        proposal.against_weight += deltas.against_weight;
        proposal.abstain_weight += deltas.abstain_weight;
        proposal.total_voted_weight += deltas.total_voted;

        store.set(&(KEY_PROPOSAL, proposal_id), &proposal);
        env.storage()
            .persistent()
            .set(&ballot_key, &Ballot { vote, weight });
        refresh_instance_ttl(&env);
        VoteCast {
            id: proposal_id,
            voter,
            vote,
            weight,
        }
        .publish(&env);
    }

    /// Deterministically settle a proposal once its voting window has closed.
    ///
    /// - Participation below the dynamic quorum threshold -> `Rejected`.
    /// - Quorum met and `for > against` -> `Passed` (timelock starts now).
    /// - Quorum met but not a strict `for` majority (including ties) ->
    ///   `Rejected`.
    pub fn finalize(env: Env, proposal_id: u32) {
        let store = env.storage().instance();
        let mut proposal: Proposal = load_proposal(&env, proposal_id);
        if proposal.state != ProposalState::Active {
            env.panic_with_error(&GovernanceError::AlreadyFinalized);
        }
        let current = env.ledger().sequence();
        if !proposal.voting_closed(current) {
            env.panic_with_error(&GovernanceError::CannotFinalizeEarly);
        }

        let total_weight: i128 = store.get(&INSTANCE_TOTAL_WEIGHT).unwrap();
        let outcome = if !quorum_met(
            proposal.total_voted_weight,
            total_weight,
            proposal.quorum_percent_bps,
        ) {
            Tally::FailedQuorum
        } else if proposal.for_weight > proposal.against_weight {
            Tally::Passed
        } else {
            Tally::Defeated
        };

        let new_state = match outcome {
            Tally::Passed => {
                proposal.pass_ledger = Some(current);
                ProposalState::Passed
            }
            Tally::FailedQuorum | Tally::Defeated => ProposalState::Rejected,
        };
        proposal.state = new_state.clone();
        store.set(&(KEY_PROPOSAL, proposal_id), &proposal);
        refresh_instance_ttl(&env);
        ProposalFinalized {
            id: proposal_id,
            state: new_state,
        }
        .publish(&env);
    }

    /// Execute a `Passed` proposal after its mandatory execution timelock has
    /// elapsed. The parameter change is applied on-chain (written to the
    /// governance settings registry) and the proposal becomes `Executed`.
    pub fn execute(env: Env, proposal_id: u32) {
        let store = env.storage().instance();
        let mut proposal: Proposal = load_proposal(&env, proposal_id);
        match proposal.state {
            ProposalState::Executed => env.panic_with_error(&GovernanceError::AlreadyExecuted),
            ProposalState::Rejected | ProposalState::Expired => {
                env.panic_with_error(&GovernanceError::CannotExecuteFailed)
            }
            ProposalState::Passed => {}
            ProposalState::Active => env.panic_with_error(&GovernanceError::NotPassed),
        }
        let current = env.ledger().sequence();
        if !proposal.timelock_elapsed(current) {
            env.panic_with_error(&GovernanceError::TimelockNotElapsed);
        }

        let param = proposal.parameter.clone();
        store.set(&(KEY_SETTING, param.clone()), &proposal.new_value);
        proposal.state = ProposalState::Executed;
        store.set(&(KEY_PROPOSAL, proposal_id), &proposal);
        refresh_instance_ttl(&env);
        ProposalExecuted {
            id: proposal_id,
            parameter: param,
            value: proposal.new_value,
        }
        .publish(&env);
    }

    /// Gracefully expire an abandoned proposal that is still `Active` well
    /// past its voting deadline (beyond the configured expiration horizon).
    /// Expired proposals can never be voted on, finalized, or executed.
    pub fn expire(env: Env, proposal_id: u32) {
        let store = env.storage().instance();
        let mut proposal: Proposal = load_proposal(&env, proposal_id);
        if proposal.state != ProposalState::Active {
            env.panic_with_error(&GovernanceError::ProposalNotActive);
        }
        let current = env.ledger().sequence();
        let horizon: u32 = store.get(&INSTANCE_HORIZON).unwrap();
        if current <= proposal.deadline().saturating_add(horizon) {
            env.panic_with_error(&GovernanceError::CannotExpireYet);
        }

        proposal.state = ProposalState::Expired;
        store.set(&(KEY_PROPOSAL, proposal_id), &proposal);
        refresh_instance_ttl(&env);
        ProposalExpired { id: proposal_id }.publish(&env);
    }

    // ------------------------------------------------------------------
    // Queries
    // ------------------------------------------------------------------

    pub fn admin(env: Env) -> Address {
        admin(&env)
    }

    pub fn total_weight(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&INSTANCE_TOTAL_WEIGHT)
            .unwrap()
    }

    pub fn quorum_percent_bps(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&INSTANCE_QUORUM_BPS)
            .unwrap()
    }

    pub fn expiration_horizon(env: Env) -> u32 {
        env.storage().instance().get(&INSTANCE_HORIZON).unwrap()
    }

    pub fn proposal_count(env: Env) -> u32 {
        env.storage().instance().get(&INSTANCE_COUNT).unwrap_or(0)
    }

    /// The current dynamic quorum threshold in absolute weight.
    pub fn current_quorum_threshold(env: Env) -> i128 {
        let store = env.storage().instance();
        let total_weight: i128 = store.get(&INSTANCE_TOTAL_WEIGHT).unwrap();
        let bps: u32 = store.get(&INSTANCE_QUORUM_BPS).unwrap();
        quorum_threshold(total_weight, bps)
    }

    pub fn get_proposal(env: Env, proposal_id: u32) -> Option<Proposal> {
        env.storage().instance().get(&(KEY_PROPOSAL, proposal_id))
    }

    pub fn get_vote(env: Env, proposal_id: u32, voter: Address) -> Option<Ballot> {
        env.storage()
            .persistent()
            .get(&(KEY_BALLOT, proposal_id, voter))
    }

    /// The currently applied value of a governance parameter.
    pub fn get_parameter(env: Env, parameter: Symbol) -> Option<i128> {
        env.storage().instance().get(&(KEY_SETTING, parameter))
    }
}

// ------------------------------------------------------------------
// Internal helpers
// ------------------------------------------------------------------

fn admin(env: &Env) -> Address {
    env.storage().instance().get(&INSTANCE_ADMIN).unwrap()
}

fn load_proposal(env: &Env, proposal_id: u32) -> Proposal {
    env.storage()
        .instance()
        .get(&(KEY_PROPOSAL, proposal_id))
        .unwrap_or_else(|| env.panic_with_error(&GovernanceError::ProposalNotFound))
}

/// Extend the TTL of the contract instance (and therefore all proposal state
/// held in instance storage) so that it stays alive through any active voting
/// window and pending execution timelock. We simply hold the instance TTL at
/// the maximum whenever the contract is touched.
fn refresh_instance_ttl(env: &Env) {
    let max = env.storage().max_ttl();
    env.storage().instance().extend_ttl(max, max);
}

#[cfg(test)]
mod tests {
    use super::*;
    use proposal::{ProposalState, Vote};
    use soroban_sdk::{
        testutils::{Address as _, Ledger as _},
        Env,
    };

    /// Roll a fresh contract with admin, total weight, quorum (bps) and
    /// expiration horizon; returns `(env, client, admin, voters)`.
    fn setup(
        total_weight: i128,
        quorum_bps: u32,
        horizon: u32,
    ) -> (
        Env,
        GovernanceVoteClient<'static>,
        Address,
        (Address, Address, Address),
    ) {
        let env = Env::default();
        let admin = Address::generate(&env);
        let voters = (
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
        );
        let contract_id = env.register(
            GovernanceVote,
            (&admin, total_weight, quorum_bps, horizon),
        );
        let client = GovernanceVoteClient::new(&env, &contract_id);
        env.mock_all_auths_allowing_non_root_auth();
        (env, client, admin, voters)
    }

    #[test]
    fn test_constructor_and_queries() {
        let (_env, client, admin, _) = setup(10_000, 6000, 100);
        assert_eq!(client.admin(), admin);
        assert_eq!(client.total_weight(), 10_000);
        assert_eq!(client.quorum_percent_bps(), 6000);
        assert_eq!(client.expiration_horizon(), 100);
        assert_eq!(client.proposal_count(), 0);
        // 60% of 10_000 == 6000 absolute weight
        assert_eq!(client.current_quorum_threshold(), 6000);
    }

    #[test]
    fn test_full_lifecycle_passes_and_executes() {
        let (env, client, _, (a, b, c)) = setup(10_000, 6000, 100);

        env.ledger().set_sequence_number(100);
        let id = client.propose(&a, &symbol_short!("upgrade"), &symbol_short!("fee"), &1234, &100, &50);

        let p = client.get_proposal(&id).unwrap();
        assert_eq!(p.id, id);
        assert_eq!(p.state, ProposalState::Active);
        assert_eq!(p.start_ledger, 100);
        assert_eq!(p.deadline(), 200);
        assert_eq!(client.proposal_count(), 1);

        // Vote within the window.
        client.vote(&a, &id, &Vote::For, &4000);
        client.vote(&b, &id, &Vote::For, &2500);
        client.vote(&c, &id, &Vote::Against, &1000);
        let p = client.get_proposal(&id).unwrap();
        assert_eq!(p.for_weight, 6500);
        assert_eq!(p.against_weight, 1000);
        assert_eq!(p.total_voted_weight, 7500);

        // Ballots are stored.
        assert_eq!(
            client.get_vote(&id, &a).unwrap(),
            Ballot { vote: Vote::For, weight: 4000 }
        );

        // Cannot execute before finalize / before timelock.
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.execute(&id);
        }));
        assert!(err.is_err(), "execute before timelock must panic");

        // Voting closes at ledger 200.
        env.ledger().set_sequence_number(200);
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.vote(&a, &id, &Vote::Against, &4000);
        }));
        assert!(err.is_err(), "vote after deadline must panic");

        // Finalize -> Passed (quorum met, for > against).
        client.finalize(&id);
        let p = client.get_proposal(&id).unwrap();
        assert_eq!(p.state, ProposalState::Passed);
        assert_eq!(p.pass_ledger, Some(200));

        // Not enough time has elapsed yet (delay = 50).
        env.ledger().set_sequence_number(249);
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.execute(&id);
        }));
        assert!(err.is_err(), "execute before timelock elapsed must panic");

        // Timelock elapsed -> execute applies the parameter on-chain.
        env.ledger().set_sequence_number(250);
        client.execute(&id);
        let p = client.get_proposal(&id).unwrap();
        assert_eq!(p.state, ProposalState::Executed);
        assert_eq!(client.get_parameter(&symbol_short!("fee")), Some(1234));

        // Executed proposals cannot be executed again.
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.execute(&id);
        }));
        assert!(err.is_err(), "re-executing must panic");
    }

    #[test]
    fn test_vote_flip_reweights_tallies() {
        let (_env, client, _, (a, _, _)) = setup(10_000, 6000, 100);
        let id = client.propose(&a, &symbol_short!("flip"), &symbol_short!("fee"), &1, &100, &10);

        client.vote(&a, &id, &Vote::For, &400);
        let p = client.get_proposal(&id).unwrap();
        assert_eq!(p.for_weight, 400);

        // Flip to Against: for must drop to 0, against to 400.
        client.vote(&a, &id, &Vote::Against, &400);
        let p = client.get_proposal(&id).unwrap();
        assert_eq!(p.for_weight, 0);
        assert_eq!(p.against_weight, 400);
        assert_eq!(p.total_voted_weight, 400);
        // Only one stored ballot; direction updated.
        assert_eq!(client.get_vote(&id, &a).unwrap(), Ballot { vote: Vote::Against, weight: 400 });

        // Flip back to For with a different weight.
        client.vote(&a, &id, &Vote::For, &500);
        let p = client.get_proposal(&id).unwrap();
        assert_eq!(p.for_weight, 500);
        assert_eq!(p.against_weight, 0);
        assert_eq!(p.total_voted_weight, 500);
    }

    #[test]
    fn test_tie_is_rejected() {
        let (env, client, _, (a, b, _)) = setup(1_000, 6000, 100);
        let id = client.propose(&a, &symbol_short!("tie"), &symbol_short!("fee"), &1, &100, &10);

        client.vote(&a, &id, &Vote::For, &300);
        client.vote(&b, &id, &Vote::Against, &300);
        // total_voted = 600 == threshold (60% of 1000) -> quorum met.

        env.ledger().set_sequence_number(150);
        client.finalize(&id);
        assert_eq!(client.get_proposal(&id).unwrap().state, ProposalState::Rejected);
    }

    #[test]
    fn test_fails_quorum_by_less_than_one_percent() {
        let (env, client, _, (a, b, _)) = setup(10_000, 6000, 100);
        let id = client.propose(&a, &symbol_short!("quorum"), &symbol_short!("fee"), &1, &100, &10);

        // for=3000, against=2995 -> total voted 5995 = 59.95% < 60%
        // (fails quorum by 0.05 percentage points, i.e. < 1%).
        client.vote(&a, &id, &Vote::For, &3000);
        client.vote(&b, &id, &Vote::Against, &2995);

        assert_eq!(client.current_quorum_threshold(), 6000);
        env.ledger().set_sequence_number(150);
        client.finalize(&id);
        // Rejected even though the "for" tally is larger.
        assert_eq!(client.get_proposal(&id).unwrap().state, ProposalState::Rejected);
    }

    #[test]
    fn test_abstentions_count_towards_quorum_but_do_not_carry() {
        let (env, client, _, (a, b, _)) = setup(1_000, 6000, 100);
        let id = client.propose(&a, &symbol_short!("abstain"), &symbol_short!("fee"), &1, &100, &10);

        client.vote(&a, &id, &Vote::Against, &200);
        client.vote(&b, &id, &Vote::Abstain, &400);
        // total_voted = 600 (quorum met) with 0 "for" -> rejected.

        env.ledger().set_sequence_number(150);
        client.finalize(&id);
        assert_eq!(client.get_proposal(&id).unwrap().state, ProposalState::Rejected);
    }

    #[test]
    fn test_finalize_blocked_before_deadline() {
        let (_env, client, _, (a, _, _)) = setup(10_000, 6000, 100);
        let id = client.propose(&a, &symbol_short!("early"), &symbol_short!("fee"), &1, &100, &10);

        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.finalize(&id);
        }));
        assert!(err.is_err(), "finalize before voting closes must panic");
    }

    #[test]
    fn test_expire_graceful() {
        let (env, client, _, (a, _, _)) = setup(10_000, 6000, 100);
        env.ledger().set_sequence_number(100);
        let id = client.propose(&a, &symbol_short!("ghost"), &symbol_short!("fee"), &1, &100, &10);

        // Not enough time past the deadline (horizon = 100): cannot expire yet.
        env.ledger().set_sequence_number(201);
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.expire(&id);
        }));
        assert!(err.is_err(), "expire before horizon must panic");

        // Past deadline + horizon -> can expire.
        env.ledger().set_sequence_number(301);
        client.expire(&id);
        assert_eq!(client.get_proposal(&id).unwrap().state, ProposalState::Expired);

        // Expired proposals cannot be voted on or executed.
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.vote(&a, &id, &Vote::For, &100);
        }));
        assert!(err.is_err(), "voting on expired proposal must panic");
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.execute(&id);
        }));
        assert!(err.is_err(), "executing expired proposal must panic");
    }

    #[test]
    fn test_proposal_not_found_panics() {
        let (_env, client, _, (a, _, _)) = setup(10_000, 6000, 100);
        // A fresh query for an unknown proposal yields None (not a panic).
        assert_eq!(client.get_proposal(&999), None);
        // But mutating operations on a missing proposal must panic.
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.vote(&a, &999, &Vote::For, &100);
        }));
        assert!(err.is_err());
    }

    #[test]
    fn test_quorum_threshold_pure_logic() {
        use proposal::{quorum_met, quorum_threshold};
        // 60% of 1000 == 600.
        assert_eq!(quorum_threshold(1000, 6000), 600);
        assert!(quorum_met(600, 1000, 6000));
        assert!(!quorum_met(599, 1000, 6000));
        // Rounding up: 33.5% of 1000 == 335.
        assert_eq!(quorum_threshold(1000, 3350), 335);
        // 100% quorum requires full participation.
        assert_eq!(quorum_threshold(500, 10_000), 500);
        // 0% quorum.
        assert_eq!(quorum_threshold(500, 0), 0);
    }
}
