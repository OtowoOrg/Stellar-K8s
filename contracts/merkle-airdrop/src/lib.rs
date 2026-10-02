#![no_std]

//! # Merkle Airdrop Claim Distributor
//!
//! A pull-based token distribution contract. The full list of `(address, allocation)`
//! pairs is compiled off-chain into a single 32-byte Merkle root, and each recipient
//! proves their inclusion in that tree to pull their allocation exactly once.
//!
//! This is what makes an airdrop to hundreds of thousands of accounts economically
//! viable: on-chain state and cost stay `O(1)` in the number of recipients instead of
//! `O(n)`, and the distributor never has to submit a transaction per recipient.
//!
//! ## Security model
//!
//! * **One claim per allocation, permanently.** The claim flag is a bit in a
//!   persistent bitmap keyed by *leaf index*, and the index is authenticated by the
//!   Merkle proof (it is hashed into the leaf). The flag is written *before* the token
//!   transfer, so a reentrant or repeated call cannot double-spend.
//! * **Exact payout.** The amount is also hashed into the leaf, so the contract pays
//!   precisely what the distribution committed to — a claimant cannot inflate it.
//! * **Fail closed.** The committed root is frozen after the first claim; a root swap
//!   could otherwise silently change who owns each unclaimed slot.
//! * **Bounded work.** Proof depth is capped ([`claim::MAX_PROOF_DEPTH`]), so the
//!   worst-case cost of a claim is known before the campaign launches.
//!
//! See [`claim`] for the byte-exact tree format and the reasoning behind each choice,
//! and `README.md` for the off-chain/reporting side.

// The guest build must stay `no_std`; tests need `std` for the fixtures that build a
// 100 000-recipient tree. Matches the convention used by the other contracts here.
#[cfg(any(test, feature = "testutils"))]
extern crate std;

pub mod claim;

#[cfg(test)]
mod test;

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype,
    token::Client as TokenClient, Address, BytesN, Env, Vec,
};

/// Width of a SHA-256 digest.
///
/// The `BytesN<32>` spellings below use the literal because `#[contracttype]` accepts
/// nothing else in a const-generic position; this constant is asserted against
/// [`claim::DIGEST_LEN`] at compile time so the two can never drift.
pub const DIGEST_BYTES: usize = 32;

const _: () = assert!(DIGEST_BYTES == claim::DIGEST_LEN);

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

// Published as typed `#[contractevent]` types rather than the older
// `env.events().publish(..)` tuple form (which is deprecated, and costs an extra
// diagnostic pass under `-D warnings`). `data_format = "vec"` keeps the payload a
// plain vector — the map format would store a symbol key alongside every field.

/// A distribution was created. `admin` is a topic so operators can filter by it.
#[contractevent(topics = ["init"], data_format = "vec")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DistributionInitialized {
    #[topic]
    pub admin: Address,
    pub token: Address,
    pub merkle_root: BytesN<32>,
    pub leaf_count: u32,
    pub total_amount: i128,
    pub claim_deadline: u64,
}

/// The committed root was replaced before the first claim.
#[contractevent(topics = ["root"], data_format = "vec")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RootUpdated {
    pub merkle_root: BytesN<32>,
    pub leaf_count: u32,
    pub total_amount: i128,
}

/// An allocation was paid out. `claimant` is a topic so an indexer can answer
/// "how much did this address receive?" without scanning every event.
#[contractevent(topics = ["claim"], data_format = "vec")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Claimed {
    #[topic]
    pub claimant: Address,
    pub index: u32,
    pub amount: i128,
}

/// Claims were halted or resumed.
#[contractevent(topics = ["paused"], data_format = "vec")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PauseChanged {
    pub is_paused: bool,
}

/// Unclaimed tokens were swept after the claim window closed.
#[contractevent(topics = ["clawback"], data_format = "vec")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClawedBack {
    pub recipient: Address,
    pub amount: i128,
}

/// Every way a call can fail.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    ContractPaused = 4,
    /// The root is the all-zero padding digest, which cannot be a real tree root.
    InvalidMerkleRoot = 5,
    /// Zero allocations, or more than [`claim::MAX_LEAF_COUNT`].
    InvalidLeafCount = 6,
    InvalidAmount = 7,
    /// `index` is past the end of the declared allocation range.
    IndexOutOfRange = 8,
    /// The proof is deeper than [`claim::MAX_PROOF_DEPTH`].
    ProofTooDeep = 9,
    /// The proof does not fold to the committed root for this `(index, amount, claimant)`.
    InvalidProof = 10,
    /// This allocation's bit is already set in the claim bitmap.
    AlreadyClaimed = 11,
    /// `set_merkle_root` after the first claim has landed.
    RootFinalized = 12,
    /// The distribution's claim window has closed.
    ClaimWindowClosed = 13,
    /// `clawback` was called before the claim window closed.
    ClaimWindowStillOpen = 14,
    /// `clawback` was called on a distribution that has no deadline, and so never expires.
    DeadlineNotSet = 15,
    /// Nothing left to sweep out of the distributor.
    EmptyAirdropBalance = 16,
    CalculationOverflow = 17,
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

