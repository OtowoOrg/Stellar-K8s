//! # Concentrated Liquidity AMM
//!
//! A Uniswap-v3-style pool in which liquidity providers allocate capital to
//! bounded price ranges made of discrete ticks.
//!
//! This first increment provides the pricing foundation:
//!
//! * [`ticks`] — exact Q64.96 fixed-point conversion between ticks and
//!   √price, tick-spacing rules and the per-tick liquidity cap, with their
//!   correctness arguments;
//! * pool configuration and one-time price initialisation, which places the
//!   pool on its starting tick.
//!
//! Positions, cross-tick swaps and fee accrual build on this in later
//! increments.

#![no_std]

pub mod ticks;

#[cfg(test)]
mod test;

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, Address,
    Bytes, Env, U256,
};

const DAY_IN_LEDGERS: u32 = 17_280;
const INSTANCE_BUMP: u32 = 30 * DAY_IN_LEDGERS;
const INSTANCE_THRESHOLD: u32 = INSTANCE_BUMP - DAY_IN_LEDGERS;

/// Fees are expressed in hundredths of a basis point (1e-6).
pub const FEE_DENOMINATOR: u32 = 1_000_000;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum PoolError {
    TickOutOfRange = 1,
    SqrtPriceOutOfRange = 2,
    InvalidTickSpacing = 3,
    TickNotAligned = 4,
    InvalidRange = 5,
    /// `token0` must sort strictly before `token1`.
    InvalidTokenOrder = 6,
    InvalidFee = 7,
    AlreadyInitialized = 8,
}

impl From<ticks::TickError> for PoolError {
    fn from(e: ticks::TickError) -> Self {
        match e {
            ticks::TickError::TickOutOfRange => PoolError::TickOutOfRange,
            ticks::TickError::SqrtPriceOutOfRange => PoolError::SqrtPriceOutOfRange,
            ticks::TickError::InvalidTickSpacing => PoolError::InvalidTickSpacing,
            ticks::TickError::TickNotAligned => PoolError::TickNotAligned,
            ticks::TickError::InvalidRange => PoolError::InvalidRange,
        }
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolConfig {
    pub token0: Address,
    pub token1: Address,
    /// Swap fee in units of 1e-6.
    pub fee: u32,
    pub tick_spacing: i32,
    /// `L_max(tick_spacing)`; see [`ticks::max_liquidity_per_tick`].
    pub max_liquidity_per_tick: u128,
}

/// Current price state.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Slot0 {
    /// √price in Q64.96.
    pub sqrt_price_x96: U256,
    /// Greatest tick whose √price is `<= sqrt_price_x96`.
    pub tick: i32,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    Config,
    Slot0,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Initialized {
    pub sqrt_price_x96: U256,
    pub tick: i32,
}

#[contract]
pub struct ClAmm;

fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_THRESHOLD, INSTANCE_BUMP);
}

fn fail(env: &Env, e: ticks::TickError) -> ! {
    panic_with_error!(env, PoolError::from(e))
}

fn to_host(env: &Env, v: ethnum::U256) -> U256 {
    U256::from_be_bytes(env, &Bytes::from_array(env, &v.to_be_bytes()))
}

fn from_host(v: &U256) -> ethnum::U256 {
    let mut buf = [0u8; 32];
    v.to_be_bytes().copy_into_slice(&mut buf);
    ethnum::U256::from_be_bytes(buf)
}

#[contractimpl]
impl ClAmm {
    pub fn __constructor(
        env: Env,
        admin: Address,
        token0: Address,
        token1: Address,
        fee: u32,
        tick_spacing: i32,
    ) {
        if token0 >= token1 {
            panic_with_error!(&env, PoolError::InvalidTokenOrder);
        }
        if fee >= FEE_DENOMINATOR {
            panic_with_error!(&env, PoolError::InvalidFee);
        }
        ticks::validate_tick_spacing(tick_spacing).unwrap_or_else(|e| fail(&env, e));
        let config = PoolConfig {
            token0,
            token1,
            fee,
            tick_spacing,
            max_liquidity_per_tick: ticks::max_liquidity_per_tick(tick_spacing),
        };
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Config, &config);
        bump_instance(&env);
    }

    /// Sets the starting price once. Admin only.
    pub fn initialize(env: Env, sqrt_price_x96: U256) -> i32 {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        bump_instance(&env);
        if env.storage().instance().has(&DataKey::Slot0) {
            panic_with_error!(&env, PoolError::AlreadyInitialized);
        }
        let tick =
            ticks::tick_at_sqrt_price(from_host(&sqrt_price_x96)).unwrap_or_else(|e| fail(&env, e));
        let slot0 = Slot0 {
            sqrt_price_x96: sqrt_price_x96.clone(),
            tick,
        };
        env.storage().instance().set(&DataKey::Slot0, &slot0);
        Initialized {
            sqrt_price_x96,
            tick,
        }
        .publish(&env);
        tick
    }

    pub fn config(env: Env) -> PoolConfig {
        env.storage().instance().get(&DataKey::Config).unwrap()
    }

    pub fn slot0(env: Env) -> Option<Slot0> {
        env.storage().instance().get(&DataKey::Slot0)
    }

    /// `sqrt(1.0001)^tick` in Q64.96.
    pub fn sqrt_price_at_tick(env: Env, tick: i32) -> U256 {
        let v = ticks::sqrt_price_at_tick(tick).unwrap_or_else(|e| fail(&env, e));
        to_host(&env, v)
    }

    /// Greatest tick whose √price is `<= sqrt_price_x96`.
    pub fn tick_at_sqrt_price(env: Env, sqrt_price_x96: U256) -> i32 {
        ticks::tick_at_sqrt_price(from_host(&sqrt_price_x96)).unwrap_or_else(|e| fail(&env, e))
    }
}
