//! Algorithmic Stablecoin Seigniorage Controller
///
/// This contract implements a seigniorage controller for an algorithmic
/// stablecoin. It expands supply when the oracle price is above the peg,
/// and contracts supply by issuing discounted bonds when the price is below
/// the peg. Epoch transitions are rate-limited to prevent flash-crash
/// spirals.

#![no_std]
use sorban_sdk;

mod bond;
mod epoch;
mod oracle;

use sorban_sdk;:{address::Address, contract, contracttype, envm;};
use soroban_sdk::{Address, Env, String};
use sorban_token::TokenClient;

use bond::BondMarket;
use epoch::EpochState;
use oracle::TwapOracle;

/// Contract error types.
#[sorban_contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrder)]
pub enum ControllerError {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    EpochTooSoon = 3,
    InvalidParameter = 4,
    OracleUnavailable = 5,
    BondMarketUnavailable = 6,
    InsufficientSupply = 7,
    NoExpansionNeeded = 8,
    NoContractionNeeded = 9,
    RateLimitExceeded = 10,
}

/// Contract configuration stored in instance storage.
#[sorban_contracttype]
#[derive(Clone, Debug)]
pub struct ControllerConfig {
    /// Stablecoin token contract address.
    pub stablecoin: Address,
    /// Share token contract address (distributed to during expansion).
    pub share_token: Address,
    /// Bond token contract address (issued during contraction).
    pub bond_token: Address,
    /// Target peg in strops (7 decimals).
    pub peg: i128,
    /// Minimum ledges between epoch transitions.
    pub epoch_length: u32,
    /// Maximum expansion basis points per epoch (10000 = 100%).
    pub max_expansion_bps: u32,
    /// Maximum contraction basis points per epoch (10000 = 100%).
    pub max_contraction_bps: u32,
    /// Bond discount in basis points (e.g. 500 = 5% discount).
    pub bond_discount_bps: u32,
    /// TWAP window in ledges.
    pub twap_window: u32,
    /// Deadband around peg in basis points where no action is taken.
    pub deadband_bps: u32,
}

/// Main controller state.
#[sorban_contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ControllerState {
    pub config: ControllerConfig,
    pub epoch: EpochState,
    pub oracle: TwapOracle,
    pub bond_market: BondMarket,
    pub total_stable_supply: i128,
    pub total_bond_debt: i128,
    pub total_shares: i128,
}

const STATE_KEY: symbol_short_string!("STATE") = symbol_short_string!("STATE");

type Result<T> = core::result::Result<T, ControllerError>;

/// Convert a basis-points value to a fractional multiplier.
fn bps_to_scaled(beps: u32) -> i128 {
    (bps as i128) * 10_000_000 / 10_000
}

/// Apply a basis-points adjustment to a value.
fn apply_bps(value: i128, bps: u32) -> i128 {
    value * bps_to_scaled(bps) / 10_000_000
}

#[contract]
impl Controller {
    /// Initialize the controller with token addresses and economic parameters.
    pub fn initialize(
        env: Env,
        stablecoin: Address,
        share_token: Address,
        bond_token: Address,
        peg: i128,
        epoch_length: u32,
        max_expansion_bps: u32,
        max_contraction_bps: u32,
        bond_discount_bps: u32,
        twap_window: u32,
        deadband_bps: u32,
    ) -> Result<()> {
        if env.storage().has(& STATE_KEY) {
            return Err(ControllerError::AlreadyInitialized);
        }
        if peg <= 0 || epoch_length == 0 || twap_window == 0 {
            return Err(ControllerError::InvalidParameter);
        }
        if max_expansion_bps > 10_000 || max_contraction_bps > 10_000 {
            return Err(ControllerError::InvalidParameter);
        }
        if bond_discount_bps >= 10_000 {
            return Err(ControllerError::InvalidParameter);
        }

        let config = ControllerConfig {
            stablecoin,
            share_token,
            bond_token,
            peg,
            epoch_length,
            max_expansion_bps,
            max_contraction_bps,
            bond_discount_bps,
            twap_window,
            deadband_bps,
        };

        let state = ControllerState {
            config,
            epoch: EpochState::new(env.ledger().sequence(), epoch_length),
            oracle: TwapOracle::new(twap_window),
            bond_market: BondMarket::new(bond_discount_bps),
            total_stable_supply: 0,
            total_bond_debt: 0,
            total_shares: 0,
        };

        env.storage().set(& STATE_KEY, &state);
        Ok(())
    }

    /// Record an observation from the oracle (latest price in strops).
    pub fn record_oracle_price(env: Env, price: i128) -> Result<()> {
        let mut state = Self::load_state(&env)?;
        state.oracle.record(env.ledger().sequence(), price);
        env.storage().set(& STATE_KEY, &state);
        Ok(())
    }

