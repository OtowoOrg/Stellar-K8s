//! Property-based tests verifying the total-solvency invariant of the vault
//! under arbitrary, adversarial operation sequences.
//!
//! Invariants checked after *every* operation (success or failure):
//!   1. `token_balance == liabilities`  (every liability is fully backed);
//!   2. per-position conservation: `deposit == slashed + released + remainder`;
//!   3. `slashed_reserve == Σ slashed` over all positions;
//!   4. no negative balances anywhere; terminal positions hold no remainder.
//!
//! A separate wind-down test proves no funds can ever become permanently stuck.

use std::collections::HashMap;

use ed25519_dalek::SigningKey;
use proptest::collection::vec as prop_vec;
use proptest::prelude::*;

use stellar_escrow_vault::slashing::test_support::sign_downtime;
use stellar_escrow_vault::{Address, Vault, VaultConfig};

const LOCKUP: u64 = 100;

fn addr(b: u8) -> Address {
    Address::from_bytes([b; 32])
}

fn operator_of(i: usize) -> Address {
    addr((i as u8 % 4).wrapping_add(3))
}

fn config() -> VaultConfig {
    VaultConfig {
        admin: addr(1),
        notifier: addr(2),
        lockup_window: LOCKUP,
        dispute_window: 30,
        slashing_staleness: 50,
        downtime_slash_bps: 6_000, // partial, to exercise remainder releases
        double_sign_slash_bps: 10_000,
        reporters: HashMap::new(),
    }
}

/// One step in a random, adversarial operation sequence.
#[derive(Debug, Clone)]
enum Op {
    Deposit,
    CreditYield { amount: i128 },
    Distribute,
    ClaimYield,
    DowntimeSlash,
    Release,
    ClaimSlashed,
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        Just(Op::Deposit),
        any::<u16>().prop_map(|a| Op::CreditYield {
            amount: i128::from(a) + 1
        }),
        Just(Op::Distribute),
        Just(Op::ClaimYield),
        Just(Op::DowntimeSlash),
        Just(Op::Release),
        Just(Op::ClaimSlashed),
    ]
}

fn ops() -> impl Strategy<Value = Vec<Op>> {
    prop_vec(op_strategy(), 0..120)
}

/// Total-solvency invariant checked after every single operation.
fn check_invariants(v: &Vault) -> Result<(), String> {
    v.assert_solvent()
        .map_err(|e| format!("solvency violation: {e:?}"))?;

    for p in v.positions() {
        let accounted = p.slashed + p.claimed + p.released + p.remainder();
        if accounted != p.deposit {
            return Err(format!(
                "position {} conservation broken: deposit {} != {p:?}",
                p.id, p.deposit
            ));
        }
        if p.unclaimed_yield < 0 {
            return Err("negative unclaimed yield".into());
        }
        if matches!(
            p.status,
            stellar_escrow_vault::PositionStatus::Released
                | stellar_escrow_vault::PositionStatus::Forfeited
        ) && p.remainder() != 0
        {
            return Err("terminal position still holds collateral".into());
        }
    }

    let sum_slashed: i128 = v.positions().map(|p| p.slashed).sum();
    if v.slashed_reserve() != sum_slashed {
        return Err(format!(
            "slashed reserve {} != Σ slashed {}",
            v.slashed_reserve(),
            sum_slashed
        ));
    }
    Ok(())
}

