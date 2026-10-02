#![cfg(test)]

use super::*;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{token, Address, Env};
use proptest::prelude::*;

#[test]
fn test_stream_lifecycle() {
    let env = Env::default();
    env.mock_all_auths();
    
    let payer = Address::generate(&env);
    let payee = Address::generate(&env);
    
    let token_admin = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = token::Client::new(&env, &token_id.address());
    let token_admin_client = token::StellarAssetClient::new(&env, &token_id.address());

    // Mint tokens to payer
    token_admin_client.mint(&payer, &100_000);

    let contract_id = env.register_contract(None, StreamingPayContract);
    let client = StreamingPayContractClient::new(&env, &contract_id);

    env.ledger().set_timestamp(1000);

    // Create stream: 100 USDC over 100 seconds
    let stream_id = client.create_stream(&payer, &payee, &token_id.address(), &100_000, &100);
    assert_eq!(stream_id, 1);
    assert_eq!(token.balance(&payer), 0);
    assert_eq!(token.balance(&contract_id), 100_000);

    // Advance time by 50 seconds (midway)
    env.ledger().set_timestamp(1050);
    
    // Payee withdraws
    client.withdraw(&stream_id);
    
    // Half of the stream (50,000) should be withdrawn
    assert_eq!(token.balance(&payee), 50_000);
    assert_eq!(token.balance(&contract_id), 50_000);

    // Advance to end
    env.ledger().set_timestamp(1100);
    client.withdraw(&stream_id);
    assert_eq!(token.balance(&payee), 100_000);
    assert_eq!(token.balance(&contract_id), 0);
}

#[test]
fn test_stream_cancellation() {
    let env = Env::default();
    env.mock_all_auths();
    
    let payer = Address::generate(&env);
    let payee = Address::generate(&env);
    
    let token_admin = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = token::Client::new(&env, &token_id.address());
    let token_admin_client = token::StellarAssetClient::new(&env, &token_id.address());

    token_admin_client.mint(&payer, &100_000);

    let contract_id = env.register_contract(None, StreamingPayContract);
    let client = StreamingPayContractClient::new(&env, &contract_id);

    env.ledger().set_timestamp(1000);

    let stream_id = client.create_stream(&payer, &payee, &token_id.address(), &100_000, &100);

    // Advance time by 30 seconds
    env.ledger().set_timestamp(1030);
    
    // Cancel stream
    client.cancel_stream(&stream_id);
    
    // Payee gets 30_000, payer gets 70_000
    assert_eq!(token.balance(&payee), 30_000);
    assert_eq!(token.balance(&payer), 70_000);
    assert_eq!(token.balance(&contract_id), 0);
}

// Proptest for precision-loss
proptest! {
    #[test]
    fn test_precision_loss(
        amount in 1..1_000_000_000_000_000_i128,
        duration in 1..31_536_000_u64, // up to 1 year
        elapsed in 1..31_536_000_u64,
    ) {
        let env = Env::default();
        env.mock_all_auths();
        
        let payer = Address::generate(&env);
        let payee = Address::generate(&env);
        
        let token_admin = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract_v2(token_admin.clone());
        let token = token::Client::new(&env, &token_id.address());
        let token_admin_client = token::StellarAssetClient::new(&env, &token_id.address());

        token_admin_client.mint(&payer, &amount);

        let contract_id = env.register_contract(None, StreamingPayContract);
        let client = StreamingPayContractClient::new(&env, &contract_id);

        env.ledger().set_timestamp(1000);

        let stream_id = client.create_stream(&payer, &payee, &token_id.address(), &amount, &duration);

        env.ledger().set_timestamp(1000 + elapsed);
        
        if elapsed < duration {
            client.withdraw(&stream_id);
            let withdrawn = token.balance(&payee);
            // Verify rounding in favor of payer
            // Payee gets amount * elapsed / duration
            let expected = (amount as u128 * elapsed as u128 / duration as u128) as i128;
            assert_eq!(withdrawn, expected);
        } else {
            client.withdraw(&stream_id);
            let withdrawn = token.balance(&payee);
            assert_eq!(withdrawn, amount);
        }
    }
}
