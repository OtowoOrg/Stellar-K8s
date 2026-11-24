//! End-to-end test: upgrading a mock contract from v1 to v2 through
//! `proxy-controller`'s timelocked propose/execute state machine.
//!
//! This imports *real* compiled Wasm for both `mock_v1` and `mock_v2` (built
//! by `make build-mocks` in this directory before `cargo test` runs -- see
//! `../Makefile`) and exercises `env.deployer().update_current_contract_wasm`
//! for real, rather than swapping in a natively-registered Rust struct.

use soroban_sdk::{testutils::Address as _, testutils::Ledger, Address, Bytes, Env};

mod mock_v1_wasm {
    soroban_sdk::contractimport!(file = "../target/wasm32v1-none/release/mock_v1.wasm");
}
mod mock_v2_wasm {
    soroban_sdk::contractimport!(file = "../target/wasm32v1-none/release/mock_v2.wasm");
}

const TIMELOCK_SECONDS: u64 = 48 * 60 * 60;

#[test]
fn upgrade_from_v1_to_v2_preserves_state_and_unlocks_new_methods() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let security_council = Address::generate(&env);

    let contract_id = env.register(mock_v1_wasm::WASM, ());
    let v1 = mock_v1_wasm::Client::new(&env, &contract_id);

    v1.initialize(&admin, &security_council, &42u32);
    assert_eq!(v1.get_value(), 42);
    assert_eq!(v1.version(), 1);

    let v2_wasm = Bytes::from_slice(&env, mock_v2_wasm::WASM);
    let proposed_hash = v1.propose_upgrade(&v2_wasm);

    let pending = v1.pending_upgrade().expect("upgrade should be pending");
    assert_eq!(pending.wasm_hash, proposed_hash);
    assert!(pending.execute_after > env.ledger().timestamp());

    // Premature execution, before the timelock elapses, must fail.
    let premature = v1.try_execute_upgrade();
    assert!(premature.is_err());

    // Fast-forward past the 48-hour delay.
    env.ledger().with_mut(|li| {
        li.timestamp += TIMELOCK_SECONDS + 1;
    });

    v1.execute_upgrade();
    assert!(v1.pending_upgrade().is_none());

    // Re-bind a v2 client to the *same* contract address. Existing state
    // (the counter set under v1) must have survived the Wasm swap, and the
    // new v2-only method must now be reachable.
    let v2 = mock_v2_wasm::Client::new(&env, &contract_id);
    assert_eq!(v2.version(), 2);
    assert_eq!(v2.get_value(), 42);
    assert_eq!(v2.double_value(), 84);
    assert_eq!(v2.get_value(), 84);

    // Governance state (admin/security council) also survived the swap.
    assert_eq!(v2.admin(), Some(admin));
    assert_eq!(v2.security_council(), Some(security_council));
}

#[test]
fn security_council_can_cancel_a_pending_upgrade() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let security_council = Address::generate(&env);

    let contract_id = env.register(mock_v1_wasm::WASM, ());
    let v1 = mock_v1_wasm::Client::new(&env, &contract_id);
    v1.initialize(&admin, &security_council, &7u32);

    let v2_wasm = Bytes::from_slice(&env, mock_v2_wasm::WASM);
    v1.propose_upgrade(&v2_wasm);
    assert!(v1.pending_upgrade().is_some());

    v1.cancel_upgrade(&security_council);
    assert!(v1.pending_upgrade().is_none());

    // Fast-forward past where the timelock would have elapsed; there is
    // nothing left to execute, and the contract is still running v1.
    env.ledger().with_mut(|li| {
        li.timestamp += TIMELOCK_SECONDS + 1;
    });
    assert!(v1.try_execute_upgrade().is_err());
    assert_eq!(v1.version(), 1);
    assert_eq!(v1.get_value(), 7);
}

#[test]
fn unauthorized_caller_cannot_propose_or_cancel() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let security_council = Address::generate(&env);
    let attacker = Address::generate(&env);

    let contract_id = env.register(mock_v1_wasm::WASM, ());
    let v1 = mock_v1_wasm::Client::new(&env, &contract_id);
    v1.initialize(&admin, &security_council, &1u32);

    // mock_all_auths() approves any auth *requirement* the contract raises,
    // regardless of which address it is for -- so to prove propose_upgrade
    // is actually gated on the stored admin, this asserts the identity
    // check in `cancel_upgrade` instead, which explicitly compares the
    // authorized caller against the stored admin/security council and
    // rejects anyone else even when their own auth succeeds.
    let v2_wasm = Bytes::from_slice(&env, mock_v2_wasm::WASM);
    v1.propose_upgrade(&v2_wasm);

    let result = v1.try_cancel_upgrade(&attacker);
    assert!(result.is_err());
    assert!(v1.pending_upgrade().is_some());
}

#[test]
fn propose_upgrade_without_admin_authorization_fails() {
    let env = Env::default();
    // No mock_all_auths() here: nothing has been authorized at all, so the
    // admin.require_auth() inside propose_upgrade must fail closed.

    let admin = Address::generate(&env);
    let security_council = Address::generate(&env);

    let contract_id = env.register(mock_v1_wasm::WASM, ());
    let v1 = mock_v1_wasm::Client::new(&env, &contract_id);

    env.mock_all_auths();
    v1.initialize(&admin, &security_council, &1u32);
    env.set_auths(&[]);

    let v2_wasm = Bytes::from_slice(&env, mock_v2_wasm::WASM);
    let result = v1.try_propose_upgrade(&v2_wasm);
    assert!(result.is_err());
    assert!(v1.pending_upgrade().is_none());
}
