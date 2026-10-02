//! # Index Basket
//!
//! A Soroban contract that bundles several SEP-41 tokens into one fully
//! collateralised basket token (e.g. 50% XLM / 30% USDC / 20% sBTC). The
//! basket contract *is* the share token (SEP-41), so shares are transferable
//! like any other Soroban token.
//!
//! ## Issuance and redemption
//!
//! * [`IndexBasket::issue`] pulls every underlying asset in the exact ratio
//!   currently held by the basket and mints shares. The very first issuance
//!   (or one after the basket was fully redeemed) uses the per-share
//!   `units` composition instead.
//! * [`IndexBasket::redeem`] burns shares and releases the pro-rata share of
//!   every underlying asset in the same invocation.
//!
//! Deposits round **up** and withdrawals round **down**, so the basket can
//! never become under-collateralised through rounding, whatever the
//! precision of the underlying tokens. Reserves are tracked internally, so
//! unsolicited token donations cannot be used to manipulate share prices.
//!
//! ## Atomicity
//!
//! Every multi-asset movement (issue, redeem, rebalance) validates all
//! amounts, commits state, then performs all token transfers within a single
//! contract invocation. Soroban invocations are atomic: if any single transfer
//! fails (frozen trustline, missing balance, ...) the entire call reverts and
//! no partial execution is possible.
//!
//! ## Rebalancing
//!
//! See [`rebalance`] for the oracle-guided arbitrageur flow that restores the
//! target value weights after prices drift.
//!
//! ## Storage layout
//!
//! | Key | Type | Storage |
//! |-----|------|---------|
//! | `Admin`, `Name`, `Symbol`, `Decimals` | scalar | Instance |
//! | `Components` | `Vec<Component>` | Instance |
//! | `Reserves` | `Vec<i128>` | Instance |
//! | `TotalSupply` | `i128` | Instance |
//! | `Params` | `RebalanceParams` | Instance |
//! | `Arbitrageur(addr)` | `bool` | Persistent |
//! | `Balance(addr)` | `i128` | Persistent |
//! | `Allowance(from, spender)` | `AllowanceValue` | Temporary |

#![no_std]

pub mod rebalance;
mod token;
mod types;

#[cfg(test)]
mod test;

pub use rebalance::{OracleAsset, PriceData};
pub use types::*;

use soroban_sdk::{
    contract, contractimpl, panic_with_error, token::TokenClient, Address, Env, Vec, I256,
};

#[contract]
pub struct IndexBasket;

pub(crate) fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_THRESHOLD, INSTANCE_BUMP);
}

fn read_admin(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::Admin).unwrap()
}

fn require_admin(env: &Env) -> Address {
    let admin = read_admin(env);
    admin.require_auth();
    admin
}

fn read_components(env: &Env) -> Vec<Component> {
    env.storage().instance().get(&DataKey::Components).unwrap()
}

fn read_reserves(env: &Env) -> Vec<i128> {
    env.storage().instance().get(&DataKey::Reserves).unwrap()
}

pub(crate) fn write_reserves(env: &Env, reserves: &Vec<i128>) {
    env.storage().instance().set(&DataKey::Reserves, reserves);
}

fn read_params(env: &Env) -> RebalanceParams {
    env.storage().instance().get(&DataKey::Params).unwrap()
}

fn read_decimals(env: &Env) -> u32 {
    env.storage().instance().get(&DataKey::Decimals).unwrap()
}

fn validate_params(env: &Env, params: &RebalanceParams) {
    if params.max_price_age == 0 || params.incentive_bps > MAX_INCENTIVE_BPS {
        panic_with_error!(env, BasketError::InvalidConfig);
    }
}

/// `a * b / d` computed in 256 bits, rounded up or down.
fn mul_div(env: &Env, a: i128, b: i128, d: i128, round_up: bool) -> i128 {
    let num = I256::from_i128(env, a).mul(&I256::from_i128(env, b));
    let den = I256::from_i128(env, d);
    let mut q = num.div(&den);
    if round_up && q.mul(&den) != num {
        q = q.add(&I256::from_i32(env, 1));
    }
    q.to_i128()
        .unwrap_or_else(|| panic_with_error!(env, BasketError::Overflow))
}

fn pow10(exp: u32) -> i128 {
    10i128.pow(exp)
}

