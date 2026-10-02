//! Oracle-guided rebalancing.
//!
//! When prices move, the value weights of the basket drift away from their
//! targets. Allow-listed arbitrageurs restore them by depositing an
//! *underweight* asset and withdrawing an *overweight* one at oracle prices,
//! optionally earning a small, capped premium (`incentive_bps`) on the
//! outgoing leg.
//!
//! ## Valuation
//!
//! Prices come from a SEP-40 oracle quoted per *whole* token. Every trade and
//! weight check only compares ratios of values from the same oracle, so the
//! oracle's own precision cancels out. To make those comparisons exact across
//! tokens of different precision, each component value is scaled to the
//! basket's largest component precision `D` and computed in 256-bit integers:
//!
//! ```text
//! P_i = price_i * 10^(D - decimals_i)       (value per base unit)
//! v_i = reserve_i * P_i                      (exact, no rounding)
//! ```
//!
//! ## Trade rules
//!
//! For `amount_in` of asset `in` the arbitrageur receives
//! `floor(amount_in * P_in * (1 + incentive) / P_out)` of asset `out`. A trade
//! is accepted only if `in` is underweight and `out` overweight beforehand,
//! and neither leg overshoots its target afterwards:
//!
//! ```text
//! v_in'  * BPS <= target_in  * V'
//! v_out' * BPS >= target_out * V'
//! ```
//!
//! Rebalancing therefore always moves the basket monotonically toward its
//! targets and can never be used to push it further out of balance.

use soroban_sdk::{
    contractclient, contracttype, panic_with_error, token, Address, Env, Symbol, Vec, I256,
};

use crate::types::{BasketError, Component, RebalanceParams, RebalanceQuote, BPS};

/// SEP-40 asset identifier.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OracleAsset {
    Stellar(Address),
    Other(Symbol),
}

/// SEP-40 price record.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceData {
    pub price: i128,
    pub timestamp: u64,
}

/// Subset of the SEP-40 price feed interface used by the basket.
#[allow(dead_code)]
#[contractclient(name = "PriceOracleClient")]
pub trait PriceOracle {
    fn lastprice(env: Env, asset: OracleAsset) -> Option<PriceData>;
}

/// Exact valuation of every component at current oracle prices.
pub(crate) struct Valuation {
    /// Value of one base unit of each component (`P_i`).
    pub unit_values: Vec<I256>,
    /// Value of each reserve (`v_i`).
    pub values: Vec<I256>,
    /// Total basket value (`V`).
    pub total: I256,
}

fn pow10(env: &Env, exp: u32) -> I256 {
    I256::from_i32(env, 10).pow(exp)
}

fn i256(env: &Env, v: i128) -> I256 {
    I256::from_i128(env, v)
}

fn i256_u32(env: &Env, v: u32) -> I256 {
    I256::from_i128(env, v.into())
}

fn fetch_price(env: &Env, oracle: &PriceOracleClient, token: &Address, max_age: u64) -> i128 {
    let data = oracle
        .lastprice(&OracleAsset::Stellar(token.clone()))
        .unwrap_or_else(|| panic_with_error!(env, BasketError::PriceUnavailable));
    if data.price <= 0 {
        panic_with_error!(env, BasketError::PriceUnavailable);
    }
    if env.ledger().timestamp().saturating_sub(data.timestamp) > max_age {
        panic_with_error!(env, BasketError::StalePrice);
    }
    data.price
}

pub(crate) fn valuation(
    env: &Env,
    components: &Vec<Component>,
    reserves: &Vec<i128>,
    params: &RebalanceParams,
) -> Valuation {
    let oracle = PriceOracleClient::new(env, &params.oracle);
    let max_decimals = components.iter().map(|c| c.decimals).max().unwrap_or(0);

    let mut unit_values = Vec::new(env);
    let mut values = Vec::new(env);
    let mut total = i256(env, 0);
    for (component, reserve) in components.iter().zip(reserves.iter()) {
        let price = fetch_price(env, &oracle, &component.token, params.max_price_age);
        let unit_value = i256(env, price).mul(&pow10(env, max_decimals - component.decimals));
        let value = unit_value.mul(&i256(env, reserve));
        total = total.add(&value);
        unit_values.push_back(unit_value);
        values.push_back(value);
    }
    Valuation {
        unit_values,
        values,
        total,
    }
}

/// Weight of `value` within `total`, in `scale` units (floored).
pub(crate) fn weight_of(env: &Env, value: &I256, total: &I256, scale: u32) -> u32 {
    if *total <= i256(env, 0) {
        return 0;
    }
    value
        .mul(&i256_u32(env, scale))
        .div(total)
        .to_i128()
        .map_or(scale, |w| w as u32)
}

fn index_of(env: &Env, components: &Vec<Component>, token: &Address) -> u32 {
    components
        .iter()
        .position(|c| c.token == *token)
        .map(|i| i as u32)
        .unwrap_or_else(|| panic_with_error!(env, BasketError::UnknownComponent))
}

fn resolve_pair(
    env: &Env,
    components: &Vec<Component>,
    asset_in: &Address,
    asset_out: &Address,
) -> (u32, u32) {
    if asset_in == asset_out {
        panic_with_error!(env, BasketError::SameAsset);
    }
    (
        index_of(env, components, asset_in),
        index_of(env, components, asset_out),
    )
}

/// `floor(amount_in * P_in * (BPS + incentive) / (BPS * P_out))`.
fn amount_out_for(
    env: &Env,
    val: &Valuation,
    i: u32,
    o: u32,
    amount_in: i128,
    incentive_bps: u32,
) -> I256 {
    i256(env, amount_in)
        .mul(&val.unit_values.get_unchecked(i))
        .mul(&i256_u32(env, BPS + incentive_bps))
        .div(&i256_u32(env, BPS).mul(&val.unit_values.get_unchecked(o)))
}

