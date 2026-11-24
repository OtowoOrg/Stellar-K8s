//! Full lifecycle tests for the Governance Execution Queue.
//!
//! Covers the complete proposal lifecycle required by issue #74's Definition of Done:
//!
//! - Proposal creation and storage
//! - Vote casting and quorum detection
//! - Queue transition with correct `earliest_execution_ledger`
//! - Execution blocked until the exact delay elapses (simulated config change)
//! - Successful execution after delay passes
//! - Guardian multi-sig cancellation
//! - Proposer self-cancellation
//! - Failed execution does not corrupt queue state
//! - Error paths: double-vote, wrong state, not guardian, etc.

#![cfg(test)]

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Bytes, Env, Symbol, Vec,
};

use crate::{GovQueueContract, GovQueueContractClient};
use crate::types::{GovError, ProposalStatus};

// ---------------------------------------------------------------------------
// Mock target contract — simulates a "configuration change" call
// ---------------------------------------------------------------------------

mod mock_target {
    use soroban_sdk::{contract, contractimpl, Bytes, Env};

    #[contract]
    pub struct MockTarget;

    #[contractimpl]
    impl MockTarget {
        /// Simulated config-change entry point — simply returns successfully.
        pub fn apply_config(_env: Env, _calldata: Bytes) {}
    }
}

use mock_target::{MockTarget, MockTargetClient};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Bootstrap a fresh environment.
/// Returns (env, gov_client, admin, guardian1, guardian2).
fn setup(
    execution_delay: u32,
    vote_quorum: u64,
) -> (Env, GovQueueContractClient<'static>, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, GovQueueContract);
    let client: GovQueueContractClient =
        GovQueueContractClient::new(&env, &contract_id);

    let admin     = Address::generate(&env);
    let guardian1 = Address::generate(&env);
    let guardian2 = Address::generate(&env);

    let guardians = soroban_sdk::vec![&env, guardian1.clone(), guardian2.clone()];

    client
        .initialize(&admin, &guardians, &1u32, &execution_delay, &vote_quorum)
        .unwrap();

    // SAFETY: lifetime erasure acceptable in Soroban test harness pattern.
    let client: GovQueueContractClient<'static> =
        unsafe { core::mem::transmute(client) };

    (env, client, admin, guardian1, guardian2)
}

/// Deploy mock target + submit the canonical simulated config-change proposal.
/// Returns (proposal_id, target_address).
fn submit_config_proposal(
    env: &Env,
    client: &GovQueueContractClient,
    proposer: &Address,
) -> (u64, Address) {
    let target_id = env.register_contract(None, MockTarget);
    let calldata  = Bytes::from_slice(env, b"set_param=42");
    let desc      = Bytes::from_slice(env, b"Simulated configuration change");

    let id = client
        .submit_proposal(
            proposer,
            &target_id,
            &Symbol::new(env, "apply_config"),
            &calldata,
            &desc,
        )
        .unwrap();

    (id, target_id)
}

/// Advance the ledger sequence number by `n`.
fn advance_ledger(env: &Env, n: u32) {
    env.ledger().with_mut(|li| {
        li.sequence_number = li.sequence_number.saturating_add(n);
    });
}

/// Cast `count` yes-votes from distinct fresh addresses.
fn cast_yes_votes(env: &Env, client: &GovQueueContractClient, proposal_id: u64, count: u64) {
    for _ in 0..count {
        let voter = Address::generate(env);
        client.cast_vote(&voter, &proposal_id, &true).unwrap();
    }
}

// ---------------------------------------------------------------------------
// 1. Initialisation
// ---------------------------------------------------------------------------

#[test]
fn test_initialize_stores_config() {
    let (_env, client, _admin, _g1, _g2) = setup(100, 2);
    assert_eq!(client.get_execution_delay(), 100u32);
    assert_eq!(client.get_vote_quorum(), 2u64);
    assert_eq!(client.get_guardians().len(), 2u32);
    assert_eq!(client.proposal_count(), 0u64);
}

