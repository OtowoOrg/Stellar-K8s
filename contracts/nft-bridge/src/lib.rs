//! # NFT Bridge Vault (ERC-721 ⇄ Soroban)
//!
//! Moves NFTs between Soroban and Ethereum. Native Soroban NFTs are escrowed
//! in this vault while they circulate on Ethereum. Ethereum NFTs are
//! represented on Soroban by synthetic tokens.
//!
//! This first increment provides:
//!
//! * the token registry with an explicit [`vault::Origin`] per token, so
//!   native and synthetic supply can never be confused;
//! * an ERC-721-style read interface (`name`, `symbol`, `owner_of`,
//!   `balance_of`) adapted to Soroban `Address`es;
//! * admin issuance of native NFTs;
//! * [`NftBridge::lock`], which escrows a native NFT and emits a
//!   [`NativeLocked`] bridge event carrying the destination chain and
//!   Ethereum address. Synthetic tokens cannot be locked, and a token
//!   cannot be locked twice.
//!
//! Synthetic minting by the relayer multi-sig, burn-to-unlock and the
//! return path are separate follow-ups built on the same registry.

#![no_std]

pub mod vault;

#[cfg(test)]
mod test;

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, Address,
    BytesN, Env, String, U256,
};

const DAY_IN_LEDGERS: u32 = 17_280;
const INSTANCE_BUMP: u32 = 30 * DAY_IN_LEDGERS;
const INSTANCE_THRESHOLD: u32 = INSTANCE_BUMP - DAY_IN_LEDGERS;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum BridgeError {
    UnknownToken = 1,
    TokenExists = 2,
    NotOwner = 3,
    /// Synthetic tokens must be burned to return to Ethereum, never locked.
    SyntheticNotLockable = 4,
    AlreadyLocked = 5,
    InvalidDestination = 6,
    Overflow = 7,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Name,
    Symbol,
    LockNonce,
    Token(U256),
    Balance(Address),
}

/// Bridge event observed by the relayers: a native NFT is escrowed and
/// should be minted to `dest_address` on EVM chain `dest_chain_id`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeLocked {
    #[topic]
    pub token_id: U256,
    #[topic]
    pub owner: Address,
    pub dest_chain_id: u64,
    pub dest_address: BytesN<20>,
    /// Unique, increasing per vault; relayers deduplicate on it.
    pub nonce: u64,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeMinted {
    #[topic]
    pub token_id: U256,
    #[topic]
    pub to: Address,
}

#[contract]
pub struct NftBridge;

fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_THRESHOLD, INSTANCE_BUMP);
}

#[contractimpl]
impl NftBridge {
    pub fn __constructor(env: Env, admin: Address, name: String, symbol: String) {
        let storage = env.storage().instance();
        storage.set(&DataKey::Admin, &admin);
        storage.set(&DataKey::Name, &name);
        storage.set(&DataKey::Symbol, &symbol);
        bump_instance(&env);
    }

    /// Issues a native NFT. Admin only.
    pub fn mint(env: Env, to: Address, token_id: U256) {
        Self::admin(env.clone()).require_auth();
        bump_instance(&env);
        vault::create(&env, &token_id, &to, vault::Origin::Native);
        NativeMinted { token_id, to }.publish(&env);
    }

    /// Escrows the native NFT `token_id` in the vault for transfer to
    /// `dest_address` on EVM chain `dest_chain_id`. Returns the lock nonce.
    pub fn lock(
        env: Env,
        owner: Address,
        token_id: U256,
        dest_chain_id: u64,
        dest_address: BytesN<20>,
    ) -> u64 {
        owner.require_auth();
        bump_instance(&env);
        if dest_chain_id == 0 || dest_address.to_array() == [0u8; 20] {
            panic_with_error!(&env, BridgeError::InvalidDestination);
        }
        let nonce = vault::lock(&env, &owner, &token_id);
        NativeLocked {
            token_id,
            owner,
            dest_chain_id,
            dest_address,
            nonce,
        }
        .publish(&env);
        nonce
    }

    // -----------------------------------------------------------------------
    // ERC-721-style views
    // -----------------------------------------------------------------------

    pub fn name(env: Env) -> String {
        env.storage().instance().get(&DataKey::Name).unwrap()
    }

    pub fn symbol(env: Env) -> String {
        env.storage().instance().get(&DataKey::Symbol).unwrap()
    }

    /// Current owner; the vault's own address while the token is locked.
    pub fn owner_of(env: Env, token_id: U256) -> Address {
        vault::existing_token(&env, &token_id).owner
    }

    pub fn balance_of(env: Env, owner: Address) -> u64 {
        vault::balance(&env, &owner)
    }

    pub fn token(env: Env, token_id: U256) -> Option<vault::TokenRecord> {
        vault::token(&env, &token_id)
    }

    pub fn admin(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Admin).unwrap()
    }
}