/// Claim-bitmap TTL is extended once it drops below roughly 30 days of ledger closes at
/// the 5-second mainnet target.
pub const CLAIM_TTL_THRESHOLD: u32 = 17280 * 30;

/// ...and topped back up to roughly 90 days, so an in-flight campaign never makes a
/// claimant pay for a footprint restore.
pub const CLAIM_TTL_EXTEND_TO: u32 = 17280 * 90;

/// Storage keys.
///
/// Configuration lives under a single key rather than a key per field. The host charges one
/// lookup per `get` and, as it turns out, neither a tuple of keys nor a `Vec` of keys can be
/// read in one call (`instance().get(&(k1, k2, ..))` compiles but returns `None`), so
/// bundling the immutable configuration into one struct is the only way to remove those host
/// calls — six of them, on every claim. The mutable counters are grouped the same way, for
/// the same reason.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Config,
    Progress,
    /// Claim bitmap bucket holding the flags for 128 consecutive leaf indices.
    ClaimedBucket(u32),
}

/// Immutable distribution configuration.
///
/// Field order is part of the stored encoding; append new fields at the end.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    /// Address allowed to pause, rotate the root before the first claim, and claw back.
    pub admin: Address,
    /// Token distributed to claimants.
    pub token: Address,
    /// Commitment to the whole distribution.
    pub merkle_root: BytesN<32>,
    /// Number of real allocations in the tree (padding excluded).
    pub leaf_count: u32,
    /// Sum of the allocations the tree commits to. Informational: the hard cap on
    /// total outflow is the distributor's token balance.
    pub total_allocated: i128,
    /// Unix timestamp after which claims stop and `clawback` becomes available.
    /// Zero means the distribution never expires (and cannot be clawed back).
    pub claim_deadline: u64,
}

/// Mutable distribution counters.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Progress {
    /// Total tokens paid out so far.
    pub total_claimed: i128,
    /// Number of successful claims. Also the "has anything been claimed yet?" flag
    /// that freezes the root.
    pub claim_count: u32,
    /// Emergency stop for new claims.
    pub is_paused: bool,
}

#[contract]
pub struct MerkleAirdropContract;

#[contractimpl]
impl MerkleAirdropContract {
    /// Create a distribution.
    ///
    /// `merkle_root` is the root of the off-chain tree built with the format in
    /// [`claim`]; `leaf_count` is the number of *real* allocations in it (the tree is
    /// padded to a power of two internally); `total_amount` is the sum those
    /// allocations add up to. `claim_deadline` of `0` means the distribution never
    /// expires.
    ///
    /// The contract does not require the distributor to be funded here — funding it is
    /// a separate token transfer, which keeps `initialize` free of a token call and
    /// lets a campaign be staged before the treasury moves.
    pub fn initialize(
        env: Env,
        admin: Address,
        token: Address,
        merkle_root: BytesN<32>,
        leaf_count: u32,
        total_amount: i128,
        claim_deadline: u64,
    ) -> Result<(), Error> {
        let storage = env.storage().instance();
        if storage.has(&DataKey::Config) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();

        validate_merkle_root(&merkle_root)?;
        validate_leaf_count(leaf_count)?;
        if total_amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let config = Config {
            admin: admin.clone(),
            token: token.clone(),
            merkle_root: merkle_root.clone(),
            leaf_count,
            total_allocated: total_amount,
            claim_deadline,
        };
        storage.set(&DataKey::Config, &config);
        storage.set(
            &DataKey::Progress,
            &Progress {
                total_claimed: 0,
                claim_count: 0,
                is_paused: false,
            },
        );

        DistributionInitialized {
            admin,
            token,
            merkle_root,
            leaf_count,
            total_amount,
            claim_deadline,
        }
        .publish(&env);
        Ok(())
    }

