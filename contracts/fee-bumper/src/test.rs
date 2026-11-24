use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Bytes, Env,
};
use std::vec::Vec;
use crate::FeeBumperClient;

const BASE_FEE: u64 = 100_000;
const FEE_BUMP_CAP: u64 = 250_000;
const MAX_FEE: u64 = 400_000;
const RATE_LIMIT: u64 = 5;
const RATE_WINDOW: u64 = 100;
const TOTAL_ACCOUNTS: usize = 10;
const TXS_PER_ACCOUNT: usize = 10;

fn setup() -> (Env, FeeBumperClient<'static>) {
    let env = Env::default();
    let contract_id = env.register_contract(None, crate::FeeBumper);
    let client = FeeBumperClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let token = Address::generate(&env);

    client.initialize(&admin, &FEE_BUMP_CAP, &token, &RATE_LIMIT, &RATE_WINDOW);

    (env, client)
}

fn advance_ledger(env: &Env, seq: u64) {
    env.ledger().set(LedgerInfo {
        sequence_number: seq,
        ..Default::default()
    });
}

fn make_tx_bytes(env: &Env, account_idx: u32, tx_idx: u32) -> Bytes {
    let mut data = Bytes::new(env);
    let mut buf = [0u8; 8];
    buf[..4].copy_from_slice(&account_idx.to_be_bytes());
    buf[4..].copy_from_slice(&tx_idx.to_be_bytes());
    data.extend_from_slice(&buf);
    data
}

#[test]
fn test_initialize_sets_config() {
    let (env, client) = setup();
    let config = client.get_config();
    assert_eq!(config.fee_bump_cap, FEE_BUMP_CAP);
    assert_eq!(config.rate_limit, RATE_LIMIT);
    assert_eq!(config.rate_window, RATE_WINDOW);
}

#[test]
fn test_underfunded_rejected() {
    let (env, client) = setup();
    advance_ledger(&env, 1000);
    let account = Address::generate(&env);
    let bytes = make_tx_bytes(&env, 0, 0);

    let result = client.try_wrap_fee_bump(&account, &bytes, &BASE_FEE, &50_000);
    assert!(result.is_err());
}

#[test]
fn test_invalid_tx_rejected() {
    let (env, client) = setup();
    let account = Address::generate(&env);
    let bytes = Bytes::from_array(&env, &[1u8, 2, 3]);

    let result = client.try_wrap_fee_bump(&account, &bytes, &BASE_FEE, &MAX_FEE);
    assert!(result.is_err());
}

#[test]
fn test_valid_wrap_succeeds() {
    let (env, client) = setup();
    advance_ledger(&env, 1000);
    let account = Address::generate(&env);
    let bytes = make_tx_bytes(&env, 0, 0);

    let result = client.wrap_fee_bump(&account, &bytes, &BASE_FEE, &MAX_FEE);
    assert!(result.success);
    assert_eq!(result.fee_charged, BASE_FEE + FEE_BUMP_CAP);
}

#[test]
fn test_replay_protection() {
    let (env, client) = setup();
    advance_ledger(&env, 1000);
    let account = Address::generate(&env);
    let bytes = make_tx_bytes(&env, 0, 0);

    let _ = client.wrap_fee_bump(&account, &bytes, &BASE_FEE, &MAX_FEE);

    let result = client.try_wrap_fee_bump(&account, &bytes, &BASE_FEE, &MAX_FEE);
    assert!(result.is_err());
}

#[test]
fn test_sequential_100_txs() {
    let (env, client) = setup();
    advance_ledger(&env, 1000);

    let mut accounts = Vec::new();
    for _ in 0..TOTAL_ACCOUNTS {
        accounts.push(Address::generate(&env));
    }

    let mut success_count: u64 = 0;
    let mut rate_limited_count: u64 = 0;

    for acct_idx in 0..TOTAL_ACCOUNTS {
        let account = &accounts[acct_idx];
        for tx_idx in 0..TXS_PER_ACCOUNT {
            let bytes = make_tx_bytes(&env, acct_idx as u32, tx_idx as u32);
            let result = client.try_wrap_fee_bump(account, &bytes, &BASE_FEE, &MAX_FEE);

            if result.is_ok() {
                success_count += 1;
            } else {
                rate_limited_count += 1;
            }
        }
    }

    assert_eq!(success_count, TOTAL_ACCOUNTS as u64 * RATE_LIMIT);
    assert_eq!(
        rate_limited_count,
        TOTAL_ACCOUNTS as u64 * (TXS_PER_ACCOUNT as u64 - RATE_LIMIT)
    );
}

#[test]
fn test_reimbursement_tracking() {
    let (env, client) = setup();
    advance_ledger(&env, 1000);

    let account = Address::generate(&env);
    let expected_per_tx = BASE_FEE + FEE_BUMP_CAP;

    for i in 0..RATE_LIMIT {
        let bytes = make_tx_bytes(&env, 0, i as u32);
        let _ = client.wrap_fee_bump(&account, &bytes, &BASE_FEE, &MAX_FEE);
    }

    let claimable = client.claim_reimbursement(&account);
    assert_eq!(claimable, expected_per_tx * RATE_LIMIT);

    let after_claim = client.claim_reimbursement(&account);
    assert_eq!(after_claim, 0);
}

#[test]
fn test_rate_window_reset() {
    let (env, client) = setup();
    advance_ledger(&env, 1000);

    let account = Address::generate(&env);

    for i in 0..RATE_LIMIT {
        let bytes = make_tx_bytes(&env, 0, i as u32);
        let _ = client.wrap_fee_bump(&account, &bytes, &BASE_FEE, &MAX_FEE);
    }

    let blocked = client.try_wrap_fee_bump(
        &account,
        &make_tx_bytes(&env, 0, RATE_LIMIT as u32),
        &BASE_FEE,
        &MAX_FEE,
    );
    assert!(blocked.is_err());

    advance_ledger(&env, 1000 + RATE_WINDOW + 1);

    let recovered = client.try_wrap_fee_bump(
        &account,
        &make_tx_bytes(&env, 0, (RATE_LIMIT + 1) as u32),
        &BASE_FEE,
        &MAX_FEE,
    );
    assert!(recovered.is_ok());
}
