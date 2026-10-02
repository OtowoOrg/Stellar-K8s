// Copyright 2024 Stellar K8s Contributors
// SPDX-License-Identifier: Apache-2.0

//! Custom error codes for the Ephemeral Oracle contract.

use soroban_sdk::contracterror;

/// All failure modes for the ephemeral-oracle contract.
///
/// Error codes start at 1 (0 is reserved by the SDK for success).
#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum OracleError {
    /// The contract has already been initialised; call `initialize` only once.
    AlreadyInitialized = 1,

    /// An operation that requires prior initialisation was attempted before
    /// `initialize` was called.
    NotInitialized = 2,

    /// The caller is not the registered admin/updater address.
    Unauthorized = 3,

    /// The asset symbol string is empty or exceeds the 32-byte limit.
    InvalidAsset = 4,

    /// The supplied price value is zero or overflows the allowed range.
    InvalidPrice = 5,

    /// The TTL value supplied was zero or exceeded `MAX_PRICE_TTL`.
    InvalidTtl = 6,

    /// The price entry for the requested asset does not exist or has already
    /// expired and been purged from ledger state.
    PriceNotFound = 7,

    /// The price feed TTL has expired; dependent DeFi transactions should
    /// revert to avoid acting on stale data.
    PriceFeedExpired = 8,

    /// Attempt to write price data to Persistent Storage was blocked.
    /// The contract explicitly prevents this to maintain zero-growth state.
    PersistentStorageForbidden = 9,

    /// Batch update exceeded the maximum allowed batch size.
    BatchTooLarge = 10,

    /// A batch update contained an empty asset list.
    EmptyBatch = 11,
}