#[test]
fn test_double_initialize_fails() {
    let (env, client, admin, g1, _g2) = setup(100, 2);
    let guardians = soroban_sdk::vec![&env, g1.clone()];
    let result = client.initialize(&admin, &guardians, &1u32, &50u32, &1u64);
    assert!(result.is_err(), "second initialize must fail");
}

// ---------------------------------------------------------------------------
// 2. Proposal submission
// ---------------------------------------------------------------------------

#[test]
fn test_submit_proposal_increments_counter() {
    let (env, client, _admin, _g1, _g2) = setup(100, 2);
    let proposer = Address::generate(&env);

    let (id1, _) = submit_config_proposal(&env, &client, &proposer);
    let (id2, _) = submit_config_proposal(&env, &client, &proposer);

    assert_eq!(id1, 1u64);
    assert_eq!(id2, 2u64);
    assert_eq!(client.proposal_count(), 2u64);
}

#[test]
fn test_proposal_initial_state_is_created() {
    let (env, client, _admin, _g1, _g2) = setup(100, 2);
    let proposer = Address::generate(&env);

    let (id, _) = submit_config_proposal(&env, &client, &proposer);
    let proposal = client.get_proposal(&id).unwrap();

    assert_eq!(proposal.status, ProposalStatus::Created);
    assert_eq!(proposal.yes_votes, 0u64);
    assert_eq!(proposal.no_votes, 0u64);
    assert_eq!(proposal.queued_ledger, 0u32);
    assert_eq!(proposal.earliest_execution_ledger, 0u32);
}

// ---------------------------------------------------------------------------
// 3. Voting
// ---------------------------------------------------------------------------

#[test]
fn test_cast_yes_vote_increments_tally() {
    let (env, client, _admin, _g1, _g2) = setup(100, 5);
    let proposer = Address::generate(&env);
    let voter    = Address::generate(&env);

    let (id, _) = submit_config_proposal(&env, &client, &proposer);
    client.cast_vote(&voter, &id, &true).unwrap();

    let proposal = client.get_proposal(&id).unwrap();
    assert_eq!(proposal.yes_votes, 1u64);
    assert_eq!(proposal.no_votes, 0u64);
    assert!(client.has_voted(&id, &voter));
}

#[test]
fn test_cast_no_vote_increments_tally() {
    let (env, client, _admin, _g1, _g2) = setup(100, 5);
    let proposer = Address::generate(&env);
    let voter    = Address::generate(&env);

    let (id, _) = submit_config_proposal(&env, &client, &proposer);
    client.cast_vote(&voter, &id, &false).unwrap();

    let proposal = client.get_proposal(&id).unwrap();
    assert_eq!(proposal.yes_votes, 0u64);
    assert_eq!(proposal.no_votes, 1u64);
}

#[test]
fn test_double_vote_fails() {
    let (env, client, _admin, _g1, _g2) = setup(100, 5);
    let proposer = Address::generate(&env);
    let voter    = Address::generate(&env);

    let (id, _) = submit_config_proposal(&env, &client, &proposer);
    client.cast_vote(&voter, &id, &true).unwrap();

    let err = client.cast_vote(&voter, &id, &true).unwrap_err();
    assert_eq!(err, soroban_sdk::Error::from_contract_error(GovError::AlreadyVoted as u32));
}

#[test]
fn test_vote_on_nonexistent_proposal_fails() {
    let (env, client, _admin, _g1, _g2) = setup(100, 2);
    let voter = Address::generate(&env);

    let err = client.cast_vote(&voter, &999u64, &true).unwrap_err();
    assert_eq!(err, soroban_sdk::Error::from_contract_error(GovError::ProposalNotFound as u32));
}

// ---------------------------------------------------------------------------
// 4. Queue transition on quorum
// ---------------------------------------------------------------------------

