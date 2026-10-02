//! Error codes returned by the Upgrade Proxy contract.
//!
//! Every variant is assigned a fixed `u32` discriminant.  These codes are
//! surfaced directly to callers via the Soroban host error mechanism and
//! should never be re-ordered or re-numbered once the contract is deployed.

use soroban_sdk::contracterror;

/// All error conditions that the Upgrade Proxy may return.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ProxyError {
    /// `initialize` was called on an already-initialised contract.
    AlreadyInitialized = 1,

    /// A function that requires prior initialisation was called before
    /// `initialize`.
    NotInitialized = 2,

    /// The caller is not permitted to perform the requested operation.
    Unauthorized = 3,

    /// A new upgrade was proposed while one is already pending.  Call
    /// `abort_upgrade` first to replace an existing proposal.
    UpgradeAlreadyPending = 4,

    /// An operation that requires a pending upgrade (e.g. `execute_upgrade`
    /// or `abort_upgrade`) was called when no upgrade is queued.
    NoPendingUpgrade = 5,

    /// `execute_upgrade` was called before the mandatory 7-day window elapsed.
    TimelockNotElapsed = 6,

    /// Arithmetic overflow detected (should never occur in practice but is
    /// included as a safety net).
    ArithmeticOverflow = 7,
}