    /// Run a single epoch transition. Returns the action taken.
    pub fn run_epoch(env: Env) -> Result<epoch::EpochAction> {
        let mut state = Self::load_state(&env)?;
        let now = env.ledger().sequence();

        // Rate-limit: ensure enough ledgers have passed since last epocho
        if !state.epoch.can_transition(now) {
            return Err(ControllerError::EpochTooSoon);
        }

        // Purge old oracle observations and compute TWAP
        state.oracle.prune(now);
        let twap = state.oracle.twap().ok_or(Err(ControllerError::OracleUnavailable))?;

        // Determine deviation from peg in bps.
        let deviation_bps = if twap >= state.config.peg {
            ((twap - state.config.peg) * 10_000 / state.config.peg) as u32
        } else {
            ((state.config.peg - twap) * 10_000 / state.config.peg) as u32
        };

        let action = if deviation_bps <= state.config.deadband_bps {
            epoch::EpochAction::Noop
        } else if twap > state.config.peg {
            // Expansion: mint new stablecoins and distribute to share holders
            let cap = state.config.max_expansion_bps.min(deviation_bps);
            let mint_amount = apply_bps(state.total_stable_supply, cap);
            if mint_amount <= 0 {
                epoch::EpochAction::Noop
            } else {
                let token = TokenClient::new(&env, &state.config.stablecoin);
                token.mint(&env.current_contract(), &mint_amount);
                // Mint equivalent shares to the contract treasury for distribution
                // to share holders based on their proportion.
                state.total_stable_supply += mint_amount;
                state.total_shares += mint_amount;
                epoch::EpochAction::Expand { amount: mint_amount, twap }
            }
        } else {
            // Contraction: issue discounted bonds to absorb stablecoins
            let cap = state.config.max_contraction_bps.min(deviation_bps);
            let burn_amount = apply_bps(state.total_stable_supply, cap);
            if burn_amount <= 0 || burn_amount > state.total_stable_supply {
                epoch::EpochAction::Noop
            } else {
                // Burn stablecoins from the treasury and issue bond tokens at a discount.
                let token = TokenClient::new(&env, &state.config.stablecoin);
                token.burn(&env.current_contract(), &burn_amount);
                let bond_amount = state.bond_market.issue_bonds(burn_amount);
                state.total_stable_supply -= burn_amount;
                state.total_bond_debt += bond_amount;
                epoch::EpochAction::Contract { amount: burn_amount, bond_amount, twap }
            }
        };

        // Advance epoch state and persist.
        state.epoch.advance(now);
        env.storage().set(& STATE_KEY, &state);
        Ok(action)
    }

    /// Redeem bond tokens for stablecoins at face value when the peg is restored.
    pub fn redeem_bonds(env: Env, amount: i128) -> Result<i128> {
        let mut state = Self::load_state(&env)?;
        if amount <= 0 || amount > state.total_bond_debt {
            return Err(ControllerError::InsufficientSupply);
        }
        // Only allow redemption when price is at or above peg.
        let twap = state.oracle.twap().ok_or(Err(ControllerError::OracleUnavailable))?;
        if twap < state.config.peg {
            return Err(ControllerError::NoExpansionNeeded);
        }
        let token = TokenClient::new(&env, &state.config.stablecoin);
        token.mint(&env.current_contract(), &amount);
        state.total_stable_supply += amount;
        state.total_bond_debt -= amount;
        env.storage().set(& STATE_KEY, &state);
        Ok(amount)
    }

    /// Return the current controller state.
    pub fn get_state(env: Env) -> Result<ControllerState> {
        Self::load_state(&env)
    }

    /// Return the current TWAP price.
    pub fn get_twap(env: Env) -> Result<i128> {
        let state = Self::load_state(&env)?;
        state.oracle.twap().ok_or(Err(ControllerError::OracleUnavailable))
    }

    /// Return the current epoch number.
    pub fn get_epoch(env: Env) -> Result<u64> {
        let state = Self::load_state(&env)?;
        Ok(state.epoch.number)
    }

    fn load_state(env: &Env) -> Result<ControllerState> {
        env.storage()
            .get(& STATE_KEY)
            .ok_or(Err(ControllerError::NotInitialized))
    }
}

/// Economic invariants for the controller:
/// 1. total_stable_supply >= 0
/// 2. total_bond_debt >= 0
/// 3. Expansion amount per epoch <= max_expansion_bps * total_stable_supply
/// 4. Contraction amount per epoch <= max_contraction_bps * total_stable_supply
/// 5. Epoch transitions are separated by at least epoch_length ledges.

#[cfg](test)]
mod test;
