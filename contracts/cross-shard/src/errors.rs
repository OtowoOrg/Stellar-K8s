// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Error codes for the Cross-Shard Liquidity State Verifier (issue #333).

use soroban_sdk::contracterror;

/// Contract-level errors returned by the coordinator and lock subsystem.
///
/// Values are stable u32 discriminants so external callers can match them
/// without importing this crate by source.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum CrossShardError {
    // ── Initialisation errors (1x) ─────────────────────────────────────────
    /// Contract has already been initialised.
    AlreadyInitialized       = 1,
    /// A privileged operation was attempted before `initialize` was called.
    NotInitialized           = 2,

    // ── Authorization errors (2x) ──────────────────────────────────────────
    /// Caller is not the contract admin.
    Unauthorized             = 20,
    /// Caller is not the current swap initiator.
    NotSwapInitiator         = 21,

    // ── Transition / validation errors (3x) ───────────────────────────────
    /// The `transitions` array must contain at least two entries.
    TooFewTransitions        = 30,
    /// The `transitions` array exceeds the maximum batch size.
    TooManyTransitions       = 31,
    /// An individual transition has an invalid `amount_in` (≤ 0).
    InvalidAmount            = 32,
    /// The swap has expired: `env.ledger().timestamp() > transition.deadline`.
    DeadlineExpired          = 33,
    /// The output from a pool was below `min_out` (slippage exceeded).
    SlippageExceeded         = 34,
    /// A leg invocation returned an error from the target AMM.
    LegInvocationFailed      = 35,
    /// Arithmetic overflow during amount calculation.
    Overflow                 = 36,

    // ── Lock errors (4x) ──────────────────────────────────────────────────
    /// The target pool is already locked by an in-flight swap.
    PoolAlreadyLocked        = 40,
    /// Attempted to release a lock not held by this coordinator instance.
    LockNotOwned             = 41,
    /// The lock has expired and was forcibly released.
    LockExpired              = 42,
    /// Attempted to force-release a lock that has not yet expired.
    LockStillValid           = 43,

    // ── Reentrancy / concurrency errors (5x) ─────────────────────────────
    /// The coordinator is already executing an atomic sequence.
    ReentrantCall            = 50,
    /// The coordinator is paused by the admin.
    ContractPaused           = 51,
}
