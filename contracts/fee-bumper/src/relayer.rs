use soroban_sdk::{contracttype, Bytes, BytesN, Env, Address, Error};

pub const RATE_LIMIT_EXCEEDED: Error = Error::from_const(1);
pub const UNDERFUNDED: Error = Error::from_const(2);
pub const REPLAY_DETECTED: Error = Error::from_const(3);
pub const UNAUTHORIZED: Error = Error::from_const(4);
pub const INVALID_TX: Error = Error::from_const(5);

pub const MIN_FEE_RESERVE: u64 = 10_000;

#[contracttype]
pub struct RateLimitState {
    pub count: u64,
    pub window_start: u64,
}

#[contracttype]
pub struct RelayerConfig {
    pub admin: Address,
    pub fee_bump_cap: u64,
    pub reimbursement_token: Address,
    pub rate_limit: u64,
    pub rate_window: u64,
}

#[contracttype]
pub enum DataKey {
    Config,
    RateLimit(Address),
    TxRegistry(BytesN<32>),
    Reimbursement(Address),
}

pub fn store_config(env: &Env, config: &RelayerConfig) {
    env.storage().instance().set(&DataKey::Config, config);
}

pub fn get_config(env: &Env) -> RelayerConfig {
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .expect("contract not initialized")
}

pub fn validate_inner_tx(env: &Env, inner_tx_bytes: &Bytes, fee_offered: u64) -> Result<(), Error> {
    if inner_tx_bytes.len() < 4 {
        return Err(INVALID_TX);
    }

    let config = get_config(env);
    let min_required = MIN_FEE_RESERVE.saturating_add(config.fee_bump_cap);

    if fee_offered < min_required {
        return Err(UNDERFUNDED);
    }

    Ok(())
}

pub fn check_rate_limit(env: &Env, account: &Address) -> Result<(), Error> {
    let config = get_config(env);
    let current_ledger = env.ledger().sequence() as u64;
    let key = DataKey::RateLimit(account.clone());

    let state: RateLimitState = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or(RateLimitState {
            count: 0,
            window_start: current_ledger,
        });

    let window_elapsed = current_ledger.saturating_sub(state.window_start);
    let window_expired = window_elapsed >= config.rate_window;

    let effective_count = if window_expired { 0 } else { state.count };
    let effective_start = if window_expired {
        current_ledger
    } else {
        state.window_start
    };

    if effective_count >= config.rate_limit {
        return Err(RATE_LIMIT_EXCEEDED);
    }

    let new_state = RateLimitState {
        count: effective_count + 1,
        window_start: effective_start,
    };
    env.storage().persistent().set(&key, &new_state);

    Ok(())
}

pub fn check_replay(env: &Env, tx_hash: &BytesN<32>) -> Result<(), Error> {
    let key = DataKey::TxRegistry(tx_hash.clone());
    if env.storage().persistent().has(&key) {
        return Err(REPLAY_DETECTED);
    }
    env.storage().persistent().set(&key, &true);
    Ok(())
}

pub fn record_reimbursement(env: &Env, account: &Address, amount: u64) {
    let key = DataKey::Reimbursement(account.clone());
    let current: u64 = env.storage().persistent().get(&key).unwrap_or(0);
    env.storage()
        .persistent()
        .set(&key, &current.saturating_add(amount));
}

pub fn claim_reimbursement(env: &Env, account: &Address) -> Result<u64, Error> {
    let key = DataKey::Reimbursement(account.clone());
    let amount: u64 = env.storage().persistent().get(&key).unwrap_or(0);
    if amount == 0 {
        return Ok(0);
    }
    env.storage().persistent().set(&key, &0u64);
    Ok(amount)
}

pub fn compute_inner_tx_hash(env: &Env, inner_tx_bytes: &Bytes) -> BytesN<32> {
    env.crypto().sha256(inner_tx_bytes).into()
}