/// Underlying amounts required to mint `shares` (rounded up).
fn issue_amounts(
    env: &Env,
    components: &Vec<Component>,
    reserves: &Vec<i128>,
    shares: i128,
) -> Vec<i128> {
    let supply = token::total_supply(env);
    let one_share = pow10(read_decimals(env));
    let mut amounts = Vec::new(env);
    for (component, reserve) in components.iter().zip(reserves.iter()) {
        let amount = if supply == 0 {
            mul_div(env, shares, component.units, one_share, true)
        } else {
            mul_div(env, reserve, shares, supply, true)
        };
        amounts.push_back(amount);
    }
    amounts
}

/// Underlying amounts released when burning `shares` (rounded down). The
/// final redeemer receives every remaining unit, including rounding dust.
fn redeem_amounts(env: &Env, reserves: &Vec<i128>, shares: i128) -> Vec<i128> {
    let supply = token::total_supply(env);
    if shares > supply {
        panic_with_error!(env, BasketError::InsufficientSupply);
    }
    let mut amounts = Vec::new(env);
    for reserve in reserves.iter() {
        let amount = if shares == supply {
            reserve
        } else {
            mul_div(env, reserve, shares, supply, false)
        };
        amounts.push_back(amount);
    }
    amounts
}

fn check_length(env: &Env, expected: u32, actual: u32) {
    if expected != actual {
        panic_with_error!(env, BasketError::LengthMismatch);
    }
}

#[contractimpl]
impl IndexBasket {
    // -----------------------------------------------------------------------
    // Construction
    // -----------------------------------------------------------------------

    /// Creates the basket. Panics (aborting the deployment) on any invalid
    /// configuration.
    pub fn __constructor(env: Env, admin: Address, config: BasketConfig) {
        let n = config.components.len();
        if n < 2 || config.name.is_empty() || config.symbol.is_empty() {
            panic_with_error!(&env, BasketError::InvalidConfig);
        }
        if n > MAX_COMPONENTS {
            panic_with_error!(&env, BasketError::TooManyComponents);
        }
        if config.decimals > MAX_DECIMALS {
            panic_with_error!(&env, BasketError::InvalidDecimals);
        }
        let params = RebalanceParams {
            oracle: config.oracle,
            max_price_age: config.max_price_age,
            incentive_bps: config.rebalance_incentive_bps,
        };
        validate_params(&env, &params);

        let this = env.current_contract_address();
        let mut components: Vec<Component> = Vec::new(&env);
        let mut reserves: Vec<i128> = Vec::new(&env);
        let mut weight_sum: u32 = 0;
        for spec in config.components.iter() {
            if spec.token == this {
                panic_with_error!(&env, BasketError::InvalidConfig);
            }
            if components.iter().any(|c| c.token == spec.token) {
                panic_with_error!(&env, BasketError::DuplicateComponent);
            }
            if spec.units <= 0 {
                panic_with_error!(&env, BasketError::InvalidAmount);
            }
            if spec.weight_bps == 0 || spec.weight_bps > BPS {
                panic_with_error!(&env, BasketError::InvalidWeights);
            }
            weight_sum += spec.weight_bps;

            let decimals = TokenClient::new(&env, &spec.token).decimals();
            if decimals > MAX_DECIMALS {
                panic_with_error!(&env, BasketError::InvalidDecimals);
            }
            components.push_back(Component {
                token: spec.token,
                decimals,
                units: spec.units,
                weight_bps: spec.weight_bps,
            });
            reserves.push_back(0);
        }
        if weight_sum != BPS {
            panic_with_error!(&env, BasketError::InvalidWeights);
        }

        let storage = env.storage().instance();
        storage.set(&DataKey::Admin, &admin);
        storage.set(&DataKey::Name, &config.name);
        storage.set(&DataKey::Symbol, &config.symbol);
        storage.set(&DataKey::Decimals, &config.decimals);
        storage.set(&DataKey::Components, &components);
        storage.set(&DataKey::Reserves, &reserves);
        storage.set(&DataKey::TotalSupply, &0i128);
        storage.set(&DataKey::Params, &params);
        bump_instance(&env);
    }

    // -----------------------------------------------------------------------
    // Issuance / redemption
    // -----------------------------------------------------------------------

