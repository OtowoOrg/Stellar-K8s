extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Events as _},
    Address, BytesN, Env, Map, String, Symbol, TryFromVal, Val, U256,
};

use crate::vault::{self, Origin, Status, TokenRecord};
use crate::{BridgeError, NftBridge, NftBridgeClient};

const ETHEREUM: u64 = 1;

struct Setup<'a> {
    env: Env,
    admin: Address,
    alice: Address,
    client: NftBridgeClient<'a>,
}

impl<'a> Setup<'a> {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let alice = Address::generate(&env);
        let id = env.register(
            NftBridge,
            (
                admin.clone(),
                String::from_str(&env, "Bridged Punks"),
                String::from_str(&env, "BPUNK"),
            ),
        );
        let client = NftBridgeClient::new(&env, &id);
        Setup {
            env,
            admin,
            alice,
            client,
        }
    }

    fn id(&self, n: u32) -> U256 {
        U256::from_u32(&self.env, n)
    }

    fn eth_address(&self) -> BytesN<20> {
        BytesN::from_array(&self.env, &[0xAB; 20])
    }

    fn lock(&self, owner: &Address, n: u32) -> u64 {
        self.client
            .lock(owner, &self.id(n), &ETHEREUM, &self.eth_address())
    }
}

/// Decoded last `native_locked` event: (token_id, owner, chain, address, nonce).
fn last_lock_event(env: &Env) -> (U256, Address, u64, BytesN<20>, u64) {
    let (_, topics, data) = env.events().all().last().unwrap();
    assert_eq!(
        Symbol::try_from_val(env, &topics.get_unchecked(0)).unwrap(),
        Symbol::new(env, "native_locked")
    );
    let data = Map::<Symbol, Val>::try_from_val(env, &data).unwrap();
    let field = |name: &str| data.get(Symbol::new(env, name)).unwrap();
    (
        U256::try_from_val(env, &topics.get_unchecked(1)).unwrap(),
        Address::try_from_val(env, &topics.get_unchecked(2)).unwrap(),
        u64::try_from_val(env, &field("dest_chain_id")).unwrap(),
        BytesN::<20>::try_from_val(env, &field("dest_address")).unwrap(),
        u64::try_from_val(env, &field("nonce")).unwrap(),
    )
}

// ---------------------------------------------------------------------------
// Registry and ERC-721-style views
// ---------------------------------------------------------------------------

#[test]
fn mint_registers_native_token_and_updates_balance() {
    let s = Setup::new();
    s.client.mint(&s.alice, &s.id(1));
    s.client.mint(&s.alice, &s.id(2));

    assert_eq!(s.client.name(), String::from_str(&s.env, "Bridged Punks"));
    assert_eq!(s.client.symbol(), String::from_str(&s.env, "BPUNK"));
    assert_eq!(s.client.owner_of(&s.id(1)), s.alice);
    assert_eq!(s.client.balance_of(&s.alice), 2);
    assert_eq!(
        s.client.token(&s.id(1)),
        Some(TokenRecord {
            owner: s.alice.clone(),
            origin: Origin::Native,
            status: Status::Active,
        })
    );
}

#[test]
fn token_ids_span_the_full_uint256_range() {
    let s = Setup::new();
    let max = U256::from_parts(&s.env, u64::MAX, u64::MAX, u64::MAX, u64::MAX);
    s.client.mint(&s.alice, &max);
    assert_eq!(s.client.owner_of(&max), s.alice);
}

#[test]
fn mint_is_admin_only() {
    let s = Setup::new();
    s.client.mint(&s.alice, &s.id(1));
    assert_eq!(s.env.auths()[0].0, s.admin);
}

#[test]
fn mint_rejects_unauthorised_caller() {
    let env = Env::default();
    let client = NftBridgeClient::new(
        &env,
        &env.register(
            NftBridge,
            (
                Address::generate(&env),
                String::from_str(&env, "N"),
                String::from_str(&env, "N"),
            ),
        ),
    );
    assert!(client
        .try_mint(&Address::generate(&env), &U256::from_u32(&env, 1))
        .is_err());
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn mint_rejects_duplicate_token_id() {
    let s = Setup::new();
    s.client.mint(&s.alice, &s.id(1));
    s.client.mint(&s.alice, &s.id(1));
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn owner_of_unknown_token_fails() {
    let s = Setup::new();
    s.client.owner_of(&s.id(9));
}

// ---------------------------------------------------------------------------
// Locking native NFTs
// ---------------------------------------------------------------------------

#[test]
fn lock_escrows_token_and_emits_bridge_event() {
    let s = Setup::new();
    s.client.mint(&s.alice, &s.id(7));

    let nonce = s.lock(&s.alice, 7);
    // Events are per invocation, so read it before any other call.
    let event = last_lock_event(&s.env);

    let vault = s.client.address.clone();
    assert_eq!(nonce, 1);
    assert_eq!(s.client.owner_of(&s.id(7)), vault);
    assert_eq!(s.client.balance_of(&s.alice), 0);
    assert_eq!(s.client.balance_of(&vault), 1);
    assert_eq!(s.client.token(&s.id(7)).unwrap().status, Status::Locked);
    assert_eq!(
        event,
        (s.id(7), s.alice.clone(), ETHEREUM, s.eth_address(), 1)
    );
}

#[test]
fn lock_nonces_are_unique_and_increasing() {
    let s = Setup::new();
    for n in 1..=3 {
        s.client.mint(&s.alice, &s.id(n));
    }
    let nonces: std::vec::Vec<u64> = (1..=3).map(|n| s.lock(&s.alice, n)).collect();
    assert_eq!(nonces, [1, 2, 3]);
    assert_eq!(last_lock_event(&s.env).4, 3);
}

#[test]
fn lock_requires_owner_auth() {
    let s = Setup::new();
    s.client.mint(&s.alice, &s.id(1));
    s.lock(&s.alice, 1);
    assert_eq!(s.env.auths()[0].0, s.alice);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn lock_rejects_non_owner() {
    let s = Setup::new();
    s.client.mint(&s.alice, &s.id(1));
    let mallory = Address::generate(&s.env);
    s.lock(&mallory, 1);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn token_cannot_be_locked_twice() {
    let s = Setup::new();
    s.client.mint(&s.alice, &s.id(1));
    s.lock(&s.alice, 1);
    // The vault now owns it; even the vault itself cannot re-lock it.
    let vault = s.client.address.clone();
    s.lock(&vault, 1);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn synthetic_token_cannot_enter_native_vault_pool() {
    let s = Setup::new();
    // Synthetic minting is not part of this increment; register one
    // directly through the vault module.
    s.env.as_contract(&s.client.address, || {
        vault::create(&s.env, &s.id(42), &s.alice, Origin::Synthetic)
    });
    s.lock(&s.alice, 42);
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn lock_rejects_unknown_token() {
    let s = Setup::new();
    s.lock(&s.alice, 1);
}

#[test]
fn lock_rejects_invalid_destination() {
    let s = Setup::new();
    s.client.mint(&s.alice, &s.id(1));
    let zero = BytesN::from_array(&s.env, &[0u8; 20]);
    let invalid = Ok(BridgeError::InvalidDestination.into());
    assert_eq!(
        s.client
            .try_lock(&s.alice, &s.id(1), &ETHEREUM, &zero)
            .unwrap_err(),
        invalid
    );
    assert_eq!(
        s.client
            .try_lock(&s.alice, &s.id(1), &0, &s.eth_address())
            .unwrap_err(),
        invalid
    );
    // A rejected lock leaves the token untouched.
    assert_eq!(s.client.owner_of(&s.id(1)), s.alice);
}
