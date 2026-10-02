//! Shared storage key definitions for the yield-stablecoin contract.
//!
//! Centralising all [`DataKey`] variants here ensures that `lib.rs` and
//! `hooks.rs` reference the exact same key layout without duplication.

use soroban_sdk::{contracttype, Address};

/// All storage keys used by the contract.
///
/// Keys stored in **instance** storage (admin config):
/// - `Admin`, `Name`, `TokenSymbol`, `Decimals`, `TotalSupply`,
///   `MultiSigSigners`, `MultiSigThreshold`, `UnblacklistThreshold`
///
/// Keys stored in **persistent** storage (per-address data):
/// - `Balance(addr)`, `Allowance(owner, spender)`,
///   `Blacklisted(addr)`, `Frozen(addr)`
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    // ── Instance keys (admin / token config) ──────────────────────────────
    /// Primary administrator address.
    Admin,
    /// Human-readable token name (e.g. "USD Yield Coin").
    Name,
    /// Ticker symbol (e.g. "USDY").
    TokenSymbol,
    /// Number of decimal places (immutable after initialization).
    Decimals,
    /// Total circulating supply.
    TotalSupply,
    /// Registered multi-sig signer set.
    MultiSigSigners,
    /// Number of signers required for standard privileged operations.
    MultiSigThreshold,
    /// Stricter quorum required to remove an address from the sanctions list.
    UnblacklistThreshold,

    // ── Persistent keys (per-address) ─────────────────────────────────────
    /// Token balance for a holder.
    Balance(Address),
    /// Spending allowance granted by `owner` (inner `Address`) to
    /// `spender` (outer `Address`).
    ///
    /// Note: Soroban's `#[contracttype]` tuple variants serialize the fields
    /// in declaration order, giving a unique key per (owner, spender) pair.
    Allowance(Address, Address),
    /// `true` if the address is on the on-chain sanctions list.
    Blacklisted(Address),
    /// `true` if the address is temporarily frozen.
    Frozen(Address),
}