proptest! {
    #[test]
    fn total_solvency_under_random_ops(ops in ops()) {
        let mut vault = Vault::initialize(config()).unwrap();
        let notifier = addr(2);
        let keeper = addr(10);
        let reporter = SigningKey::from_bytes(&[9u8; 32]);
        vault.add_reporter(addr(1), addr(9), reporter.verifying_key().to_bytes()).unwrap();

        // Pre-fund the simulated external actors.
        for i in 0..4 {
            vault.fund(operator_of(i), 1_000_000).unwrap();
        }
        vault.fund(keeper, 1_000_000).unwrap();

        // Track created positions (first-op timestamp, operator, id).
        let mut positions: Vec<(Address, u64)> = Vec::new();

        let mut yield_credited: i128 = 0;
        let mut yield_claimed: i128 = 0;

        for (i, op) in ops.iter().enumerate() {
            match op {
                Op::Deposit => {
                    let amount = (i as i128 % 100) + 1;
                    let op_addr = operator_of(i);
                    let node = addr((i as u8).wrapping_add(0x40));
                    if let Ok(id) = vault.deposit(op_addr, node, [0u8; 32], amount, 0) {
                        positions.push((op_addr, id));
                    }
                }
                Op::CreditYield { amount } => {
                    if vault.credit_yield(keeper, *amount).is_ok() {
                        yield_credited += *amount;
                    }
                }
                Op::Distribute => {
                    let _ = vault.distribute_yield();
                }
                Op::ClaimYield => {
                    if let Some(&(op_addr, id)) = positions.first() {
                        if let Ok(amt) = vault.claim_yield(op_addr, id) {
                            yield_claimed += amt;
                        }
                    }
                }
                Op::DowntimeSlash => {
                    if let Some(&(_, id)) = positions.get(i % positions.len().max(1)) {
                        let node = vault.position(id).map(|p| p.node_id);
                        if let Some(node_id) = node {
                            let proof = sign_downtime(&reporter, addr(9), node_id, [5u8; 32]);
                            let _ = vault.submit_downtime_proof(proof, 5);
                        }
                    }
                }
                Op::Release => {
                    if let Some(&(op_addr, id)) = positions.get(i % positions.len().max(1)) {
                        let _ = vault.release_collateral(op_addr, id, LOCKUP);
                    }
                }
                Op::ClaimSlashed => {
                    if let Some(&(_, id)) = positions.first() {
                        let _ = vault.claim_slashed(notifier, id);
                    }
                }
            }

            // Invariant must hold after *every* operation, whatever its outcome.
            check_invariants(&vault).expect("invariant violated mid-sequence");

            // Yield is never created or destroyed: all credited yield that has
            // not been claimed must equal pool + Σ unclaimed.
            let unclaimed: i128 = vault.positions().map(|p| p.unclaimed_yield).sum();
            prop_assert_eq!(
                yield_credited - yield_claimed,
                vault.yield_pool() + unclaimed,
                "yield accounting is not conservative"
            );
        }
    }
}

#[test]
fn wind_down_leaves_no_funds_stuck() {
    let mut vault = Vault::initialize(config()).unwrap();
    let notifier = addr(2);
    let admin = addr(1);
    let keeper = addr(10);
    let reporter = SigningKey::from_bytes(&[9u8; 32]);
    vault
        .add_reporter(addr(1), addr(9), reporter.verifying_key().to_bytes())
        .unwrap();
    vault.fund(keeper, 1_000_000).unwrap();

    // Two operators; one gets fully slashed (downtime), the other stays honest.
    for (k, amt) in [(3u8, 5000i128), (4u8, 7000i128)] {
        vault.fund(addr(k), amt).unwrap();
        vault
            .deposit(addr(k), addr(k * 3), [0u8; 32], amt, 0)
            .unwrap();
    }

    let node_b = addr(9); // operator addr(3) deposited collateral for this node
    let proof = sign_downtime(&reporter, addr(9), node_b, [7u8; 32]);
    let _ = vault.submit_downtime_proof(proof, 5);
    // (downtime_slash_bps=6000 => operator 3 slashed 60% of 5000 => 3000,
    //  remainder 2000; operator 4 stays honest with the full 7000)
    vault.credit_yield(keeper, 1234).unwrap();
    vault.distribute_yield().unwrap();

    // Wind down: release every operator's remainder, claim all slashed, claim
    // all yield, and reclaim any idle pool. Each pull may legitimately be a
    // no-op if there was nothing to rate-claim; that is expected and fine.
    let ids: Vec<u64> = vault.positions().map(|p| p.id).collect();
    for id in ids {
        let op = vault.position(id).unwrap().operator;
        let at = vault.position(id).unwrap().locked_at + LOCKUP;
        let _ = vault.release_collateral(op, id, at);
        let _ = vault.claim_slashed(notifier, id);
        let _ = vault.claim_yield(op, id);
    }
    let _ = vault.reclaim_yield_pool(admin);

    // After every liability is pulled, the vault holds nothing.
    check_invariants(&vault).unwrap();
    assert_eq!(vault.token_balance(), 0, "funds are permanently stuck");
    assert_eq!(vault.liabilities(), 0);
}