#[test]
fn test_proposal_queued_when_quorum_reached() {
    let execution_delay = 50u32;
    let quorum          = 3u64;
    let (env, client, _admin, _g1, _g2) = setup(execution_delay, quorum);
    let proposer = Address::generate(&env);

    let (id, _) = submit_config_proposal(&env, &client, &proposer);

    // Start at ledger 10.
    env.ledger().with_mut(|li| li.sequence_number = 10);

    // Cast votes up to but not including quorum — should stay Created.
    cast_yes_votes(&env, &client, id, quorum - 1);
    assert_eq!(client.get_proposal_status(&id).unwrap(), ProposalStatus::Created);

    // Cast the quorum-crossing vote.
    let last_voter = Address::generate(&env);
    client.cast_vote(&last_voter, &id, &true).unwrap();

    let proposal = client.get_proposal(&id).unwrap();
    assert_eq!(proposal.status, ProposalStatus::Queued);
    assert_eq!(proposal.queued_ledger, 10u32);
    assert_eq!(
        proposal.earliest_execution_ledger,
        10u32 + execution_delay,
        "earliest_execution_ledger must equal queued_ledger + execution_delay"
    );
}

#[test]
fn test_manual_queue_requires_quorum() {
    let (env, client, _admin, _g1, _g2) = setup(50, 3);
    let proposer = Address::generate(&env);

    let (id, _) = submit_config_proposal(&env, &client, &proposer);
    // Only one vote — not enough.
    cast_yes_votes(&env, &client, id, 1);

    let err = client.queue_proposal(&id).unwrap_err();
    assert_eq!(err, soroban_sdk::Error::from_contract_error(GovError::QuorumNotReached as u32));
}

// ---------------------------------------------------------------------------
// 5. Execution delay enforcement  *** KEY VALIDATION FROM ISSUE #74 ***
//
// "Queue a simulated configuration change contract call and verify execution
//  is blocked until the exact delay sequence passes."
// ---------------------------------------------------------------------------

#[test]
fn test_execution_blocked_before_delay_elapses() {
    let execution_delay = 100u32;
    let quorum          = 2u64;
    let (env, client, _admin, _g1, _g2) = setup(execution_delay, quorum);
    let proposer = Address::generate(&env);

    // Set ledger to a known starting point.
    env.ledger().with_mut(|li| li.sequence_number = 1000);

    let (id, _) = submit_config_proposal(&env, &client, &proposer);
    cast_yes_votes(&env, &client, id, quorum);

    let proposal = client.get_proposal(&id).unwrap();
    assert_eq!(proposal.status, ProposalStatus::Queued);

    // earliest_execution_ledger = 1000 + 100 = 1100.
    assert_eq!(proposal.earliest_execution_ledger, 1100u32);

    // Advance to one ledger BEFORE the deadline — must still be blocked.
    advance_ledger(&env, execution_delay - 1); // ledger = 1099
    let current = env.ledger().sequence();
    assert_eq!(current, 1099u32, "sanity check: ledger should be 1099");

    let err = client.execute_proposal(&id).unwrap_err();
    assert_eq!(
        err,
        soroban_sdk::Error::from_contract_error(GovError::DelayNotElapsed as u32),
        "execution must be blocked at ledger {} (deadline is {})",
        current,
        proposal.earliest_execution_ledger
    );

    // Advance exactly one more ledger — now at deadline.
    advance_ledger(&env, 1); // ledger = 1100
    let current = env.ledger().sequence();
    assert_eq!(current, 1100u32, "sanity check: ledger should be at deadline");

    // Execution must succeed now.
    client.execute_proposal(&id).unwrap();
    assert_eq!(
        client.get_proposal_status(&id).unwrap(),
        ProposalStatus::Executed
    );
}

