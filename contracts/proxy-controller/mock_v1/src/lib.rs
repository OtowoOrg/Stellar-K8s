//! v1 of a mock upgradeable contract, used to test `proxy-controller`.
//!
//! It stores a single `u32` counter under `DataKey::Value` and links the
//! upgrade governance state machine from the `proxy-controller` crate. See
//! `mock_v2` for the "upgraded" version and `proxy_controller_tests` for the
//! end-to-end upgrade test.
#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Bytes, BytesN, Env};

use proxy_controller::{self, PendingUpgrade, ProxyError};

/// This crate's own storage keys. None of these names may ever be
/// `ProxyAdmin`, `ProxySecurityCouncil` or `ProxyPendingUpgrade` -- see the
/// collision-prevention note on `proxy_controller::ProxyDataKey`.
#[contracttype]
#[derive(Clone)]
enum DataKey {
    Value,
}

#[contract]
pub struct MockContract;

#[contractimpl]
impl MockContract {
    /// One-time setup. Production deployments should wire this up as the
    /// contract's `__constructor` instead of a callable function; see the
    /// migration guide in `contracts/proxy-controller/README.md`.
    pub fn initialize(
        env: Env,
        admin: Address,
        security_council: Address,
        initial_value: u32,
    ) -> Result<(), ProxyError> {
        proxy_controller::init(&env, &admin, &security_council)?;

        Ok(())
    }

    pub fn version(_env: Env) -> u32 {
        1
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

    // -- Upgrade governance passthroughs --

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
