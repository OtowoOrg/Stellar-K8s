#![no_std]

#[cfg(any(test, feature = "testutils"))]
extern crate std;

mod relayer;

use relayer::*;
use soroban_sdk::{contract, contractimpl, contracttype, Bytes, BytesN, Env, Address, Error};

#[contracttype]
pub struct FeeBumpResult {
    pub tx_hash: BytesN<32>,
    pub fee_charged: u64,
    pub success: bool,
}

#[contract]
pub struct FeeBumper;

#[contractimpl]
impl FeeBumper {
    pub fn initialize(
        env: Env,
        admin: Address,
        fee_bump_cap: u64,
        reimbursement_token: Address,
        rate_limit: u64,
        rate_window: u64,
    ) {
        let config = RelayerConfig {
            admin,
            fee_bump_cap,
            reimbursement_token,
            rate_limit,
            rate_window,
        };
        store_config(&env, &config);
    }

    pub fn validate_inner_tx(
        env: Env,
        inner_tx_bytes: Bytes,
        fee_offered: u64,
    ) -> Result<(), Error> {
        relayer::validate_inner_tx(&env, &inner_tx_bytes, fee_offered)
    }

    pub fn wrap_fee_bump(
        env: Env,
        account: Address,
        inner_tx_bytes: Bytes,
        base_fee: u64,
        max_fee: u64,
    ) -> Result<FeeBumpResult, Error> {
        let config = get_config(&env);

        relayer::validate_inner_tx(&env, &inner_tx_bytes, max_fee)?;
        relayer::check_rate_limit(&env, &account)?;

        let tx_hash = relayer::compute_inner_tx_hash(&env, &inner_tx_bytes);
        relayer::check_replay(&env, &tx_hash)?;

        let fee_charged = base_fee.saturating_add(config.fee_bump_cap);
        relayer::record_reimbursement(&env, &account, fee_charged);

        Ok(FeeBumpResult {
            tx_hash,
            fee_charged,
            success: true,
        })
    }

    pub fn claim_reimbursement(env: Env, account: Address) -> Result<u64, Error> {
        relayer::claim_reimbursement(&env, &account)
    }

    pub fn check_rate_limit(env: Env, account: Address) -> Result<(), Error> {
        relayer::check_rate_limit(&env, &account)
    }

    pub fn set_rate_limit(env: Env, new_limit: u64) {
        let config = get_config(&env);
        config.admin.require_auth();
        let mut updated = config;
        updated.rate_limit = new_limit;
        store_config(&env, &updated);
    }

    pub fn set_fee_cap(env: Env, new_cap: u64) {
        let config = get_config(&env);
        config.admin.require_auth();
        let mut updated = config;
        updated.fee_bump_cap = new_cap;
        store_config(&env, &updated);
    }

    pub fn get_config(env: Env) -> RelayerConfig {
        relayer::get_config(&env)
    }
}

#[cfg(test)]
mod test;