    /// Replace the committed root before anyone has claimed.
    ///
    /// Airdrop generators get things wrong — an address is omitted, a decimal is
    /// misplaced — and the whole point of publishing a root is that the fix is a single
    /// transaction rather than a redeploy. The window for that fix closes at the first
    /// claim: after that, rotating the root would reassign slots whose flags are already
    /// burned, silently stranding allocations that the earlier root had granted. Once a
    /// distribution is live its contents are immutable, and a root that turns out to be
    /// wrong afterwards is remedied by `clawback` plus a fresh distributor.
    pub fn set_merkle_root(
        env: Env,
        caller: Address,
        merkle_root: BytesN<32>,
        leaf_count: u32,
        total_amount: i128,
    ) -> Result<(), Error> {
        require_admin(&env, &caller)?;
        validate_merkle_root(&merkle_root)?;
        validate_leaf_count(leaf_count)?;
        if total_amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let mut config: Config = env
            .storage()
            .instance()
            .get(&DataKey::Config)
            .ok_or(Error::NotInitialized)?;
        let progress: Progress =
            env.storage()
                .instance()
                .get(&DataKey::Progress)
                .unwrap_or(Progress {
                    total_claimed: 0,
                    claim_count: 0,
                    is_paused: false,
                });
        if progress.claim_count != 0 {
            return Err(Error::RootFinalized);
        }

        config.merkle_root = merkle_root.clone();
        config.leaf_count = leaf_count;
        config.total_allocated = total_amount;
        env.storage().instance().set(&DataKey::Config, &config);

        RootUpdated {
            merkle_root,
            leaf_count,
            total_amount,
        }
        .publish(&env);
        Ok(())
    }

    /// Claim the allocation at `index` for `claimant`, proving it with `proof`.
    ///
    /// Checks are ordered cheapest-first on purpose. The replay guard runs *before* the
    /// proof is verified, so a duplicate claim costs the network a bitmap read rather
    /// than `depth + 1` hashes — an attacker cannot make the distributor do expensive
    /// work by replaying their own claim. Everything that mutates state then happens
    /// before the token transfer, so there is no window in which a call could re-enter
    /// and claim the same slot twice.
    pub fn claim(
        env: Env,
        claimant: Address,
        index: u32,
        amount: i128,
        proof: Vec<BytesN<32>>,
    ) -> Result<(), Error> {
        let storage = env.storage().instance();

        let config: Config = storage.get(&DataKey::Config).ok_or(Error::NotInitialized)?;
        let progress: Progress = storage.get(&DataKey::Progress).unwrap_or(Progress {
            total_claimed: 0,
            claim_count: 0,
            is_paused: false,
        });

        if progress.is_paused {
            return Err(Error::ContractPaused);
        }
        if config.claim_deadline != 0 && env.ledger().timestamp() >= config.claim_deadline {
            return Err(Error::ClaimWindowClosed);
        }
        claimant.require_auth();

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        if index >= config.leaf_count {
            return Err(Error::IndexOutOfRange);
        }
        if proof.len() > claim::MAX_PROOF_DEPTH {
            return Err(Error::ProofTooDeep);
        }

        let bucket_key = DataKey::ClaimedBucket(claim::bucket_of(index));
        let mask = claim::mask_of(index);
        let claimed_bits: u128 = env.storage().persistent().get(&bucket_key).unwrap_or(0);
        if claimed_bits & mask != 0 {
            return Err(Error::AlreadyClaimed);
        }

        let leaf = claim::hash_leaf(&env, index, amount, &claimant);
        if !claim::verify_proof(&env, &leaf, &proof, &config.merkle_root) {
            return Err(Error::InvalidProof);
        }

        // Effects. The flag is set before the transfer so that the transfer cannot be
        // used to re-enter `claim` for the same slot.
        env.storage()
            .persistent()
            .set(&bucket_key, &(claimed_bits | mask));
        env.storage().persistent().extend_ttl(
            &bucket_key,
            CLAIM_TTL_THRESHOLD,
            CLAIM_TTL_EXTEND_TO,
        );

        storage.set(
            &DataKey::Progress,
            &Progress {
                total_claimed: progress
                    .total_claimed
                    .checked_add(amount)
                    .ok_or(Error::CalculationOverflow)?,
                // Bounded by `MAX_LEAF_COUNT`: one claim per allocation.
                claim_count: progress.claim_count + 1,
                is_paused: progress.is_paused,
            },
        );

        // Interaction.
        TokenClient::new(&env, &config.token).transfer(
            &env.current_contract_address(),
            &claimant,
            &amount,
        );

        Claimed {
            claimant,
            index,
            amount,
        }
        .publish(&env);
        Ok(())
    }

    /// Emergency stop / resume for new claims. Never blocks `clawback`.
    pub fn set_paused(env: Env, caller: Address, paused: bool) -> Result<(), Error> {
        require_admin(&env, &caller)?;

        let storage = env.storage().instance();
        let mut progress: Progress = storage.get(&DataKey::Progress).unwrap_or(Progress {
            total_claimed: 0,
            claim_count: 0,
            is_paused: false,
        });
        progress.is_paused = paused;
        storage.set(&DataKey::Progress, &progress);

        PauseChanged { is_paused: paused }.publish(&env);
        Ok(())
    }

