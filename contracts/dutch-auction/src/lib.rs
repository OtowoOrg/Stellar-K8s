//! # Dutch Auction Token Launchpad Primitive
//!
//! A deterministic, front-running-resistant token distribution mechanism for
//! Stellar/Soroban.  The auction starts at a high `start_price` and linearly
//! decays to `reserve_price` over the configured duration.  All bidders pay the
//! **same clearing price** — the price at the moment the auction closes (either
//! at `end_time` or when all tokens sell out).
//!
//! ## Lifecycle
//!
//! ```text
//! initialize()   → PENDING
//! start()        → OPEN       (callable at or after start_time)
//! commit()       → OPEN       (users deposit funds to reserve tokens)
//! settle()       → SETTLED    (callable once all tokens sold OR end_time reached)
//! claim()        → (per-user) tokens + refunds distributed
//! ```
//!
//! ## Key Invariants
//!
//! 1. **Price floor**: clearing price ≥ `reserve_price` always.
//! 2. **No over-allocation**: total tokens committed ≤ `total_tokens`.
//! 3. **Exact refunds**: each user receives `deposit - tokens_bought * clearing_price`.
//! 4. **Single clearing price**: set once at settlement, immutable thereafter.
//! 5. **Idempotent claim**: a user can only claim once.

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, token, Address, Env,
};

pub mod curve;

// ─── Error codes ────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// initialize() has already been called.
    AlreadyInitialized = 1,
    /// The contract has not been initialized yet.
    NotInitialized = 2,
    /// Caller is not the auction admin.
    Unauthorized = 3,
    /// Auction is not in OPEN state for this operation.
    AuctionNotOpen = 4,
    /// Auction has not yet been settled.
    AuctionNotSettled = 5,
    /// Auction is already settled.
    AlreadySettled = 6,
    /// Settlement conditions are not met yet (auction still running and tokens remain).
    SettlementNotReady = 7,
    /// Deposit amount is too small to purchase at least one token.
    DepositTooSmall = 8,
    /// All tokens have already been committed.
    SoldOut = 9,
    /// This user has already claimed.
    AlreadyClaimed = 10,
    /// This user has no commitment to claim.
    NothingToClaim = 11,
    /// Arithmetic overflow or invalid parameter.
    InvalidParameter = 12,
    /// Auction start time has not been reached yet.
    AuctionNotStarted = 13,
}

// ─── Persistent storage keys ────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    // Immutable config (set during initialize)
    Admin,
    TokenAddress,
    ReserveTokenAddress,
    TotalTokens,
    StartPrice,
    ReservePrice,
    StartTime,
    EndTime,

    // Mutable state
    Status,             // AuctionStatus enum
    TokensRemaining,    // i128 – tokens not yet committed
    TotalCommitted,     // i128 – total tokens committed across all users
    TotalDeposits,      // i128 – total reserve-tokens deposited
    ClearingPrice,      // i128 – set at settlement

    // Per-user: keyed by Address
    UserDeposit(Address),   // i128 – how much reserve-token user deposited
    UserClaimed(Address),   // bool – has user already claimed?
}

// ─── Auction lifecycle state ─────────────────────────────────────────────────

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum AuctionStatus {
    /// Initialized but start_time not yet reached.
    Pending = 0,
    /// Auction is live; commits accepted.
    Open = 1,
    /// Auction has closed and clearing price has been set.
    Settled = 2,
}

// ─── Contract ────────────────────────────────────────────────────────────────

#[contract]
pub struct DutchAuctionContract;

#[contractimpl]
impl DutchAuctionContract {
    // ─── Initialization ───────────────────────────────────────────────────

    /// Set up the auction parameters.  Must be called exactly once.
    ///
    /// # Parameters
    /// * `admin`         – address with authority to start/settle the auction
    /// * `token`         – the SPL-style token being sold (the project token)
    /// * `reserve_token` – the payment token (e.g. USDC, XLM)
    /// * `total_tokens`  – total supply offered in this auction
    /// * `start_price`   – price per token at auction open (reserve_token units)
    /// * `reserve_price` – minimum price floor; auction cannot go lower
    /// * `start_time`    – ledger timestamp (seconds) when bidding opens
    /// * `end_time`      – ledger timestamp (seconds) when bidding closes
    pub fn initialize(
        env: Env,
        admin: Address,
        token: Address,
        reserve_token: Address,
        total_tokens: i128,
        start_price: i128,
        reserve_price: i128,
        start_time: u64,
        end_time: u64,
    ) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();

