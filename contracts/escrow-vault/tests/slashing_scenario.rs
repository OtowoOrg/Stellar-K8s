//! Validation scripts simulating slashing-proof submittals and verifying that
//! non-faulty operators can withdraw 100% of their collateral once the lockup
//! expires.

use std::collections::HashMap;

use ed25519_dalek::SigningKey;

use stellar_escrow_vault::slashing::test_support::{double_sign_from, sign_downtime};
use stellar_escrow_vault::{Address, PositionStatus, SlashOutcome, Vault, VaultConfig, VaultError};

const LOCKUP: u64 = 100;
const STALENESS: u64 = 50;

fn addr(b: u8) -> Address {
    Address::from_bytes([b; 32])
}

/// A unique, valid store key placeholder derived from an index.
fn store_key(idx: u8) -> [u8; 32] {
    let mut k = [0u8; 32];
    k[0] = idx;
    k
}

fn config(slash_bps: u32) -> VaultConfig {
    VaultConfig {
        admin: addr(1),
        notifier: addr(2),
        lockup_window: LOCKUP,
        dispute_window: 30,
        slashing_staleness: STALENESS,
        downtime_slash_bps: slash_bps,
        double_sign_slash_bps: slash_bps,
        reporters: HashMap::new(),
    }
}

fn reporter_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

#[test]
fn non_faulty_operator_withdraws_100_percent_after_lockup_expiry() {
    let mut vault = Vault::initialize(config(10_000)).unwrap();
    let operator = addr(3);
    let node = addr(4);
    let deposit_amt: i128 = 25_000;

    vault.fund(operator, deposit_amt).unwrap();
    let id = vault
        .deposit(operator, node, [0u8; 32], deposit_amt, 0)
        .unwrap();

    let pos = vault.position(id).unwrap();
    assert_eq!(pos.status, PositionStatus::Locked);
    assert_eq!(pos.remainder(), deposit_amt);

    // A non-faulty operator does not get slashed; they wait out the lockup.
    assert_eq!(
        vault.release_collateral(operator, id, LOCKUP - 1),
        Err(VaultError::LockupNotExpired),
        "release must be blocked until the lockup expires"
    );

    // After expiry the operator pulls exactly 100% of their collateral.
    let released = vault.release_collateral(operator, id, LOCKUP).unwrap();
    assert_eq!(released, deposit_amt);
    assert_eq!(vault.position(id).unwrap().status, PositionStatus::Released);
    assert_eq!(
        vault.token_balance(),
        0,
        "vault must hold no leftover funds"
    );

    // Nothing is left stuck anywhere: operator recovered everything.
    assert_eq!(vault.slashed_reserve(), 0);
    assert_eq!(vault.yield_pool(), 0);
    vault.assert_solvent().unwrap();
}

#[test]
fn valid_downtime_proof_slashes_but_a_forgery_never_mutates() {
    let mut vault = Vault::initialize(config(10_000)).unwrap();
    let operator = addr(3);
    let node = addr(4);
    let deposit_amt: i128 = 10_000;

    // Only the authorized reporter key can trigger a slash.
    let reporter = reporter_key(9);
    let reporter_vk = reporter.verifying_key().to_bytes();
    vault.add_reporter(addr(1), addr(9), reporter_vk).unwrap();

    vault.fund(operator, deposit_amt).unwrap();
    let id = vault
        .deposit(operator, node, [0u8; 32], deposit_amt, 0)
        .unwrap();

    // 1) A forged proof (signature from an attacker not on the allowlist) is
    //    rejected and must not change the position.
    let forger = reporter_key(99);
    let forged = sign_downtime(&forger, addr(99), node, store_key(1));
    assert_eq!(
        vault.submit_downtime_proof(forged, 5),
        Err(VaultError::InvalidProof("reporter not authorized".into()))
    );
    assert_eq!(
        vault.position(id).unwrap().slashed,
        0,
        "a rejected proof must not slash anything"
    );

    // 2) A proof signed by the wrong node target must not slash another node.
    let other_node = addr(5);
    let wrong_target = sign_downtime(&reporter, addr(9), other_node, store_key(2));
    assert_eq!(
        vault.submit_downtime_proof(wrong_target, 5),
        Err(VaultError::PositionNotFound)
    );
    assert_eq!(vault.position(id).unwrap().slashed, 0);

    // 3) The authentic, fresh proof from an authorized reporter slashes 100%.
    let proof = sign_downtime(&reporter, addr(9), node, store_key(1));
    let outcome: SlashOutcome = vault.submit_downtime_proof(proof, 5).unwrap();
    assert_eq!(outcome.slashed_amount, deposit_amt);
    assert_eq!(outcome.remainder, 0);
    assert_eq!(
        vault.position(id).unwrap().status,
        PositionStatus::Forfeited
    );
    assert_eq!(vault.slashed_reserve(), deposit_amt);

    // 4) Slashed collateral is claimable (pull) by the notifier only.
    assert_eq!(
        vault.claim_slashed(addr(3), id),
        Err(VaultError::Unauthorized),
        "operators cannot pull slashed collateral"
    );
    assert_eq!(vault.claim_slashed(addr(2), id).unwrap(), deposit_amt);
    assert_eq!(vault.token_balance(), 0, "no funds remain stuck");
    vault.assert_solvent().unwrap();
}

