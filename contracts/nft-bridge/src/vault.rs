//! Token registry and bridge-vault state transitions.
//!
//! Every token records its [`Origin`] from creation. The vault's safety
//! rules are enforced here, in one place:
//!
//! * only `Native` tokens can be locked into the vault pool (synthetic
//!   representations of Ethereum NFTs must be burned, never locked);
//! * a token that is already `Locked` cannot be locked again;
//! * each lock takes a fresh, monotonically increasing nonce that relayers
//!   use to deduplicate bridge events.

use soroban_sdk::{contracttype, panic_with_error, Address, Env, U256};

use crate::{BridgeError, DataKey};

const DAY_IN_LEDGERS: u32 = 17_280;
const ENTRY_BUMP: u32 = 90 * DAY_IN_LEDGERS;
const ENTRY_THRESHOLD: u32 = ENTRY_BUMP - DAY_IN_LEDGERS;

/// Where a token's canonical supply lives.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Origin {
    /// Issued on Soroban; may be locked to move to Ethereum.
    Native,
    /// Soroban representation of an NFT locked on Ethereum.
    Synthetic,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    /// Held by its owner and usable on Soroban.
    Active,
    /// Escrowed in the vault while it circulates on Ethereum.
    Locked,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenRecord {
    pub owner: Address,
    pub origin: Origin,
    pub status: Status,
}

fn bump(env: &Env, key: &DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, ENTRY_THRESHOLD, ENTRY_BUMP);
}

pub fn token(env: &Env, token_id: &U256) -> Option<TokenRecord> {
    let key = DataKey::Token(token_id.clone());
    let record = env.storage().persistent().get(&key);
    if record.is_some() {
        bump(env, &key);
    }
    record
}

pub fn existing_token(env: &Env, token_id: &U256) -> TokenRecord {
    token(env, token_id).unwrap_or_else(|| panic_with_error!(env, BridgeError::UnknownToken))
}

pub fn save_token(env: &Env, token_id: &U256, record: &TokenRecord) {
    let key = DataKey::Token(token_id.clone());
    env.storage().persistent().set(&key, record);
    bump(env, &key);
}

pub fn balance(env: &Env, owner: &Address) -> u64 {
    env.storage()
        .persistent()
        .get(&DataKey::Balance(owner.clone()))
        .unwrap_or(0)
}

fn set_balance(env: &Env, owner: &Address, balance: u64) {
    let key = DataKey::Balance(owner.clone());
    if balance == 0 {
        env.storage().persistent().remove(&key);
    } else {
        env.storage().persistent().set(&key, &balance);
        bump(env, &key);
    }
}

fn move_balance(env: &Env, from: &Address, to: &Address) {
    set_balance(env, from, balance(env, from) - 1);
    let to_balance = balance(env, to)
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, BridgeError::Overflow));
    set_balance(env, to, to_balance);
}

/// Registers a newly created token.
pub fn create(env: &Env, token_id: &U256, owner: &Address, origin: Origin) {
    if token(env, token_id).is_some() {
        panic_with_error!(env, BridgeError::TokenExists);
    }
    let owner_balance = balance(env, owner)
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, BridgeError::Overflow));
    set_balance(env, owner, owner_balance);
    save_token(
        env,
        token_id,
        &TokenRecord {
            owner: owner.clone(),
            origin,
            status: Status::Active,
        },
    );
}

fn next_lock_nonce(env: &Env) -> u64 {
    let nonce: u64 = env
        .storage()
        .instance()
        .get(&DataKey::LockNonce)
        .unwrap_or(0);
    let next = nonce
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, BridgeError::Overflow));
    env.storage().instance().set(&DataKey::LockNonce, &next);
    next
}

/// Escrows a native token owned by `owner` in the vault and returns the
/// lock nonce. The caller must already have authenticated `owner`.
pub fn lock(env: &Env, owner: &Address, token_id: &U256) -> u64 {
    let mut record = existing_token(env, token_id);
    if record.origin != Origin::Native {
        panic_with_error!(env, BridgeError::SyntheticNotLockable);
    }
    if record.status == Status::Locked {
        panic_with_error!(env, BridgeError::AlreadyLocked);
    }
    if record.owner != *owner {
        panic_with_error!(env, BridgeError::NotOwner);
    }

    let vault = env.current_contract_address();
    move_balance(env, owner, &vault);
    record.owner = vault;
    record.status = Status::Locked;
    save_token(env, token_id, &record);
    next_lock_nonce(env)
}
