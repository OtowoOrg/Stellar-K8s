#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Env};

mod curve;

#[contract]
pub struct BondingCurveContract;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Token,
    ReserveToken,
    ReserveBalance,
    TotalSupply,
    ReserveRatio, // Expressed as a fraction of 10000 (e.g. 5000 for 1/2)
    FeePercentage, // Fee in basis points
}

#[contractimpl]
impl BondingCurveContract {
    pub fn initialize(
        env: Env,
        token: Address,
        reserve_token: Address,
        reserve_ratio: u32,
        fee_percentage: u32,
    ) {
        assert!(!env.storage().instance().has(&DataKey::Token), "already initialized");
        env.storage().instance().set(&DataKey::Token, &token);
        env.storage().instance().set(&DataKey::ReserveToken, &reserve_token);
        env.storage().instance().set(&DataKey::ReserveBalance, &0i128);
        env.storage().instance().set(&DataKey::TotalSupply, &0i128);
        env.storage().instance().set(&DataKey::ReserveRatio, &reserve_ratio);
        env.storage().instance().set(&DataKey::FeePercentage, &fee_percentage);
    }

    pub fn buy(env: Env, buyer: Address, amount: i128, max_cost: i128) {
        buyer.require_auth();

        let token: Address = env.storage().instance().get(&DataKey::Token).unwrap();
        let reserve_token: Address = env.storage().instance().get(&DataKey::ReserveToken).unwrap();
        
        let reserve_balance: i128 = env.storage().instance().get(&DataKey::ReserveBalance).unwrap();
        let total_supply: i128 = env.storage().instance().get(&DataKey::TotalSupply).unwrap();
        let reserve_ratio: u32 = env.storage().instance().get(&DataKey::ReserveRatio).unwrap();

        let cost = curve::calculate_purchase_return(
            total_supply,
            reserve_balance,
            reserve_ratio,
            amount
        );

        let fee_percentage: u32 = env.storage().instance().get(&DataKey::FeePercentage).unwrap();
        let fee = (cost * fee_percentage as i128) / 10000;
        let total_cost = cost + fee;

        assert!(total_cost <= max_cost, "slippage limit exceeded");

        let reserve_client = token::Client::new(&env, &reserve_token);
        reserve_client.transfer(&buyer, &env.current_contract_address(), &total_cost);

        let token_client = token::Client::new(&env, &token);
        token_client.mint(&buyer, &amount);

        env.storage().instance().set(&DataKey::ReserveBalance, &(reserve_balance + cost));
        env.storage().instance().set(&DataKey::TotalSupply, &(total_supply + amount));
    }

    pub fn sell(env: Env, seller: Address, amount: i128, min_return: i128) {
        seller.require_auth();

        let token: Address = env.storage().instance().get(&DataKey::Token).unwrap();
        let reserve_token: Address = env.storage().instance().get(&DataKey::ReserveToken).unwrap();
        
        let reserve_balance: i128 = env.storage().instance().get(&DataKey::ReserveBalance).unwrap();
        let total_supply: i128 = env.storage().instance().get(&DataKey::TotalSupply).unwrap();
        let reserve_ratio: u32 = env.storage().instance().get(&DataKey::ReserveRatio).unwrap();

        let return_amount = curve::calculate_sale_return(
            total_supply,
            reserve_balance,
            reserve_ratio,
            amount
        );

        let fee_percentage: u32 = env.storage().instance().get(&DataKey::FeePercentage).unwrap();
        let fee = (return_amount * fee_percentage as i128) / 10000;
        let net_return = return_amount - fee;

        assert!(net_return >= min_return, "slippage limit exceeded");

        let token_client = token::Client::new(&env, &token);
        token_client.burn(&seller, &amount);

        let reserve_client = token::Client::new(&env, &reserve_token);
        reserve_client.transfer(&env.current_contract_address(), &seller, &net_return);

        env.storage().instance().set(&DataKey::ReserveBalance, &(reserve_balance - return_amount));
        env.storage().instance().set(&DataKey::TotalSupply, &(total_supply - amount));
    }
}
