//! Seven-day time-lock enforcement for the Upgrade Proxy.
//!
//! Once an upgrade is proposed it enters a mandatory 7-day (604 800 second)
//! review window.  During this period the community — and automated tooling —
//! can inspect the incoming WASM binary.  Only after the window has fully
//! elapsed can the admin call `execute_upgrade` to apply the swap.
//!
//! # Why seconds, not ledgers?
//!
//! Stellar's ledger close time is ~5 seconds but can vary under network
//! congestion.  Using `env.ledger().timestamp()` (Unix seconds) gives a
//! stable, human-readable guarantee that is independent of ledger velocity.

use soroban_sdk::{contracttype, BytesN, Env};

use crate::error::ProxyError;

/// Mandatory delay between a bytecode proposal and its activation: 7 days.
pub const TIMELOCK_SECONDS: u64 = 7 * 24 * 60 * 60; // 604 800

/// An upgrade queued for future activation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingUpgrade {
    /// Content hash of the incoming WASM binary (returned by
    /// `env.deployer().upload_contract_wasm`).
    pub wasm_hash: BytesN<32>,

    /// Unix timestamp at which the upgrade was proposed.
    pub proposed_at: u64,

    /// The earliest Unix timestamp at which `execute_upgrade` is permitted.
    /// Equals `proposed_at + TIMELOCK_SECONDS`.
    pub execute_after: u64,
}

/// Validate that the timelock for `pending` has elapsed.
///
/// Returns `Ok(())` when `env.ledger().timestamp() >= pending.execute_after`,
/// or `Err(ProxyError::TimelockNotElapsed)` otherwise.
pub fn assert_elapsed(env: &Env, pending: &PendingUpgrade) -> Result<(), ProxyError> {
    if env.ledger().timestamp() < pending.execute_after {
        return Err(ProxyError::TimelockNotElapsed);
    }
    Ok(())
}

/// Compute the `execute_after` timestamp for a proposal submitted *now*.
///
/// Returns `Ok(now + TIMELOCK_SECONDS)` or `Err(ProxyError::ArithmeticOverflow)`
/// if the addition would overflow (cannot occur in practice before ~year 2554).
pub fn execute_after_timestamp(env: &Env) -> Result<u64, ProxyError> {
    let now = env.ledger().timestamp();
    now.checked_add(TIMELOCK_SECONDS)
        .ok_or(ProxyError::ArithmeticOverflow)
}