#[test]
fn partial_slash_carves_collateral_and_operator_releases_the_rest() {
    // downtime_slash_bps = 5000 => exactly 50% is slashed for a fault.
    let mut vault = Vault::initialize(config(5_000)).unwrap();
    let operator = addr(3);
    let node = addr(4);
    let deposit_amt: i128 = 8_000;

    let reporter = reporter_key(9);
    vault
        .add_reporter(addr(1), addr(9), reporter.verifying_key().to_bytes())
        .unwrap();

    vault.fund(operator, deposit_amt).unwrap();
    let id = vault
        .deposit(operator, node, [0u8; 32], deposit_amt, 0)
        .unwrap();

    let proof = sign_downtime(&reporter, addr(9), node, store_key(1));
    let outcome = vault.submit_downtime_proof(proof, 5).unwrap();
    assert_eq!((outcome.slashed_amount, outcome.remainder), (4_000, 4_000));
    assert_eq!(vault.position(id).unwrap().status, PositionStatus::Slashed);

    // The un-slashed remainder is still released to the operator on expiry.
    assert_eq!(
        vault.release_collateral(operator, id, LOCKUP).unwrap(),
        4_000
    );
    assert_eq!(vault.position(id).unwrap().status, PositionStatus::Released);
    assert_eq!(vault.claim_slashed(addr(2), id).unwrap(), 4_000);
    assert_eq!(vault.token_balance(), 0, "no funds remain stuck");
    vault.assert_solvent().unwrap();
}

#[test]
fn double_sign_proof_is_self_incriminating_and_slashes() {
    let mut vault = Vault::initialize(config(10_000)).unwrap();
    let operator = addr(3);
    let node = addr(4);
    let deposit_amt: i128 = 20_000;

    // The node's own consensus key is registered at deposit time.
    let node_consensus_key = reporter_key(7);
    let node_vk = node_consensus_key.verifying_key().to_bytes();

    vault.fund(operator, deposit_amt).unwrap();
    let id = vault
        .deposit(operator, node, node_vk, deposit_amt, 0)
        .unwrap();

    // Two conflicting, correctly-signed messages for the same slot prove
    // double-signing; this is the only way both verify against the node key.
    let slot = 4242u64;
    let material = double_sign_from(&node_consensus_key, node, slot);
    let outcome = vault
        .submit_double_sign_proof(&material, node, slot, 5)
        .unwrap();
    assert_eq!(outcome.slashed_amount, deposit_amt);
    assert_eq!(
        vault.position(id).unwrap().status,
        PositionStatus::Forfeited
    );
    assert_eq!(vault.claim_slashed(addr(2), id).unwrap(), deposit_amt);
    assert_eq!(vault.token_balance(), 0);
    vault.assert_solvent().unwrap();
}

#[test]
fn dispute_freezes_release_and_admin_adjudicates() {
    let mut vault = Vault::initialize(config(10_000)).unwrap();
    let operator = addr(3);
    let node = addr(4);
    vault.fund(operator, 5_000).unwrap();
    let id = vault.deposit(operator, node, [0u8; 32], 5_000, 0).unwrap();

    // A dispute freezes release; the operator cannot pull collateral early.
    vault.open_dispute(addr(1), id, 10).unwrap();
    assert_eq!(vault.position(id).unwrap().status, PositionStatus::Disputed);
    assert_eq!(
        vault.release_collateral(operator, id, LOCKUP),
        Err(VaultError::WrongStatus("disputed".into()))
    );

    // Dismissed => back to Locked and the operator recovers 100% at expiry.
    vault.resolve_dispute(addr(1), id, false, 10).unwrap();
    assert_eq!(vault.position(id).unwrap().status, PositionStatus::Locked);
    assert_eq!(
        vault.release_collateral(operator, id, LOCKUP).unwrap(),
        5_000
    );
    assert_eq!(vault.token_balance(), 0);
    vault.assert_solvent().unwrap();
}