    /// Sweep whatever the campaign did not pay out, once the claim window has closed.
    ///
    /// Only available on distributions that set a deadline, so a campaign configured
    /// without one can never be drained from under its recipients.
    pub fn clawback(env: Env, caller: Address, recipient: Address) -> Result<i128, Error> {
        require_admin(&env, &caller)?;

        let config: Config = env
            .storage()
            .instance()
            .get(&DataKey::Config)
            .ok_or(Error::NotInitialized)?;
        if config.claim_deadline == 0 {
            return Err(Error::DeadlineNotSet);
        }
        if env.ledger().timestamp() < config.claim_deadline {
            return Err(Error::ClaimWindowStillOpen);
        }

        let token_client = TokenClient::new(&env, &config.token);
        let balance = token_client.balance(&env.current_contract_address());
        if balance <= 0 {
            return Err(Error::EmptyAirdropBalance);
        }

        token_client.transfer(&env.current_contract_address(), &recipient, &balance);

        ClawedBack {
            recipient,
            amount: balance,
        }
        .publish(&env);
        Ok(balance)
    }

    // -----------------------------------------------------------------------
    // Views
    // -----------------------------------------------------------------------

    /// The committed root of the distribution.
    pub fn merkle_root(env: Env) -> Result<BytesN<32>, Error> {
        Ok(load_config(&env)?.merkle_root)
    }

    /// Number of real allocations in the distribution (padding excluded).
    pub fn leaf_count(env: Env) -> Result<u32, Error> {
        Ok(load_config(&env)?.leaf_count)
    }

    /// Length of every proof in this distribution: `ceil(log2(leaf_count))`.
    ///
    /// Lets a client size and pre-flight a claim without reimplementing the geometry.
    pub fn required_proof_depth(env: Env) -> Result<u32, Error> {
        Ok(claim::proof_depth_for(load_config(&env)?.leaf_count))
    }

    /// Sum of the allocations the tree commits to.
    pub fn total_allocated(env: Env) -> Result<i128, Error> {
        Ok(load_config(&env)?.total_allocated)
    }

    /// Timestamp after which claims stop, or `0` if the distribution never expires.
    pub fn claim_deadline(env: Env) -> Result<u64, Error> {
        Ok(load_config(&env)?.claim_deadline)
    }

    /// Address allowed to administer the distribution.
    pub fn admin(env: Env) -> Result<Address, Error> {
        Ok(load_config(&env)?.admin)
    }

    /// Token being distributed.
    pub fn token(env: Env) -> Result<Address, Error> {
        Ok(load_config(&env)?.token)
    }

    /// Tokens paid out so far and the number of successful claims.
    pub fn progress(env: Env) -> (i128, u32) {
        let progress = load_progress(&env);
        (progress.total_claimed, progress.claim_count)
    }

    /// Whether new claims are currently halted.
    pub fn is_paused(env: Env) -> bool {
        load_progress(&env).is_paused
    }

    /// Whether the allocation at `index` has already been claimed.
    pub fn is_claimed(env: Env, index: u32) -> bool {
        let bits: u128 = env
            .storage()
            .persistent()
            .get(&DataKey::ClaimedBucket(claim::bucket_of(index)))
            .unwrap_or(0);
        bits & claim::mask_of(index) != 0
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn load_config(env: &Env) -> Result<Config, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .ok_or(Error::NotInitialized)
}

fn load_progress(env: &Env) -> Progress {
    env.storage()
        .instance()
        .get(&DataKey::Progress)
        .unwrap_or(Progress {
            total_claimed: 0,
            claim_count: 0,
            is_paused: false,
        })
}

fn require_admin(env: &Env, caller: &Address) -> Result<(), Error> {
    let config = load_config(env)?;
    if caller != &config.admin {
        return Err(Error::Unauthorized);
    }
    caller.require_auth();
    Ok(())
}

/// The all-zero digest is reserved for padding, so it can never be a real root — a
/// zero root would mean every padded slot verifies.
fn validate_merkle_root(root: &BytesN<32>) -> Result<(), Error> {
    if root.to_array().as_slice() == claim::PAD_LEAF.as_slice() {
        return Err(Error::InvalidMerkleRoot);
    }
    Ok(())
}

/// A tree must hold at least one allocation and fit inside the supported depth.
fn validate_leaf_count(leaf_count: u32) -> Result<(), Error> {
    if leaf_count == 0 || leaf_count > claim::MAX_LEAF_COUNT {
        return Err(Error::InvalidLeafCount);
    }
    Ok(())
}