        if total_tokens <= 0 || start_price <= 0 || reserve_price <= 0 {
            return Err(Error::InvalidParameter);
        }
        if start_price < reserve_price {
            return Err(Error::InvalidParameter);
        }
        if end_time <= start_time {
            return Err(Error::InvalidParameter);
        }

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::TokenAddress, &token);
        env.storage().instance().set(&DataKey::ReserveTokenAddress, &reserve_token);
        env.storage().instance().set(&DataKey::TotalTokens, &total_tokens);
        env.storage().instance().set(&DataKey::StartPrice, &start_price);
        env.storage().instance().set(&DataKey::ReservePrice, &reserve_price);
        env.storage().instance().set(&DataKey::StartTime, &start_time);
        env.storage().instance().set(&DataKey::EndTime, &end_time);
        env.storage().instance().set(&DataKey::Status, &AuctionStatus::Pending);
        env.storage().instance().set(&DataKey::TokensRemaining, &total_tokens);
        env.storage().instance().set(&DataKey::TotalCommitted, &0i128);
        env.storage().instance().set(&DataKey::TotalDeposits, &0i128);
        env.storage().instance().set(&DataKey::ClearingPrice, &0i128);

        Ok(())
    }

    // ─── Admin: open auction ──────────────────────────────────────────────

    /// Transition the auction from Pending → Open.
    ///
    /// Requires: current ledger time ≥ start_time.
    /// The admin must transfer `total_tokens` of the project token to this
    /// contract before or during this call.
    pub fn start(env: Env, caller: Address) -> Result<(), Error> {
        caller.require_auth();
        Self::require_admin(&env, &caller)?;

        let status: AuctionStatus = env.storage().instance().get(&DataKey::Status).ok_or(Error::NotInitialized)?;
        if status != AuctionStatus::Pending {
            return Err(Error::AlreadySettled); // re-use: not in the right state
        }

        let start_time: u64 = env.storage().instance().get(&DataKey::StartTime).unwrap();
        let current_time = env.ledger().timestamp();
        if current_time < start_time {
            return Err(Error::AuctionNotStarted);
        }

        env.storage().instance().set(&DataKey::Status, &AuctionStatus::Open);
        Ok(())
    }

    // ─── User: commit funds ───────────────────────────────────────────────

    /// Commit funds to the auction.
    ///
    /// The caller deposits `amount` of reserve-token.  The contract notes how
    /// many tokens the deposit represents *at the current price* for informational
    /// purposes, but the **actual token allocation is determined at settlement**
    /// using the final clearing price.
    ///
    /// - Multiple commits from the same user accumulate.
    /// - If the deposit would exceed the remaining token supply (at the current
    ///   price), only enough to fill the remaining supply is accepted and the
    ///   auction transitions to Sold-Out state (automatically triggering settlement).
    ///
    /// # Returns
    /// `Ok(())` on success.
    pub fn commit(env: Env, bidder: Address, amount: i128) -> Result<(), Error> {
        bidder.require_auth();

        let status: AuctionStatus = env
            .storage()
            .instance()
            .get(&DataKey::Status)
            .ok_or(Error::NotInitialized)?;
        if status != AuctionStatus::Open {
            return Err(Error::AuctionNotOpen);
        }

        if amount <= 0 {
            return Err(Error::InvalidParameter);
        }

        let start_price: i128 = env.storage().instance().get(&DataKey::StartPrice).unwrap();
        let reserve_price: i128 = env.storage().instance().get(&DataKey::ReservePrice).unwrap();
        let start_time: u64 = env.storage().instance().get(&DataKey::StartTime).unwrap();
        let end_time: u64 = env.storage().instance().get(&DataKey::EndTime).unwrap();
        let current_time = env.ledger().timestamp();

        // If we are past end_time the auction should have been settled already —
        // do not accept new commits after the window has closed.
        if current_time >= end_time {
            return Err(Error::AuctionNotOpen);
        }

        let price = curve::current_price(start_price, reserve_price, start_time, end_time, current_time);

        // How many tokens could this deposit buy at the current price?
        let tokens_wanted = curve::tokens_for_deposit(amount, price);
        if tokens_wanted == 0 {
            return Err(Error::DepositTooSmall);
        }

        let tokens_remaining: i128 = env.storage().instance().get(&DataKey::TokensRemaining).unwrap();
        if tokens_remaining == 0 {
            return Err(Error::SoldOut);
        }

        // Cap the commitment at the remaining supply.
        let tokens_to_commit = if tokens_wanted > tokens_remaining {
            tokens_remaining
        } else {
            tokens_wanted
        };

        // The effective deposit cost is based on what we can actually commit.
        // Any excess amount above what's needed stays with the bidder (we only pull
        // what covers the tokens_to_commit at the current price, rounded up to ensure
        // full coverage).
        let deposit_needed = curve::cost_for_tokens(tokens_to_commit, price);
        // If the caller sent more than needed (because of rounding or overshooting
        // supply), we only pull deposit_needed.
        let actual_deposit = if amount > deposit_needed && tokens_to_commit < tokens_wanted {
            // Auction is filling the remaining supply — accept only the needed amount.
            deposit_needed
        } else {
            // Normal case: accept the full amount (user may get some refund at settlement
            // if rounding leaves a remainder).
            amount
        };

        // Pull reserve-tokens from bidder into the contract.
        let reserve_token: Address = env.storage().instance().get(&DataKey::ReserveTokenAddress).unwrap();
        let reserve_client = token::Client::new(&env, &reserve_token);
        reserve_client.transfer(&bidder, &env.current_contract_address(), &actual_deposit);

        // Update per-user deposit.
        let existing_deposit: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::UserDeposit(bidder.clone()))
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&DataKey::UserDeposit(bidder.clone()), &(existing_deposit + actual_deposit));

        // Update global counters.
        let new_remaining = tokens_remaining - tokens_to_commit;
        let total_committed: i128 = env.storage().instance().get(&DataKey::TotalCommitted).unwrap();
        let total_deposits: i128 = env.storage().instance().get(&DataKey::TotalDeposits).unwrap();

        env.storage().instance().set(&DataKey::TokensRemaining, &new_remaining);
        env.storage().instance().set(&DataKey::TotalCommitted, &(total_committed + tokens_to_commit));
        env.storage().instance().set(&DataKey::TotalDeposits, &(total_deposits + actual_deposit));

        // Early sell-out: if all tokens are now committed, close the auction.
        if new_remaining == 0 {
            Self::_settle_internal(&env, price)?;
        }

        Ok(())
    }

    // ─── Admin: settle auction ────────────────────────────────────────────

    /// Settle the auction and lock in the clearing price.
    ///
    /// Settlement is valid when either:
    /// - All tokens have been sold (sell-out), OR
    /// - The auction `end_time` has passed.
    ///
    /// After settlement, users can call `claim()` to receive tokens and refunds.
    pub fn settle(env: Env, caller: Address) -> Result<(), Error> {
        caller.require_auth();
        Self::require_admin(&env, &caller)?;

        let status: AuctionStatus = env
            .storage()
            .instance()
            .get(&DataKey::Status)
            .ok_or(Error::NotInitialized)?;

        // Already settled (e.g. by early sell-out in commit())
        if status == AuctionStatus::Settled {
            return Ok(());
        }

        if status != AuctionStatus::Open {
            return Err(Error::AuctionNotOpen);
        }

        let tokens_remaining: i128 = env.storage().instance().get(&DataKey::TokensRemaining).unwrap();
        let end_time: u64 = env.storage().instance().get(&DataKey::EndTime).unwrap();
        let current_time = env.ledger().timestamp();

        // Must be sold-out OR past the end time.
        if tokens_remaining > 0 && current_time < end_time {
            return Err(Error::SettlementNotReady);
        }

        let start_price: i128 = env.storage().instance().get(&DataKey::StartPrice).unwrap();
        let reserve_price: i128 = env.storage().instance().get(&DataKey::ReservePrice).unwrap();
        let start_time: u64 = env.storage().instance().get(&DataKey::StartTime).unwrap();

        let clearing_price = curve::current_price(start_price, reserve_price, start_time, end_time, current_time);
        Self::_settle_internal(&env, clearing_price)
    }

    // ─── User: claim tokens + refund ─────────────────────────────────────

    /// Claim purchased tokens and any reserve-token refund.
    ///
    /// After settlement, each participant receives:
    /// - `floor(deposit / clearing_price)` project tokens
    /// - `deposit - tokens_bought * clearing_price` reserve-tokens back (refund)
    ///
    /// A user with a deposit that cannot buy a single token at the clearing price
    /// receives a full refund of their deposit.
    pub fn claim(env: Env, bidder: Address) -> Result<(), Error> {
        bidder.require_auth();

        let status: AuctionStatus = env
            .storage()
            .instance()
            .get(&DataKey::Status)
            .ok_or(Error::NotInitialized)?;
        if status != AuctionStatus::Settled {
            return Err(Error::AuctionNotSettled);
        }

        // Idempotency guard
        let already_claimed: bool = env
            .storage()
            .persistent()
            .get(&DataKey::UserClaimed(bidder.clone()))
            .unwrap_or(false);
        if already_claimed {
            return Err(Error::AlreadyClaimed);
        }

        let deposit: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::UserDeposit(bidder.clone()))
            .unwrap_or(0);
        if deposit == 0 {
            return Err(Error::NothingToClaim);
        }

        let clearing_price: i128 = env.storage().instance().get(&DataKey::ClearingPrice).unwrap();
        let tokens_bought = curve::tokens_for_deposit(deposit, clearing_price);
        let tokens_cost = curve::cost_for_tokens(tokens_bought, clearing_price);
        let refund = deposit - tokens_cost;

        // Mark as claimed before any external calls (checks-effects-interactions)
        env.storage()
            .persistent()
            .set(&DataKey::UserClaimed(bidder.clone()), &true);

        // Transfer project tokens to bidder (if any were purchased)
        if tokens_bought > 0 {
            let token: Address = env.storage().instance().get(&DataKey::TokenAddress).unwrap();
            let token_client = token::Client::new(&env, &token);
            token_client.transfer(&env.current_contract_address(), &bidder, &tokens_bought);
        }

        // Return unused reserve-tokens (refund)
        if refund > 0 {
            let reserve_token: Address = env
                .storage()
                .instance()
                .get(&DataKey::ReserveTokenAddress)
                .unwrap();
            let reserve_client = token::Client::new(&env, &reserve_token);
            reserve_client.transfer(&env.current_contract_address(), &bidder, &refund);
        }

        Ok(())
    }

    // ─── View helpers ─────────────────────────────────────────────────────

    /// Return the current price per token based on the current ledger timestamp.
    pub fn price(env: Env) -> Result<i128, Error> {
        let start_price: i128 = env.storage().instance().get(&DataKey::StartPrice).ok_or(Error::NotInitialized)?;
        let reserve_price: i128 = env.storage().instance().get(&DataKey::ReservePrice).unwrap();
        let start_time: u64 = env.storage().instance().get(&DataKey::StartTime).unwrap();
        let end_time: u64 = env.storage().instance().get(&DataKey::EndTime).unwrap();
        let current_time = env.ledger().timestamp();
        Ok(curve::current_price(start_price, reserve_price, start_time, end_time, current_time))
    }

    /// Return the current auction status.
    pub fn status(env: Env) -> Result<AuctionStatus, Error> {
        env.storage().instance().get(&DataKey::Status).ok_or(Error::NotInitialized)
    }

    /// Return the locked-in clearing price (only valid after settlement).
    pub fn clearing_price(env: Env) -> Result<i128, Error> {
        let status: AuctionStatus = env.storage().instance().get(&DataKey::Status).ok_or(Error::NotInitialized)?;
        if status != AuctionStatus::Settled {
            return Err(Error::AuctionNotSettled);
        }
        Ok(env.storage().instance().get(&DataKey::ClearingPrice).unwrap())
    }

    /// Return the total reserve-token deposit of `user`.
    pub fn user_deposit(env: Env, user: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::UserDeposit(user))
            .unwrap_or(0)
    }

    /// Return whether `user` has already claimed.
    pub fn user_claimed(env: Env, user: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::UserClaimed(user))
            .unwrap_or(false)
    }

    /// Return the number of tokens remaining (not yet committed).
    pub fn tokens_remaining(env: Env) -> Result<i128, Error> {
        env.storage()
            .instance()
            .get(&DataKey::TokensRemaining)
            .ok_or(Error::NotInitialized)
    }

    // ─── Internal helpers ─────────────────────────────────────────────────

    fn require_admin(env: &Env, caller: &Address) -> Result<(), Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        if *caller != admin {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }

    fn _settle_internal(env: &Env, clearing_price: i128) -> Result<(), Error> {
        env.storage().instance().set(&DataKey::Status, &AuctionStatus::Settled);
        env.storage().instance().set(&DataKey::ClearingPrice, &clearing_price);
        Ok(())
    }
}

// Tests live in curve.rs (inline #[cfg(test)] module) and test.rs.
// The test.rs module contains pure-logic unit tests that validate the pricing
// curve and settlement mathematics without requiring the Soroban mock environment,
// matching the project-wide convention.
#[cfg(test)]
mod test;
