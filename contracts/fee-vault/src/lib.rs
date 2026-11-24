#![no_std]

mod allowance;

use allowance::Allowance;
use soroban_sdk::{contract, contractimpl, Address, Env};

#[contract]
pub struct FeeVault;

#[contractimpl]
impl FeeVault {
    pub fn initialize(
        _env: Env,
        _admin: Address,
        _window_seconds: u64,
        _emergency_threshold: i128,
    ) {
    }

    pub fn set_allowance(
        env: Env,
        operator: Address,
        limit: i128,
        window_seconds: u64,
    ) -> Allowance {
        operator.require_auth();

        Allowance::new(
            operator,
            limit,
            env.ledger().timestamp(),
            window_seconds,
        )
    }

    pub fn check_allowance(
        env: Env,
        allowance: Allowance,
        amount: i128,
    ) -> bool {
        allowance.available(env.ledger().timestamp()) >= amount
    }

    pub fn consume(
        env: Env,
        mut allowance: Allowance,
        amount: i128,
    ) -> bool {
        allowance.operator.require_auth();

        allowance.consume(
            amount,
            env.ledger().timestamp(),
        )
    }
}