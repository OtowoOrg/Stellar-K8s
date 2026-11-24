//! Example script demonstrating a complete governance proposal lifecycle.
//!
//! Run with:
//!
//! ```text
//! cargo run --example lifecycle
//! ```
//!
//! It walks a simulation through: contract initialisation -> proposal
//! creation -> weighted voting (including a vote flip) -> settlement ->
//! mandatory execution timelock -> final on-chain execution.

use governance_vote::{
    GovernanceVoteClient, proposal::{Proposal, ProposalState, Vote},
};
use soroban_sdk::{
    symbol_short, testutils::{Address as _, Ledger as _}, Address, Env,
};

fn main() {
    let env = Env::default();

    // --- Initialise -------------------------------------------------------
    let admin = Address::generate(&env);
    let voters = (
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    );

    // 10_000 units of total staked/participating weight;
    // 60% quorum required (6000 basis points);
    // proposals abandoned > 100 ledgers past their deadline may expire.
    let contract_id =
        env.register(governance_vote::GovernanceVote, (&admin, 10_000i128, 6000u32, 100u32));
    let client = GovernanceVoteClient::new(&env, &contract_id);
    env.mock_all_auths_allowing_non_root_auth();

    println!("Governance contract deployed at {contract_id:?}");
    println!("  total_weight          = {}", client.total_weight());
    println!("  quorum (bps)          = {}", client.quorum_percent_bps());
    println!("  quorum threshold      = {}", client.current_quorum_threshold());

    // --- Proposal creation ------------------------------------------------
    env.ledger().set_sequence_number(100);
    let id = client.propose(
        &admin,
        &symbol_short!("raise_cap"),    // title
        &symbol_short!("tx_fee"),        // parameter under change
        &250,                            // proposed new value
        &100,                            // voting window (ledgers)
        &50,                             // execution timelock (ledgers)
    );
    println!("\nProposal #{id} created: change `tx_fee` -> 250 (voting open until ledger {}).", 200);
    println!("  initial state = {:?}", client.get_proposal(&id).unwrap().state);

    // --- Voting (with a vote flip) ----------------------------------------
    client.vote(&voters.0, &id, &Vote::For, &1500);
    client.vote(&voters.1, &id, &Vote::For, &500);
    // Flip voter 1 from 500 For to 3000 For.
    client.vote(&voters.1, &id, &Vote::For, &3000);
    client.vote(&voters.2, &id, &Vote::Abstain, &2500);
    print_proposal(&client, &id);

    // --- Settle as soon as voting closes ----------------------------------
    env.ledger().set_sequence_number(200);
    client.finalize(&id);
    let p = client.get_proposal(&id).unwrap();
    println!("\nVoting closed; finalized state = {:?}", p.state);
    assert_eq!(p.state, ProposalState::Passed);
    println!("  timelock starts at ledger {} ; executable from ledger {}.",
             p.pass_ledger.unwrap(), p.pass_ledger.unwrap() + 50);

    // --- Timelock enforcement ---------------------------------------------
    env.ledger().set_sequence_number(249);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| client.execute(&id)));
    assert!(result.is_err(), "execute() before the timelock must fail");
    println!("\nAttempted execution at ledger 249 -> rejected by the timelock (as required).");

    // --- Executable once the timelock elapses -----------------------------
    env.ledger().set_sequence_number(250);
    client.execute(&id);
    let p = client.get_proposal(&id).unwrap();
    println!("\nExecuted at ledger 250.");
    println!("  proposal state = {:?}", p.state);
    println!("  `tx_fee` now   = {:?}", client.get_parameter(&symbol_short!("tx_fee")));
    assert_eq!(p.state, ProposalState::Executed);
    assert_eq!(client.get_parameter(&symbol_short!("tx_fee")), Some(250));

    println!("\nComplete proposal lifecycle demonstrated successfully.");
}

fn print_proposal(client: &GovernanceVoteClient<'_>, id: &u32) {
    let p: Proposal = client.get_proposal(id).unwrap();
    println!("\n  for     = {}", p.for_weight);
    println!("  against = {}", p.against_weight);
    println!("  abstain = {}", p.abstain_weight);
    println!("  voted   = {}", p.total_voted_weight);
    println!("  state   = {:?}", p.state);
}
