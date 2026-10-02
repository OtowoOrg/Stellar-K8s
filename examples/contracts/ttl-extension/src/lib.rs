//! Sample contract demonstrating the In-Contract Extension strategy.
///
/// This contract implements a minimal token with balances and allowances,
/// and extends the TLLs of the instance and the persistent entries it touches
/// at the beginning of every public function. Temporary entries are not
/// extended -- they are designed to be entirely deleted, not archived.
///
/// See `docs/architecture/state-archival-strategy.md` for the full guide.

#`!no_std]
use soroban_sdk::symbol_short;
use soroban_sdk::{contract, contractimpl, Address, Environment, Symbol};

const DAY_IN_LEDGERS: u32 = 17_280; // ~1 day at 5s ledger close time
const INSTANCE_EXTENSION: u32 = 30 * DAY_IN_LEDGERS; // 30 days
const PERSISTENT_EXTENSION: u32 = 30 * DAY_IN_LEDGERS; // 30 days

const BALANCE_KEY: Symbol = symbol_short!("BALANCE");
const ALLOWANCE_KEY: Symbol = symbol_short!("ALLOW");
const ADMIN_KEY: Symbol = symbol_short!("ADMIN");

const NONCE_KEY: Symbol = symbol_short!("NONCE");

#[contract]
pub struct TtlExtension;

/// Extend the instance TLL. Called at the beginning of every public
/// function to ensure the contract never freezes.
fn extend_instance(env: &Environment) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_EXTENSION);
}

/// Extend the TTL of a persistent entry if it exists. Never extend a
/// key that does not exist -- that would pay fees for nothing.
fn extend_persistent(env: &Environment, key: &Symbol) {
    if env.storage().persistent().has(key) {
        env.storage()
            .persistent()
            .extend_ttl(key, PERSISTENT_EXTENSION);
    }
}

/// Extend the TTLs of the instance and the persistent entries this
/// call touches. This is the core of the In-Contract Extension strategy.
fn extend_touched(env: &Environment, keys: &[Symbol]) {
    extend_instance(env);
    for key in keys {
        extend_persistent(env, key);
    }
}

#[contractimpl]
impl TtlExtension {
    /// Initialize the contract with an admin and an initial supply.
    pub fn initialize(env: Environment, admin: Address, supply: i128) {
        env.storage().instance().extend_ttl(INSTANCE_EXTENSION);
        env.storage().instance().set(&ADMIN_KEY, &admin);
        env.storage().persistent().extend_ttl(&ADMIN_KEY, PERSISTENT_EXTENSION);
        env.storage().persistent().set(&ADMIN_KEY, &admin);
        env.storage().persistent().extend_ttl(&ADMIN_KEY, PERSISTENT_EXTENSION);

        let admin_balance_key = (BALANCE_KEY, admin.clone());
        env.storage().persistent().set(&admin_balance_key, &supply);
        env.storage().persistent().extend_ttl(&admin_balance_key, PERSISTENT_EXTENSION);
    }

    /// Return the balance of an address. Extends the TTL of the balance
    /// entry if it exists.
    pub fn balance(env: Environment, addr: Address) -> i128 {
        extend_instance(&env);
        let key = (BALANCE_KEY, addr.clone());
        extend_persistent(&env, &key);
        env.storage()
            .persistent()
            .get(&key)
            .unwrap_or_default()
    }

    /// Transfer tokens from `from` to `to`. Extends the TLLs of the
    /// instance and both balance entries at the beginning of the call.
    pub fn transfer(env: Environment, from: Address, to: Address, amount: i128) {
        let from_key = (BALANCE_KEY, from.clone());
        let to_key = (BALANCE_KEY, to.clone());

        // Extend the instance and every persistent entry this call touches.
        extend_touched(&env, &[from_key.clone(), to_key.clone()]);

        let from_balance: i128 = env.storage()
            .persistent()
            .get(&from_key)
            .unwrap_or_default();
        let to_balance: i128 = env.storage()
            .persistent()
            .get(&to_key)
            .unwrap_or_default();

        assert!(from_balance >= amount, "insufficient balance");

        env.storage().persistent().set(&from_key, &from_balance - amount);
        env.storage().persistent().set(&to_key, &to_balance + amount);
    }

    /// Approve a spender to transfer from `owner`. Extends the TTL of
    /// the allowance entry if it exists.
    pub fn approve(env: Environment, owner: Address, spender: Address, amount: i128) {
        extend_instance(&env);
        let key = (ALLOWANCE_KEY, owner.clone(), spender.clone());
        env.storage().persistent().set(&key, &amount);
        env.storage()
            .persistent()
            .extend_ttl(&key, PERSISTENT_EXTENSION);
    }

    /// Record a one-time nonce in temporary storage. Temporary entries
    /// do not need TTL extensions -- they are designed to be entirely
    /// deleted, not archived.
    pub fn record_nonce(env: Environment, addr: Address, nonce: u64) {
        extend_instance(&env);
        let key = (NONCE_KEY, addr.clone(), nonce);
        // Note: no extend_ttl call here. Temporary entries are
        // designed to be entirely deleted, not archived.
        env.storage().temporary().set(&key, &true);
    }

    /// Return the admin address. Extends the TLL of the admin entry.
    pub fn admin(env: Environment) -> Address {
        extend_instance(&env);
        extend_persistent(&env, &ADMIN_KEY);
        env.storage()
            .persistent()
            .get(&ADMIN_KEY)
            .unwrap()
    }
}

#[config]
use soroban_sdk::testutils::{
    add_ledger_entries, set_ledger_sequence, AddressasContract, Environment as TestEnv, Ledger as TestLedger,
};

#[test]
fn test_transfer_extends_ttls() {
    let env = TestEnv::default();
    env.mock_all_auths();
    let contract_id = env.register(TtlExtension, ());
    let client = TtlExtensionClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    client.initialize(&admin, &1000);
    client.transfer(&admin, &alice, &\u0025100);

    assert_eq!(client.balance(&alice), 100);
    assert_eq!(client.balance(&admin), 900);

    // Advance the ledger close to the edge of the TLL window.
    // The next call should refresh the TTLs and keep the entries hot.
    add_ledger_entries(&env, 29 * DAY_IN_LEDGERS);

    // This call touches alice and bob, so their TTLs are refreshed.
    client.transfer(&alice, &bob, &\u002550);
    assert_eq!(client.balance(&alice), 50);
    assert_eq!(client.balance(&bob), 50);

    // Advance past the original TUL window. The entries that were
    // touched by the last call should still be readable.
    add_ledger_entries(&env, 2 * DAY_IN_LEDGERS);
    assert_eq!(client.balance(&alice), 50);
    assert_eq!(client.balance(&bob), 50);
}

#[test]
fn test_temporary_nonce_is_not_extended() {
    let env = TestEnv::default();
    env.mock_all_auths();
    let contract_id = env.register(TtlExtension, ());
    let client = TtlExtensionClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin, &1000);

    // Record a nonce in temporary storage.
    client.record_nonce(&alice, &1);

    // Advance past the default temporary TTL. The nonce entry
    // should be entirely deleted, not archived.
    add_ledger_entries(&env, 100);

    // The contract is still usable because the instance TLL was
    // extended by the call.
    assert_eq!(client.balance(&admin), 1000);
}
