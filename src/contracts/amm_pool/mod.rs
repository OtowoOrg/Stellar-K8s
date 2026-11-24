//! Dynamic Liquidity Pool Automated Market Maker (AMM) primitive.
//!
//! Provides a Soroban-compatible constant-product AMM with LP token minting /
//! burning, fee-bearing swaps, and slippage protection.
//!
//! # Sub-modules
//!
//! - [`math`] — fixed-point arithmetic: `isqrt`, fee helpers, swap formulas, LP share math
//! - [`lib`]  — pool state machine (initialise, add/remove liquidity, swap)

pub mod lib;
pub mod math;

pub use lib::{AmmError, AmmPool, PoolEvent, PoolState, TokenSide};
pub use math::{
    bps_to_scale, get_amount_in, get_amount_out, initial_lp_shares, isqrt, lp_to_amounts,
    subsequent_lp_shares, verify_k_invariant, MathError, FEE_SCALE, LP_SCALE, MINIMUM_LIQUIDITY,
};