    /// Deposits every underlying asset in the basket's exact ratio and mints
    /// `shares` basket tokens to `user`.
    ///
    /// `max_amounts` bounds the deposit of each component (same order as
    /// `components`). Returns the amounts actually deposited.
    pub fn issue(env: Env, user: Address, shares: i128, max_amounts: Vec<i128>) -> Vec<i128> {
        user.require_auth();
        bump_instance(&env);
        if shares <= 0 {
            panic_with_error!(&env, BasketError::InvalidAmount);
        }
        let components = read_components(&env);
        let mut reserves = read_reserves(&env);
        check_length(&env, components.len(), max_amounts.len());

        let amounts = issue_amounts(&env, &components, &reserves, shares);
        for (i, amount) in amounts.iter().enumerate() {
            let i = i as u32;
            if amount > max_amounts.get_unchecked(i) {
                panic_with_error!(&env, BasketError::SlippageExceeded);
            }
            let reserve = reserves
                .get_unchecked(i)
                .checked_add(amount)
                .unwrap_or_else(|| panic_with_error!(&env, BasketError::Overflow));
            reserves.set(i, reserve);
        }
        write_reserves(&env, &reserves);
        token::mint_shares(&env, &user, shares);

        let this = env.current_contract_address();
        for (component, amount) in components.iter().zip(amounts.iter()) {
            if amount > 0 {
                TokenClient::new(&env, &component.token).transfer(&user, &this, &amount);
            }
        }

        Issued {
            to: user,
            shares,
            amounts: amounts.clone(),
        }
        .publish(&env);
        amounts
    }

    /// Burns `shares` basket tokens held by `user` and releases the pro-rata
    /// share of every underlying asset in one atomic multi-transfer.
    ///
    /// `min_amounts` bounds the payout of each component. Returns the amounts
    /// released.
    pub fn redeem(env: Env, user: Address, shares: i128, min_amounts: Vec<i128>) -> Vec<i128> {
        user.require_auth();
        bump_instance(&env);
        if shares <= 0 {
            panic_with_error!(&env, BasketError::InvalidAmount);
        }
        if token::balance_of(&env, &user) < shares {
            panic_with_error!(&env, BasketError::InsufficientBalance);
        }
        let mut components = read_components(&env);
        let mut reserves = read_reserves(&env);
        check_length(&env, components.len(), min_amounts.len());

        let supply = token::total_supply(&env);
        let amounts = redeem_amounts(&env, &reserves, shares);
        if amounts.iter().all(|a| a == 0) {
            panic_with_error!(&env, BasketError::InvalidAmount);
        }
        for (i, amount) in amounts.iter().enumerate() {
            let i = i as u32;
            if amount < min_amounts.get_unchecked(i) {
                panic_with_error!(&env, BasketError::SlippageExceeded);
            }
        }

        // Full redemption: carry the latest (post-rebalance) per-share
        // composition forward so a later bootstrap issuance starts on target.
        if shares == supply {
            let one_share = pow10(read_decimals(&env));
            let mut units = Vec::new(&env);
            for reserve in reserves.iter() {
                units.push_back(mul_div(&env, reserve, one_share, supply, false));
            }
            if units.iter().all(|u| u > 0) {
                for (i, unit) in units.iter().enumerate() {
                    let mut c = components.get_unchecked(i as u32);
                    c.units = unit;
                    components.set(i as u32, c);
                }
                env.storage()
                    .instance()
                    .set(&DataKey::Components, &components);
            }
        }

        for (i, amount) in amounts.iter().enumerate() {
            let i = i as u32;
            reserves.set(i, reserves.get_unchecked(i) - amount);
        }
        write_reserves(&env, &reserves);
        token::burn_shares(&env, &user, shares);

        let this = env.current_contract_address();
        for (component, amount) in components.iter().zip(amounts.iter()) {
            if amount > 0 {
                TokenClient::new(&env, &component.token).transfer(&this, &user, &amount);
            }
        }

        Redeemed {
            from: user,
            shares,
            amounts: amounts.clone(),
        }
        .publish(&env);
        amounts
    }

    /// Underlying amounts required to issue `shares` right now.
    pub fn quote_issue(env: Env, shares: i128) -> Vec<i128> {
        if shares <= 0 {
            panic_with_error!(&env, BasketError::InvalidAmount);
        }
        issue_amounts(&env, &read_components(&env), &read_reserves(&env), shares)
    }

    /// Underlying amounts released by redeeming `shares` right now.
    pub fn quote_redeem(env: Env, shares: i128) -> Vec<i128> {
        if shares <= 0 {
            panic_with_error!(&env, BasketError::InvalidAmount);
        }
        redeem_amounts(&env, &read_reserves(&env), shares)
    }

