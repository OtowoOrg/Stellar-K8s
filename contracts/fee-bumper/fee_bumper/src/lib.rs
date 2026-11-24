//! Fee-Bump Relayer Sponsorship Contract (partial implementation of
//! issue #29).
//!
//! ## What a Soroban contract can and cannot do here
//!
//! The issue asks for a contract that "validates an inner transaction
//! envelope and wraps it in a fee-bump transaction structure." A Soroban
//! contract cannot do that: contracts execute *inside* an already-built,
//! already-fee-bid transaction — there is no host function to construct a
//! `FeeBumpTransactionEnvelope`, sign it, or submit it to the network.
//! That part of a fee-bump relayer is necessarily off-chain client code
//! (e.g. `stellar-sdk`/`js-stellar-sdk` on the relayer's own server), not
//! a contract. See the crate README for the off-chain relayer flow this
//! contract is designed to be called from.
//!
//! What *can* live on-chain, and is what this contract implements, is the
//! reimbursement accounting the relayer needs around that off-chain step:
//!
//! 1. [`authorize_sponsorship`][FeeBumperContract::authorize_sponsorship] —
//!    a pre-flight check the relayer calls *before* fronting the real
//!    network fee. Rejects a request that's rate-limited, and otherwise
//!    escrows `estimated_fee` of the reimbursement token out of the
//!    account's balance immediately, atomically with the rate-limit
//!    reservation.
//! 2. [`settle_reimbursement`][FeeBumperContract::settle_reimbursement] —
//!    called by the relayer after it has observed the real fee the
//!    network actually charged, to collect exactly that much from escrow
//!    and refund the sponsored account any difference.
//! 3. [`expire_stale_sponsorship`][FeeBumperContract::expire_stale_sponsorship]
//!    — refunds an escrow the relayer never settled (e.g. it decided not
//!    to broadcast after all) back to the account once it has expired.
//!
//! Escrowing at authorization time (rather than only checking a token
//! `allowance`) is a deliberate front-running fix — see "Security
//! analysis" in the README.

#![no_std]

mod relayer;

pub use relayer::{AccountWindow, RateLimitConfig};

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, token, Address, Env, MuxedAddress};

/// How long an escrowed sponsorship may sit unsettled before it can be
/// reclaimed by [`FeeBumperContract::expire_stale_sponsorship`]. Chosen
/// generously relative to how long a real Stellar transaction stays
/// eligible for submission (its `timeBounds`, typically on the order of a
/// couple of minutes), so a relayer that's simply slow to observe the
/// network result and call `settle_reimbursement` has ample time before
/// anyone can force a refund out from under it.
pub const PENDING_SPONSORSHIP_TTL_SECONDS: u64 = 600;

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    ReimbursementToken,
    RateLimitConfig,
    AccountWindow(Address),
    Pending(Address),
}

/// An escrowed, not-yet-settled sponsorship.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingSponsorship {
    /// The amount already pulled from the account into this contract's
    /// custody. This is the ceiling `settle_reimbursement` may collect,
    /// and the amount `expire_stale_sponsorship` refunds in full.
    pub escrowed_amount: i128,
    pub authorized_at: u64,
    pub expires_at: u64,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum FeeBumpError {
    NotInitialized = 1,
    InvalidRateLimitConfig = 2,
    InvalidFeeAmount = 3,
    RateLimited = 4,
    SponsorshipAlreadyPending = 5,
    NoPendingSponsorship = 6,
    SponsorshipExpired = 7,
    SponsorshipNotYetExpired = 8,
    ActualFeeExceedsEscrow = 9,
}

#[contract]
pub struct FeeBumperContract;

#[contractimpl]
impl FeeBumperContract {
    /// Runs atomically as part of contract creation, so — unlike a plain
    /// callable `initialize` method — it cannot be front-run by a
    /// separate transaction racing to claim the admin role (see the
    /// front-running note left in `contracts/proxy-controller`'s README,
    /// which this contract takes as a lesson rather than repeating the
    /// mistake).
    pub fn __constructor(
        env: Env,
        admin: Address,
        reimbursement_token: Address,
        max_per_window: u32,
        window_seconds: u64,
    ) -> Result<(), FeeBumpError> {
        let config = RateLimitConfig { max_per_window, window_seconds };
        if !config.is_valid() {
            return Err(FeeBumpError::InvalidRateLimitConfig);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::ReimbursementToken, &reimbursement_token);
        env.storage().instance().set(&DataKey::RateLimitConfig, &config);
        Ok(())
    }

