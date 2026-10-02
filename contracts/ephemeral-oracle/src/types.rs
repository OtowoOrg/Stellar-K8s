// Copyright 2024 Stellar K8s Contributors
// SPDX-License-Identifier: Apache-2.0

//! Shared data types used across the ephemeral-oracle contract modules.

use soroban_sdk::{contracttype, Address, Bytes, String};

// ---------------------------------------------------------------------------
// Storage key discriminators
// ---------------------------------------------------------------------------

/// Top-level storage key enum.
///
/// Every variant is stored in **Temporary Storage** (except `Admin` and
/// `Config`, which use *Instance Storage* because they are set once and
/// accessed on every call).
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataKey {
    // --- Instance keys (set once, long-lived) ---
    /// Privileged address that may call `initialize`, `set_updater`, and
    /// `update_config`.
    Admin,
    /// The address authorised to push price updates (may be rotated by admin).
    Updater,
    /// Runtime-tunable parameters (TTL, max batch size, …).
    Config,

    // --- Temporary keys (one per asset, evicted after TTL) ---
    /// Live price entry for the given asset symbol.
    ///
    /// Key: `Price(asset_symbol_bytes)`
    Price(Bytes),
}

// ---------------------------------------------------------------------------
// Oracle configuration
// ---------------------------------------------------------------------------

/// Runtime parameters for the oracle contract.
///
/// Stored in Instance Storage and settable by the admin only.
#[contracttype]
#[derive(Clone, Debug)]
pub struct OracleConfig {
    /// Number of ledgers a price entry lives before automatic eviction.
    ///
    /// Soroban evicts Temporary Storage entries whose TTL reaches 0 without
    /// requiring an explicit delete transaction.  Defaults to
    /// `DEFAULT_PRICE_TTL` (50 ledgers ≈ 5 minutes on Testnet).
    pub price_ttl: u32,

    /// Maximum number of assets that can be updated in a single
    /// `batch_update` call.  Bounds transaction metering.
    pub max_batch_size: u32,
}

// ---------------------------------------------------------------------------
// Price entry
// ---------------------------------------------------------------------------

/// A signed price data point for a single asset.
///
/// Stored exclusively in Temporary Storage to allow cost-free eviction once
/// the TTL window closes.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PriceEntry {
    /// Asset symbol as an ASCII byte string (e.g. `b"XLM/USD"`).
    pub asset: Bytes,

    /// Price in the smallest unit (e.g. micro-USD, i.e. price × 10^7).
    /// Using `i128` matches Soroban's native `i128` token amount convention.
    pub price: i128,

    /// Ledger sequence number at which this entry was last written.
    pub timestamp_ledger: u32,

    /// Ed25519 signature over `(asset ‖ price ‖ timestamp_ledger)` produced
    /// by the registered `updater` address.
    ///
    /// Stored as raw 64-byte `Bytes` so it can be verified on-chain.
    pub signature: Bytes,

    /// The updater address whose key was used to sign this entry.
    pub signer: Address,
}

// ---------------------------------------------------------------------------
// Batch update helper
// ---------------------------------------------------------------------------

/// A single asset/price pair used as input to `batch_update`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PriceUpdate {
    /// Asset symbol bytes (same encoding as `PriceEntry::asset`).
    pub asset: Bytes,

    /// Price in the smallest unit.
    pub price: i128,

    /// Ed25519 signature over `(asset ‖ price ‖ current_ledger)`.
    pub signature: Bytes,
}

// ---------------------------------------------------------------------------
// View response
// ---------------------------------------------------------------------------

/// Returned by `get_price` and `get_price_checked`.
///
/// Carries both the price data and the remaining TTL so callers can decide
/// whether the feed is fresh enough for their purposes.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PriceData {
    /// Asset symbol.
    pub asset: String,

    /// Price in smallest unit (× 10^7).
    pub price: i128,

    /// Ledger at which the price was last written.
    pub timestamp_ledger: u32,

    /// Remaining Temporary Storage TTL in ledgers at the time of the query.
    /// If this reaches 0 the entry will be evicted on the next ledger close.
    pub ttl_remaining: u32,
}