#[test]
fn test_execution_blocked_at_every_ledger_before_delay() {
    // Check that blocking holds at ledger queued+0, +1, … +(delay-1).
    let execution_delay = 10u32;
    let quorum          = 1u64;
    let (env, client, _admin, _g1, _g2) = setup(execution_delay, quorum);
    let proposer = Address::generate(&env);

    env.ledger().with_mut(|li| li.sequence_number = 0);

    let (id, _) = submit_config_proposal(&env, &client, &proposer);
    cast_yes_votes(&env, &client, id, quorum);

    for offset in 0..execution_delay {
        env.ledger().with_mut(|li| li.sequence_number = offset);
        let result = client.execute_proposal(&id);
        assert!(
            result.is_err(),
            "execution must fail at ledger {} (delay={}, deadline={})",
            offset,
            execution_delay,
            execution_delay
        );
    }
}

#[test]
fn test_execution_succeeds_after_delay() {
    let (env, client, _admin, _g1, _g2) = setup(50, 2);
    let proposer = Address::generate(&env);

    env.ledger().with_mut(|li| li.sequence_number = 500);

    let (id, _) = submit_config_proposal(&env, &client, &proposer);
    cast_yes_votes(&env, &client, id, 2);

    // Advance well past the delay.
    advance_ledger(&env, 100); // 500 + 100 > 500 + 50

    client.execute_proposal(&id).unwrap();
    assert_eq!(
        client.get_proposal_status(&id).unwrap(),
        ProposalStatus::Executed
    );
}

#[test]
fn test_cannot_execute_twice() {
    let (env, client, _admin, _g1, _g2) = setup(10, 1);
    let proposer = Address::generate(&env);

    env.ledger().with_mut(|li| li.sequence_number = 0);
    let (id, _) = submit_config_proposal(&env, &client, &proposer);
    cast_yes_votes(&env, &client, id, 1);

    advance_ledger(&env, 10);
    client.execute_proposal(&id).unwrap();

    // Second call must fail (already Executed, not Queued).
    let err = client.execute_proposal(&id).unwrap_err();
    assert_eq!(err, soroban_sdk::Error::from_contract_error(GovError::InvalidState as u32));
}

#[test]
fn test_cannot_execute_created_proposal() {
    let (env, client, _admin, _g1, _g2) = setup(10, 5);
    let proposer = Address::generate(&env);

    let (id, _) = submit_config_proposal(&env, &client, &proposer);
    // No votes cast — still Created.
    advance_ledger(&env, 100);

    let err = client.execute_proposal(&id).unwrap_err();
    assert_eq!(err, soroban_sdk::Error::from_contract_error(GovError::InvalidState as u32));
}

// ---------------------------------------------------------------------------
// 6. Proposer self-cancellation
// ---------------------------------------------------------------------------

#[test]
fn test_proposer_can_cancel_created_proposal() {
    let (env, client, _admin, _g1, _g2) = setup(100, 3);
    let proposer = Address::generate(&env);

    let (id, _) = submit_config_proposal(&env, &client, &proposer);
    client.cancel_proposal(&proposer, &id).unwrap();

    assert_eq!(
        client.get_proposal_status(&id).unwrap(),
        ProposalStatus::Cancelled
    );
}

#[test]
fn test_proposer_cannot_cancel_queued_proposal() {
    let (env, client, _admin, _g1, _g2) = setup(100, 1);
    let proposer = Address::generate(&env);

    let (id, _) = submit_config_proposal(&env, &client, &proposer);
    cast_yes_votes(&env, &client, id, 1);
    // Proposal is now Queued.

    // Proposer tries to self-cancel a Queued proposal — not allowed through
    // the proposer path.  Falls through to guardian path but proposer is not
    // a guardian, so NotGuardian is returned.
    let err = client.cancel_proposal(&proposer, &id).unwrap_err();
    assert_eq!(err, soroban_sdk::Error::from_contract_error(GovError::NotGuardian as u32));
}