/// Computes the largest `amount_in` that cannot overshoot either target,
/// accounting for rounding of the outgoing leg and the incentive premium.
///
/// With `B = BPS`, `f = incentive`, `F = B + f`, `x = amount_in * P_in`:
///
/// ```text
/// in  leg: x * (B^2 + t_in * f)             <= t_in * V * B - v_in * B^2
/// out leg: x * (t_out * B + F * (B - t_out)) <= v_out * B^2 - t_out * V * B
/// ```
pub(crate) fn quote(
    env: &Env,
    components: &Vec<Component>,
    reserves: &Vec<i128>,
    params: &RebalanceParams,
    asset_in: &Address,
    asset_out: &Address,
) -> RebalanceQuote {
    let (i, o) = resolve_pair(env, components, asset_in, asset_out);
    let val = valuation(env, components, reserves, params);

    let zero = i256(env, 0);
    let b = i256_u32(env, BPS);
    let b2 = b.mul(&b);
    let f = i256_u32(env, params.incentive_bps);
    let big_f = b.add(&f);
    let t_in = i256_u32(env, components.get_unchecked(i).weight_bps);
    let t_out = i256_u32(env, components.get_unchecked(o).weight_bps);
    let p_in = val.unit_values.get_unchecked(i);

    let in_room = t_in
        .mul(&val.total)
        .mul(&b)
        .sub(&val.values.get_unchecked(i).mul(&b2));
    let out_room = val
        .values
        .get_unchecked(o)
        .mul(&b2)
        .sub(&t_out.mul(&val.total).mul(&b));
    if in_room <= zero || out_room <= zero {
        return RebalanceQuote {
            max_amount_in: 0,
            amount_out: 0,
        };
    }

    let by_in = in_room.div(&p_in.mul(&b2.add(&t_in.mul(&f))));
    let by_out = out_room.div(&p_in.mul(&t_out.mul(&b).add(&big_f.mul(&b.sub(&t_out)))));
    let max_in = if by_in < by_out { by_in } else { by_out };
    let max_amount_in = max_in.to_i128().unwrap_or(i128::MAX);
    let amount_out = amount_out_for(env, &val, i, o, max_amount_in, params.incentive_bps)
        .to_i128()
        .unwrap_or_else(|| panic_with_error!(env, BasketError::Overflow));
    RebalanceQuote {
        max_amount_in,
        amount_out,
    }
}

/// Validates and applies a rebalancing trade against `reserves` (updated in
/// place), then settles both legs. Returns the amount paid out.
#[allow(clippy::too_many_arguments)]
pub(crate) fn execute(
    env: &Env,
    components: &Vec<Component>,
    reserves: &mut Vec<i128>,
    params: &RebalanceParams,
    arbitrageur: &Address,
    asset_in: &Address,
    amount_in: i128,
    asset_out: &Address,
    min_amount_out: i128,
) -> i128 {
    if amount_in <= 0 {
        panic_with_error!(env, BasketError::InvalidAmount);
    }
    let (i, o) = resolve_pair(env, components, asset_in, asset_out);
    let val = valuation(env, components, reserves, params);
    let b = i256_u32(env, BPS);
    let t_in = i256_u32(env, components.get_unchecked(i).weight_bps);
    let t_out = i256_u32(env, components.get_unchecked(o).weight_bps);
    let v_in = val.values.get_unchecked(i);
    let v_out = val.values.get_unchecked(o);

    // Drift preconditions: the incoming asset must be underweight and the
    // outgoing asset overweight at current prices.
    if v_in.mul(&b) >= t_in.mul(&val.total) {
        panic_with_error!(env, BasketError::NotUnderweight);
    }
    if v_out.mul(&b) <= t_out.mul(&val.total) {
        panic_with_error!(env, BasketError::NotOverweight);
    }

    let reserve_in = reserves.get_unchecked(i);
    let reserve_out = reserves.get_unchecked(o);
    let amount_out_wide = amount_out_for(env, &val, i, o, amount_in, params.incentive_bps);
    if amount_out_wide > i256(env, reserve_out) {
        panic_with_error!(env, BasketError::Overshoot);
    }
    let amount_out = amount_out_wide.to_i128().unwrap();
    if amount_out <= 0 {
        panic_with_error!(env, BasketError::InvalidAmount);
    }
    if amount_out < min_amount_out {
        panic_with_error!(env, BasketError::SlippageExceeded);
    }

    // Post-trade targets must not be overshot on either leg.
    let value_in = i256(env, amount_in).mul(&val.unit_values.get_unchecked(i));
    let value_out = i256(env, amount_out).mul(&val.unit_values.get_unchecked(o));
    let new_total = val.total.add(&value_in).sub(&value_out);
    if v_in.add(&value_in).mul(&b) > t_in.mul(&new_total)
        || v_out.sub(&value_out).mul(&b) < t_out.mul(&new_total)
    {
        panic_with_error!(env, BasketError::Overshoot);
    }

    let new_reserve_in = reserve_in
        .checked_add(amount_in)
        .unwrap_or_else(|| panic_with_error!(env, BasketError::Overflow));
    reserves.set(i, new_reserve_in);
    reserves.set(o, reserve_out - amount_out);
    crate::write_reserves(env, reserves);

    let this = env.current_contract_address();
    token::TokenClient::new(env, asset_in).transfer(arbitrageur, &this, &amount_in);
    token::TokenClient::new(env, asset_out).transfer(&this, arbitrageur, &amount_out);
    amount_out
}
