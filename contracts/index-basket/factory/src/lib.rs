//! # Index Basket Factory
//!
//! Deploys customisable `index-basket` contracts from an
//! uploaded WASM hash and keeps an on-chain registry of every basket it has
//! created.
//!
//! * The creator becomes the new basket's admin.
//! * Deployment salts are namespaced by creator, so nobody can front-run a
//!   creator's deterministic basket address.
//! * The basket constructor validates the configuration; an invalid
//!   configuration aborts the whole deployment, leaving no registry entry.
//!
//! The configuration types below mirror `index_basket::BasketConfig` field
//! for field, so their XDR encoding is identical. Mirroring (rather than
//! depending on the basket crate) keeps the basket's contract exports out of
//! the factory WASM.

#![no_std]

#[cfg(test)]
mod test;

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error,
    xdr::ToXdr, Address, BytesN, Env, String, Vec,
};

const DAY_IN_LEDGERS: u32 = 17_280;
const INSTANCE_BUMP: u32 = 30 * DAY_IN_LEDGERS;
const INSTANCE_THRESHOLD: u32 = INSTANCE_BUMP - DAY_IN_LEDGERS;
const ENTRY_BUMP: u32 = 60 * DAY_IN_LEDGERS;
const ENTRY_THRESHOLD: u32 = ENTRY_BUMP - DAY_IN_LEDGERS;
/// Maximum page size for [`BasketFactory::baskets`].
const MAX_PAGE: u32 = 50;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum FactoryError {
    InvalidPage = 1,
}

/// Mirror of `index_basket::ComponentSpec`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentSpec {
    pub token: Address,
    pub units: i128,
    pub weight_bps: u32,
}

/// Mirror of `index_basket::BasketConfig`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BasketConfig {
    pub name: String,
    pub symbol: String,
    pub decimals: u32,
    pub components: Vec<ComponentSpec>,
    pub oracle: Address,
    pub max_price_age: u64,
    pub rebalance_incentive_bps: u32,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    WasmHash,
    Count,
    Basket(u32),
    Registered(Address),
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BasketCreated {
    #[topic]
    pub basket: Address,
    #[topic]
    pub creator: Address,
    pub index: u32,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WasmHashUpdated {
    pub wasm_hash: BytesN<32>,
}

#[contract]
pub struct BasketFactory;

fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_THRESHOLD, INSTANCE_BUMP);
}

fn set_persistent<V: soroban_sdk::IntoVal<Env, soroban_sdk::Val>>(
    env: &Env,
    key: &DataKey,
    value: &V,
) {
    env.storage().persistent().set(key, value);
    env.storage()
        .persistent()
        .extend_ttl(key, ENTRY_THRESHOLD, ENTRY_BUMP);
}

fn require_admin(env: &Env) {
    let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
    admin.require_auth();
}

/// Salt namespaced by creator: `sha256(creator_xdr || salt)`.
fn creator_salt(env: &Env, creator: &Address, salt: &BytesN<32>) -> BytesN<32> {
    let mut preimage = creator.clone().to_xdr(env);
    preimage.append(&salt.clone().into());
    env.crypto().sha256(&preimage).into()
}

#[contractimpl]
impl BasketFactory {
    /// `basket_wasm_hash` is the hash of the uploaded `index_basket.wasm`.
    pub fn __constructor(env: Env, admin: Address, basket_wasm_hash: BytesN<32>) {
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::WasmHash, &basket_wasm_hash);
        env.storage().instance().set(&DataKey::Count, &0u32);
        bump_instance(&env);
    }

    /// Deploys a new basket administered by `creator` and registers it.
    pub fn create_basket(
        env: Env,
        creator: Address,
        salt: BytesN<32>,
        config: BasketConfig,
    ) -> Address {
        creator.require_auth();
        bump_instance(&env);

        let wasm_hash: BytesN<32> = env.storage().instance().get(&DataKey::WasmHash).unwrap();
        let basket = env
            .deployer()
            .with_current_contract(creator_salt(&env, &creator, &salt))
            .deploy_v2(wasm_hash, (creator.clone(), config));

        let index: u32 = env.storage().instance().get(&DataKey::Count).unwrap();
        set_persistent(&env, &DataKey::Basket(index), &basket);
        set_persistent(&env, &DataKey::Registered(basket.clone()), &true);
        env.storage().instance().set(&DataKey::Count, &(index + 1));

        BasketCreated {
            basket: basket.clone(),
            creator,
            index,
        }
        .publish(&env);
        basket
    }

    /// Deterministic address `create_basket(creator, salt, ..)` deploys to.
    pub fn basket_address(env: Env, creator: Address, salt: BytesN<32>) -> Address {
        env.deployer()
            .with_current_contract(creator_salt(&env, &creator, &salt))
            .deployed_address()
    }

    /// Points future deployments at a new basket WASM. Existing baskets are
    /// unaffected.
    pub fn set_basket_wasm_hash(env: Env, wasm_hash: BytesN<32>) {
        require_admin(&env);
        bump_instance(&env);
        env.storage().instance().set(&DataKey::WasmHash, &wasm_hash);
        WasmHashUpdated { wasm_hash }.publish(&env);
    }

    /// Transfers factory administration. Both admins must sign.
    pub fn set_admin(env: Env, new_admin: Address) {
        require_admin(&env);
        new_admin.require_auth();
        bump_instance(&env);
        env.storage().instance().set(&DataKey::Admin, &new_admin);
    }

    pub fn admin(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Admin).unwrap()
    }

    pub fn basket_wasm_hash(env: Env) -> BytesN<32> {
        env.storage().instance().get(&DataKey::WasmHash).unwrap()
    }

    pub fn basket_count(env: Env) -> u32 {
        env.storage().instance().get(&DataKey::Count).unwrap()
    }

    /// Registered baskets in creation order, `[start, start + limit)`.
    pub fn baskets(env: Env, start: u32, limit: u32) -> Vec<Address> {
        if limit == 0 || limit > MAX_PAGE {
            panic_with_error!(&env, FactoryError::InvalidPage);
        }
        let end = Self::basket_count(env.clone()).min(start.saturating_add(limit));
        let mut out = Vec::new(&env);
        for i in start..end {
            out.push_back(env.storage().persistent().get(&DataKey::Basket(i)).unwrap());
        }
        out
    }

    /// Whether `basket` was deployed by this factory.
    pub fn is_basket(env: Env, basket: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::Registered(basket))
            .unwrap_or(false)
    }
}
