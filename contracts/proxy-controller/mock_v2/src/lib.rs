//! v2 of the mock upgradeable contract from `mock_v1`.
//!
//! Deliberately reuses the exact same `DataKey` enum (so the existing
//! `Value` entry keeps resolving to the same storage slot after the Wasm
//! swap) and adds one new method, `double_value`, to demonstrate that new
//! functionality becomes reachable once the upgrade executes. Soroban does
//! not re-run a contract's constructor on a Wasm swap, so this version has
//! no `initialize` -- it only ever comes into existence via
//! `proxy_controller::execute_upgrade` replacing `mock_v1`'s code in place.
#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Bytes, BytesN, Env};

use proxy_controller::{self, PendingUpgrade, ProxyError};

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Value,
}

#[contract]
pub struct MockContract;

#[contractimpl]
impl MockContract {
    pub fn version(_env: Env) -> u32 {
        2
    }

    pub fn get_value(env: Env) -> u32 {
        env.storage().instance().get(&DataKey::Value).unwrap_or(0)
    }

    pub fn set_value(env: Env, admin: Address, new_value: u32) -> Result<(), ProxyError> {
        let stored_admin = proxy_controller::admin(&env).ok_or(ProxyError::NotInitialized)?;
        if admin != stored_admin {
            return Err(ProxyError::Unauthorized);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Value, &new_value);
        Ok(())
    }

    /// New in v2: doubles the stored value in place and returns the result.
    pub fn double_value(env: Env) -> u32 {
        let current: u32 = env.storage().instance().get(&DataKey::Value).unwrap_or(0);
        let doubled = current.saturating_mul(2);
        env.storage().instance().set(&DataKey::Value, &doubled);
        doubled
    }

    // -- Upgrade governance passthroughs (kept so v2 can itself be upgraded) --

    pub fn propose_upgrade(env: Env, new_wasm: Bytes) -> Result<BytesN<32>, ProxyError> {
        proxy_controller::propose_upgrade(&env, new_wasm)
    }

    pub fn cancel_upgrade(env: Env, caller: Address) -> Result<(), ProxyError> {
        proxy_controller::cancel_upgrade(&env, caller)
    }

    pub fn execute_upgrade(env: Env) -> Result<BytesN<32>, ProxyError> {
        proxy_controller::execute_upgrade(&env)
    }

    pub fn pending_upgrade(env: Env) -> Option<PendingUpgrade> {
        proxy_controller::pending_upgrade(&env)
    }

    pub fn admin(env: Env) -> Option<Address> {
        proxy_controller::admin(&env)
    }

    pub fn security_council(env: Env) -> Option<Address> {
        proxy_controller::security_council(&env)
    }
}