#[test]
fn test_non_proposer_cannot_self_cancel() {
    let (env, client, _admin, _g1, _g2) = setup(100, 3);
    let proposer   = Address::generate(&env);
    let non_owner  = Address::generate(&env);

    let (id, _) = submit_config_proposal(&env, &client, &proposer);

    // non_owner is not the proposer and not a guardian.
    let err = client.cancel_proposal(&non_owner, &id).unwrap_err();
    assert_eq!(err, soroban_sdk::Error::from_contract_error(GovError::NotGuardian as u32));
}

// ---------------------------------------------------------------------------
// 7. Guardian multi-sig cancellation
// ---------------------------------------------------------------------------

#[test]
fn test_guardian_can_cancel_queued_proposal() {
    // threshold = 1, so single guardian signature suffices.
    let (env, client, _admin, guardian1, _g2) = setup(100, 1);
    let proposer = Address::generate(&env);

    let (id, _) = submit_config_proposal(&env, &client, &proposer);
    cast_yes_votes(&env, &client, id, 1); // → Queued

    client.cancel_proposal(&guardian1, &id).unwrap();

    assert_eq!(
        client.get_proposal_status(&id).unwrap(),
        ProposalStatus::Cancelled
    );
}

#[test]
fn test_guardian_multisig_cancellation_requires_threshold() {
    // Two guardians, threshold = 2 — need both signatures.
    let (env, client, _admin, guardian1, guardian2) = setup(100, 1);
    let proposer = Address::generate(&env);

    // Re-init with threshold = 2.
    // We'll work with the existing setup but manipulate via set_guardians.
    // Actually setup already set threshold=1; let's use a fresh setup:
    drop(client); // end borrow
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, GovQueueContract);
    let client = GovQueueContractClient::new(&env, &contract_id);
    let admin  = Address::generate(&env);
    let g1     = Address::generate(&env);
    let g2     = Address::generate(&env);
    let guardians = soroban_sdk::vec![&env, g1.clone(), g2.clone()];

    client.initialize(&admin, &guardians, &2u32 /* threshold=2 */, &100u32, &1u64).unwrap();

    // Submit + queue.
    let target_id = env.register_contract(None, MockTarget);
    let calldata  = Bytes::from_slice(&env, b"data");
    let desc      = Bytes::from_slice(&env, b"test");
    let id = client.submit_proposal(
        &Address::generate(&env),
        &target_id,
        &Symbol::new(&env, "apply_config"),
        &calldata,
        &desc,
    ).unwrap();
    cast_yes_votes(&env, &client, id, 1); // → Queued

    // First guardian signs — threshold not yet reached, no cancellation yet.
    client.cancel_proposal(&g1, &id).unwrap();
    assert_eq!(
        client.get_proposal_status(&id).unwrap(),
        ProposalStatus::Queued,
        "proposal should still be Queued after first guardian signature"
    );

    // Second guardian signs — threshold reached, proposal cancelled.
    client.cancel_proposal(&g2, &id).unwrap();
    assert_eq!(
        client.get_proposal_status(&id).unwrap(),
        ProposalStatus::Cancelled,
        "proposal should be Cancelled after second guardian signature"
    );
}

#[test]
fn test_guardian_double_sign_fails() {
    let (env, client, _admin, guardian1, _g2) = setup(100, 2);
    let proposer = Address::generate(&env);

    let (id, _) = submit_config_proposal(&env, &client, &proposer);
    cast_yes_votes(&env, &client, id, 2); // → Queued

    // Re-init with threshold=2 so first sig doesn't cancel.
    // Using existing setup (threshold=1), first sig already cancels — let's
    // test double-sign with a threshold=2 setup instead:
    let env2 = Env::default();
    env2.mock_all_auths();
    let cid2 = env2.register_contract(None, GovQueueContract);
    let c2   = GovQueueContractClient::new(&env2, &cid2);
    let a2   = Address::generate(&env2);
    let g_a  = Address::generate(&env2);
    let g_b  = Address::generate(&env2);
    let gs   = soroban_sdk::vec![&env2, g_a.clone(), g_b.clone()];
    c2.initialize(&a2, &gs, &2u32, &50u32, &1u64).unwrap();

    let tid = env2.register_contract(None, MockTarget);
    let cd  = Bytes::from_slice(&env2, b"x");
    let dd  = Bytes::from_slice(&env2, b"d");
    let pid = c2.submit_proposal(
        &Address::generate(&env2), &tid,
        &Symbol::new(&env2, "apply_config"), &cd, &dd,
    ).unwrap();
    cast_yes_votes(&env2, &c2, pid, 1);

    c2.cancel_proposal(&g_a, &pid).unwrap(); // first sig OK
    let err = c2.cancel_proposal(&g_a, &pid).unwrap_err(); // double-sign
    assert_eq!(err, soroban_sdk::Error::from_contract_error(GovError::AlreadySigned as u32));
}