    // -----------------------------------------------------------------------
    // Rebalancing
    // -----------------------------------------------------------------------

    /// Swaps `amount_in` of an underweight component into the basket for an
    /// overweight component at oracle prices plus the configured incentive.
    ///
    /// Only allow-listed arbitrageurs may call this. The trade must not push
    /// either leg past its target weight. Returns the amount of `asset_out`
    /// paid to the arbitrageur.
    pub fn rebalance(
        env: Env,
        arbitrageur: Address,
        asset_in: Address,
        amount_in: i128,
        asset_out: Address,
        min_amount_out: i128,
    ) -> i128 {
        arbitrageur.require_auth();
        bump_instance(&env);
        if !Self::is_arbitrageur(env.clone(), arbitrageur.clone()) {
            panic_with_error!(&env, BasketError::NotArbitrageur);
        }
        let components = read_components(&env);
        let mut reserves = read_reserves(&env);
        let amount_out = rebalance::execute(
            &env,
            &components,
            &mut reserves,
            &read_params(&env),
            &arbitrageur,
            &asset_in,
            amount_in,
            &asset_out,
            min_amount_out,
        );
        Rebalanced {
            arbitrageur,
            asset_in,
            amount_in,
            asset_out,
            amount_out,
        }
        .publish(&env);
        amount_out
    }

    /// Largest rebalancing trade currently allowed for the pair, and its
    /// payout.
    pub fn quote_rebalance(env: Env, asset_in: Address, asset_out: Address) -> RebalanceQuote {
        rebalance::quote(
            &env,
            &read_components(&env),
            &read_reserves(&env),
            &read_params(&env),
            &asset_in,
            &asset_out,
        )
    }

    /// Current value weights versus targets, both in parts per million.
    pub fn allocations(env: Env) -> Vec<Allocation> {
        let components = read_components(&env);
        let reserves = read_reserves(&env);
        let val = rebalance::valuation(&env, &components, &reserves, &read_params(&env));
        let mut out = Vec::new(&env);
        for (i, component) in components.iter().enumerate() {
            let i = i as u32;
            out.push_back(Allocation {
                token: component.token,
                reserve: reserves.get_unchecked(i),
                target_ppm: component.weight_bps * (PPM / BPS),
                weight_ppm: rebalance::weight_of(
                    &env,
                    &val.values.get_unchecked(i),
                    &val.total,
                    PPM,
                ),
            });
        }
        out
    }

    // -----------------------------------------------------------------------
    // Administration
    // -----------------------------------------------------------------------

    /// Grants or revokes rebalancing rights.
    pub fn set_arbitrageur(env: Env, arbitrageur: Address, enabled: bool) {
        require_admin(&env);
        bump_instance(&env);
        let key = DataKey::Arbitrageur(arbitrageur.clone());
        if enabled {
            env.storage().persistent().set(&key, &true);
            env.storage()
                .persistent()
                .extend_ttl(&key, BALANCE_THRESHOLD, BALANCE_BUMP);
        } else {
            env.storage().persistent().remove(&key);
        }
        ArbitrageurSet {
            arbitrageur,
            enabled,
        }
        .publish(&env);
    }

    /// Updates the oracle, price staleness bound and arbitrageur incentive.
    pub fn set_rebalance_params(env: Env, params: RebalanceParams) {
        require_admin(&env);
        validate_params(&env, &params);
        bump_instance(&env);
        env.storage().instance().set(&DataKey::Params, &params);
        ParamsUpdated { params }.publish(&env);
    }

    /// Transfers administration. Both the current and new admin must sign.
    pub fn set_admin(env: Env, new_admin: Address) {
        require_admin(&env);
        new_admin.require_auth();
        bump_instance(&env);
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        AdminChanged { admin: new_admin }.publish(&env);
    }

    // -----------------------------------------------------------------------
    // Views
    // -----------------------------------------------------------------------

    pub fn admin(env: Env) -> Address {
        read_admin(&env)
    }

    pub fn components(env: Env) -> Vec<Component> {
        read_components(&env)
    }

    pub fn reserves(env: Env) -> Vec<i128> {
        read_reserves(&env)
    }

    pub fn total_supply(env: Env) -> i128 {
        token::total_supply(&env)
    }

    pub fn rebalance_params(env: Env) -> RebalanceParams {
        read_params(&env)
    }

    pub fn is_arbitrageur(env: Env, arbitrageur: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::Arbitrageur(arbitrageur))
            .unwrap_or(false)
    }
}