    pub fn admin(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::Admin)
    }

    pub fn reimbursement_token(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::ReimbursementToken)
    }

    pub fn rate_limit_config(env: Env) -> Option<RateLimitConfig> {
        env.storage().instance().get(&DataKey::RateLimitConfig)
    }

    pub fn account_window(env: Env, account: Address) -> Option<AccountWindow> {
        env.storage().persistent().get(&DataKey::AccountWindow(account))
    }

    pub fn pending_sponsorship(env: Env, account: Address) -> Option<PendingSponsorship> {
        env.storage().persistent().get(&DataKey::Pending(account))
    }

    /// Pre-flight check + escrow. `account` must have already `approve`d
    /// this contract (SEP-41) for at least `estimated_fee` of the
    /// reimbursement token. Returns the escrow's expiry timestamp on
    /// success.
    pub fn authorize_sponsorship(env: Env, account: Address, estimated_fee: i128) -> Result<u64, FeeBumpError> {
        account.require_auth();

        if estimated_fee <= 0 {
            return Err(FeeBumpError::InvalidFeeAmount);
        }

        let token_address: Address = env
            .storage()
            .instance()
            .get(&DataKey::ReimbursementToken)
            .ok_or(FeeBumpError::NotInitialized)?;
        let config: RateLimitConfig = env
            .storage()
            .instance()
            .get(&DataKey::RateLimitConfig)
            .ok_or(FeeBumpError::NotInitialized)?;

        let pending_key = DataKey::Pending(account.clone());
        let now = env.ledger().timestamp();
        if let Some(existing) = env.storage().persistent().get::<_, PendingSponsorship>(&pending_key) {
            if now < existing.expires_at {
                return Err(FeeBumpError::SponsorshipAlreadyPending);
            }
            // A previous escrow expired without being settled or
            // explicitly reclaimed. Refund it now rather than silently
            // abandoning it or letting it be overwritten, so the account
            // is never out the escrowed amount just because the relayer
            // never called `expire_stale_sponsorship` itself.
            let token_client = token::TokenClient::new(&env, &token_address);
            token_client.transfer(
                &env.current_contract_address(),
                &MuxedAddress::from(account.clone()),
                &existing.escrowed_amount,
            );
            env.storage().persistent().remove(&pending_key);
        }

        let window_key = DataKey::AccountWindow(account.clone());
        let previous_window = env.storage().persistent().get(&window_key);
        let updated_window =
            relayer::check_and_advance(&config, previous_window, now).ok_or(FeeBumpError::RateLimited)?;

        // Escrow now, in the same call as the rate-limit reservation —
        // see the README's front-running analysis for why this must
        // happen here rather than only checking `allowance`.
        let token_client = token::TokenClient::new(&env, &token_address);
        token_client.transfer_from(
            &env.current_contract_address(),
            &account,
            &env.current_contract_address(),
            &estimated_fee,
        );

        env.storage().persistent().set(&window_key, &updated_window);
        let expires_at = now + PENDING_SPONSORSHIP_TTL_SECONDS;
        env.storage().persistent().set(
            &pending_key,
            &PendingSponsorship {
                escrowed_amount: estimated_fee,
                authorized_at: now,
                expires_at,
            },
        );

        Ok(expires_at)
    }

    /// Called by the relayer (`admin`) once it knows the real fee the
    /// network charged for the broadcast fee-bump transaction. Collects
    /// exactly `actual_fee` from escrow and refunds the rest — it can
    /// never collect more than what was escrowed at authorization time,
    /// which is the safeguard against a spike in the live network fee
    /// ("maximum network fee turbulence") being passed on to the
    /// sponsored account beyond what it approved.
    pub fn settle_reimbursement(env: Env, account: Address, actual_fee: i128) -> Result<(), FeeBumpError> {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).ok_or(FeeBumpError::NotInitialized)?;
        admin.require_auth();

        if actual_fee <= 0 {
            return Err(FeeBumpError::InvalidFeeAmount);
        }

        let pending_key = DataKey::Pending(account.clone());
        let pending: PendingSponsorship = env
            .storage()
            .persistent()
            .get(&pending_key)
            .ok_or(FeeBumpError::NoPendingSponsorship)?;

        // Validate *before* touching storage or moving any funds. A
        // rejected settlement (expired, or asking for more than was
        // escrowed) must leave the pending entry exactly as it was, so
        // the escrowed funds stay recoverable — either via a corrected
        // retry of this same call, or via `expire_stale_sponsorship` once
        // it passes `expires_at`. Deleting it here on an error path would
        // stick the escrowed funds in this contract's balance with no
        // remaining function able to reach them.
        if env.ledger().timestamp() >= pending.expires_at {
            return Err(FeeBumpError::SponsorshipExpired);
        }
        if actual_fee > pending.escrowed_amount {
            return Err(FeeBumpError::ActualFeeExceedsEscrow);
        }

        env.storage().persistent().remove(&pending_key);

        let token_address: Address = env
            .storage()
            .instance()
            .get(&DataKey::ReimbursementToken)
            .ok_or(FeeBumpError::NotInitialized)?;
        let token_client = token::TokenClient::new(&env, &token_address);

        token_client.transfer(&env.current_contract_address(), &MuxedAddress::from(admin), &actual_fee);

        let refund = pending.escrowed_amount - actual_fee;
        if refund > 0 {
            token_client.transfer(&env.current_contract_address(), &MuxedAddress::from(account), &refund);
        }

        Ok(())
    }

    /// Refunds an escrow the relayer never settled. Callable by anyone —
    /// it only ever moves funds already in this contract's custody back
    /// to the account they came from, and only once the escrow has
    /// actually expired, so there's nothing to authorize.
    pub fn expire_stale_sponsorship(env: Env, account: Address) -> Result<(), FeeBumpError> {
        let pending_key = DataKey::Pending(account.clone());
        let pending: PendingSponsorship = env
            .storage()
            .persistent()
            .get(&pending_key)
            .ok_or(FeeBumpError::NoPendingSponsorship)?;

        if env.ledger().timestamp() < pending.expires_at {
            return Err(FeeBumpError::SponsorshipNotYetExpired);
        }

        env.storage().persistent().remove(&pending_key);

        let token_address: Address = env
            .storage()
            .instance()
            .get(&DataKey::ReimbursementToken)
            .ok_or(FeeBumpError::NotInitialized)?;
        token::TokenClient::new(&env, &token_address).transfer(
            &env.current_contract_address(),
            &MuxedAddress::from(account),
            &pending.escrowed_amount,
        );

        Ok(())
    }
}
