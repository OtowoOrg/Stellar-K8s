use super::{WrappedToken, WrappedTokenClient};
use soroban_sdk::{testutils::Address as _, Address, Env, String};

fn setup() -> (Env, Address, Address, Address, Address) {
    let env = Env::default();
    let admin = Address::generate(&env);
    let relayer = Address::generate(&env);
    let user = Address::generate(&env);
    env.mock_all_auths();
    let contract_id = env.register(WrappedToken, ());
    let client = WrappedTokenClient::new(&env, &contract_id);
    client.initialize(
        &admin,
        &relayer,
        &7,
        &String::from_str(&env, "Wrapped USD"),
        &String::from_str(&env, "wUSD"),
    );
    (env, contract_id, admin, relayer, user)
}

#[test]
fn relayer_can_mint_and_burn() {
    let (env, contract_id, _admin, _relayer, user) = setup();
    let client = WrappedTokenClient::new(&env, &contract_id);
    client.mint(&user, &100);
    assert_eq!(client.balance(&user), 100);
    assert_eq!(client.total_supply(), 100);
    client.burn(&user, &40);
    assert_eq!(client.balance(&user), 60);
    assert_eq!(client.total_supply(), 60);
}

#[test]
#[should_panic]
fn unauthorized_mint_is_rejected() {
    let (env, contract_id, _admin, _relayer, user) = setup();
    let client = WrappedTokenClient::new(&env, &contract_id);
    env.mock_auths(&[]);
    client.mint(&user, &1);
}

#[test]
fn pause_freezes_transfers_and_minting() {
    let (env, contract_id, _admin, _relayer, user) = setup();
    let client = WrappedTokenClient::new(&env, &contract_id);
    client.mint(&user, &100);
    client.set_paused(&true);
    assert!(client.paused());
    client.set_paused(&false);
    assert!(!client.paused());
}

#[test]
#[should_panic]
fn paused_token_rejects_mint() {
    let (env, contract_id, _admin, _relayer, user) = setup();
    let client = WrappedTokenClient::new(&env, &contract_id);
    client.set_paused(&true);
    client.mint(&user, &1);
}

#[test]
#[should_panic]
fn paused_token_rejects_transfer() {
    let (env, contract_id, _admin, _relayer, user) = setup();
    let client = WrappedTokenClient::new(&env, &contract_id);
    client.mint(&user, &1);
    client.set_paused(&true);
    client.transfer(&user, &user, &1);
}