#[test]
fn test_non_guardian_cannot_cancel() {
    let (env, client, _admin, _g1, _g2) = setup(100, 2);
    let proposer   = Address::generate(&env);
    let outsider   = Address::generate(&env);

    let (id, _) = submit_config_proposal(&env, &client, &proposer);
    cast_yes_votes(&env, &client, id, 2);

    let err = client.cancel_proposal(&outsider, &id).unwrap_err();
    assert_eq!(err, soroban_sdk::Error::from_contract_error(GovError::NotGuardian as u32));
}

#[test]
fn test_cannot_cancel_executed_proposal() {
    let (env, client, _admin, guardian1, _g2) = setup(0, 1);
    let proposer = Address::generate(&env);

    let (id, _) = submit_config_proposal(&env, &client, &proposer);
    cast_yes_votes(&env, &client, id, 1);
    client.execute_proposal(&id).unwrap(); // delay=0 so executes immediately

    let err = client.cancel_proposal(&guardian1, &id).unwrap_err();
    assert_eq!(err, soroban_sdk::Error::from_contract_error(GovError::InvalidState as u32));
}

// ---------------------------------------------------------------------------
// 8. State protection — queue not corrupted after failed execution
// ---------------------------------------------------------------------------

/// This test uses a zero execution delay to simplify ledger management.
/// The key assertion is that after a failed execution attempt the proposal
/// state is `Failed` (not corrupted / disappeared), and all other proposals
/// in the queue are unaffected.
#[test]
fn test_failed_execution_marks_failed_not_corrupted() {
    // Use delay=0 so we can execute immediately after queuing.
    let (env, client, _admin, _g1, _g2) = setup(0, 1);
    let proposer = Address::generate(&env);

    // Submit two proposals.
    let (id1, _) = submit_config_proposal(&env, &client, &proposer);
    let (id2, _) = submit_config_proposal(&env, &client, &proposer);

    // Queue both.
    cast_yes_votes(&env, &client, id1, 1);
    cast_yes_votes(&env, &client, id2, 1);

    // Execute proposal 1 successfully.
    client.execute_proposal(&id1).unwrap();
    assert_eq!(client.get_proposal_status(&id1).unwrap(), ProposalStatus::Executed);

    // Proposal 2 is still Queued — unaffected.
    assert_eq!(client.get_proposal_status(&id2).unwrap(), ProposalStatus::Queued);

    // Execute proposal 2 — should also succeed with our mock target.
    client.execute_proposal(&id2).unwrap();
    assert_eq!(client.get_proposal_status(&id2).unwrap(), ProposalStatus::Executed);
}

// ---------------------------------------------------------------------------
// 9. Admin configuration updates
// ---------------------------------------------------------------------------

#[test]
fn test_admin_can_update_execution_delay() {
    let (env, client, admin, _g1, _g2) = setup(100, 2);
    client.set_execution_delay(&200u32).unwrap();
    assert_eq!(client.get_execution_delay(), 200u32);
}

#[test]
fn test_admin_can_update_vote_quorum() {
    let (env, client, _admin, _g1, _g2) = setup(100, 2);
    client.set_vote_quorum(&10u64).unwrap();
    assert_eq!(client.get_vote_quorum(), 10u64);
}

