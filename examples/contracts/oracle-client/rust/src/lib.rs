//! Client bindings for the Stellar-K8s gas fee oracle contract.
//!
//! The ABI mirrors `docs/architecture/gas-oracle.md` (section "Contract ABI").
//! Contracts that need a fee quote depend on this crate and call the oracle
//! through [`GasOracleClient`]; [`quote_fee`] shows the recommended pattern.

#![no_std]

pub mod ema;

use soroban_sdk::{contractclient, contracterror, contracttype, Address, Env};

/// Tunable oracle parameters, set by the admin.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleConfig {
    /// Smoothing factor numerator `a`; `alpha = a / 10_000`, `1 <= a <= 10_000`.
    pub alpha_bps: u32,
    /// Smallest accepted observation and quote, in stroops per operation.
    pub min_fee: u64,
    /// Largest accepted observation and quote, in stroops per operation.
    pub max_fee: u64,
    /// Quotes fail with [`OracleError::Stale`] once this many ledgers pass without an update.
    pub max_age_ledgers: u32,
}

/// Oracle state as returned by `state()`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleState {
    /// EMA in fixed point, units of 10^-7 stroops ([`ema::SCALE`]).
    pub ema: u128,
    /// Most recent raw observation, in stroops per operation.
    pub last_observation: u64,
    /// Ledger sequence of the most recent accepted observation.
    pub last_ledger: u32,
    /// Number of observations accepted since initialization.
    pub samples: u64,
}

/// Error codes surfaced as `Error(Contract, #n)`.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum OracleError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InvalidConfig = 4,
    ObservationOutOfRange = 5,
    AlreadyUpdated = 6,
    NoData = 7,
    Stale = 8,
    Overflow = 9,
}

/// Oracle contract interface. The generated [`GasOracleClient`] exposes each
/// function plus a `try_` variant that returns contract errors instead of trapping.
#[contractclient(name = "GasOracleClient")]
pub trait GasOracle {
    fn initialize(env: Env, admin: Address, config: OracleConfig) -> Result<(), OracleError>;
    fn set_config(env: Env, config: OracleConfig) -> Result<(), OracleError>;
    fn add_reporter(env: Env, reporter: Address) -> Result<(), OracleError>;
    fn remove_reporter(env: Env, reporter: Address) -> Result<(), OracleError>;
    fn submit(env: Env, reporter: Address, fee_per_op: u64) -> Result<u64, OracleError>;
    fn fee_per_op(env: Env) -> Result<u64, OracleError>;
    fn estimate_fee(env: Env, ops: u32) -> Result<u64, OracleError>;
    fn state(env: Env) -> Result<OracleState, OracleError>;
    fn config(env: Env) -> Result<OracleConfig, OracleError>;
}

/// Quotes the total inclusion fee for `ops` operations from the oracle at `oracle`.
///
/// Oracle errors are returned to the caller so it can choose a fallback
/// (for example a static fee on [`OracleError::Stale`]). Host-level failures,
/// such as a missing contract, trap the calling transaction.
pub fn quote_fee(env: &Env, oracle: &Address, ops: u32) -> Result<u64, OracleError> {
    match GasOracleClient::new(env, oracle).try_estimate_fee(&ops) {
        Ok(Ok(fee)) => Ok(fee),
        Err(Ok(err)) => Err(err),
        Ok(Err(_)) | Err(Err(_)) => panic!("gas oracle invocation failed"),
    }
}