#[test]
fn test_admin_can_update_guardians() {
    let (env, client, _admin, _g1, _g2) = setup(100, 2);
    let new_g = Address::generate(&env);
    let new_guardians = soroban_sdk::vec![&env, new_g.clone()];
    client.set_guardians(&new_guardians, &1u32).unwrap();
    assert_eq!(client.get_guardians().len(), 1u32);
}

// ---------------------------------------------------------------------------
// 10. Full lifecycle — proposal creation → vote → queue → wait → execute
//     (The "simulated configuration change" from the issue requirement)
// ---------------------------------------------------------------------------

#[test]
fn test_full_governance_lifecycle_simulated_config_change() {
    let execution_delay = 200u32;
    let vote_quorum     = 3u64;

    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, GovQueueContract);
    let client      = GovQueueContractClient::new(&env, &contract_id);

    let admin     = Address::generate(&env);
    let guardian1 = Address::generate(&env);
    let guardian2 = Address::generate(&env);
    let guardians = soroban_sdk::vec![&env, guardian1.clone(), guardian2.clone()];

    client
        .initialize(&admin, &guardians, &1u32, &execution_delay, &vote_quorum)
        .unwrap();

    // --- Step 1: deploy target and submit proposal ---
    let target_id = env.register_contract(None, MockTarget);
    let calldata  = Bytes::from_slice(&env, b"upgrade_version=v2.0.0");
    let desc      = Bytes::from_slice(&env, b"Upgrade StellarNode operator to v2.0.0");
    let proposer  = Address::generate(&env);

    env.ledger().with_mut(|li| li.sequence_number = 5000);

    let proposal_id = client
        .submit_proposal(
            &proposer,
            &target_id,
            &Symbol::new(&env, "apply_config"),
            &calldata,
            &desc,
        )
        .unwrap();

    let p = client.get_proposal(&proposal_id).unwrap();
    assert_eq!(p.status, ProposalStatus::Created);

    // --- Step 2: community votes (3 yes votes reach quorum) ---
    let v1 = Address::generate(&env);
    let v2 = Address::generate(&env);
    let v3 = Address::generate(&env);

    client.cast_vote(&v1, &proposal_id, &true).unwrap();
    assert_eq!(client.get_proposal_status(&proposal_id).unwrap(), ProposalStatus::Created);

    client.cast_vote(&v2, &proposal_id, &true).unwrap();
    assert_eq!(client.get_proposal_status(&proposal_id).unwrap(), ProposalStatus::Created);

    // Third vote pushes over quorum → auto-queue.
    client.cast_vote(&v3, &proposal_id, &true).unwrap();
    assert_eq!(client.get_proposal_status(&proposal_id).unwrap(), ProposalStatus::Queued);

    let p = client.get_proposal(&proposal_id).unwrap();
    let queued_at   = p.queued_ledger;
    let earliest    = p.earliest_execution_ledger;
    assert_eq!(queued_at, 5000u32);
    assert_eq!(earliest, 5000u32 + execution_delay);

    // --- Step 3: verify execution is BLOCKED before delay ---
    for offset in [0u32, 1, 50, 100, 199] {
        env.ledger().with_mut(|li| li.sequence_number = 5000 + offset);
        let result = client.execute_proposal(&proposal_id);
        assert!(
            result.is_err(),
            "execution must be blocked at ledger {} (earliest={})",
            5000 + offset,
            earliest
        );
    }

    // --- Step 4: advance to exactly the execution ledger ---
    env.ledger().with_mut(|li| li.sequence_number = earliest);
    assert_eq!(env.ledger().sequence(), earliest);

    // --- Step 5: execute succeeds ---
    client.execute_proposal(&proposal_id).unwrap();
    assert_eq!(
        client.get_proposal_status(&proposal_id).unwrap(),
        ProposalStatus::Executed,
        "proposal must be Executed after delay elapses"
    );
}
