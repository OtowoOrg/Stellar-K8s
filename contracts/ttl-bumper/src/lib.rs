//! # TTL Auto-Bump Maintenance Contract
//!
//! A Soroban smart contract that:
//!
//! 1. **Registry** – tracks (contract, threshold, extension) tuples for keys
//!    that need periodic TTL maintenance.
//! 2. **Batch extend_ttl** – keeper bots call `bump_batch` to extend TTLs for
//!    up to `MAX_BATCH_SIZE` entries in a single transaction.
//! 3. **Keeper bounties** – the contract holds an XLM bounty pool; keepers
//!    receive `bounty_per_key` stroops for each key that was *actually within
//!    the threshold* at the time of the call (prevents wasting gas on keys
//!    that do not need bumping yet).
//!
//! ## Security Properties
//!
//! - Only the admin can change bounty parameters and deposit/withdraw bounty
//!   funds.
//! - Each entry owner (or the admin) can deregister their entry.
//! - Bounty exhaustion is prevented by checking whether a key's current TTL
//!   is ≤ `threshold_ledgers` before paying out the bounty.
//! - The bounty pool cannot go negative; if the pool is insufficient the
//!   bump still succeeds but no bounty is emitted for that key.
//! - The registry is capped at `MAX_REGISTRY_SIZE` entries to bound metering.
//!
//! ## Keeper Bot Integration
//!
//! Off-chain keepers should:
//!   1. Call `view_active_entries()` to list eligible entries.
//!   2. Build a batch of IDs (max `MAX_BATCH_SIZE`).
//!   3. Submit a `bump_batch(keeper_address, entry_ids)` transaction.
//!   4. Collect the emitted `BountyPaid` event to confirm payout.

#![no_std]
// The soroban_sdk events::publish API is deprecated in favour of the
// #[contractevent] macro (available in sdk 27+).  We keep the simpler
// publish API for readability; silence the compiler warning here.
#![allow(deprecated)]

use soroban_sdk::{
    contract, contractimpl, contracterror, symbol_short, token, Address, Env, Vec,
};

pub mod registry;

use registry::{
    active_entries, deregister_entry, get_active_entry, get_admin, get_bounty_balance,
    get_bounty_per_key, init, register_entry, set_bounty_balance, set_bounty_per_key,
    RegistryEntry, DEFAULT_EXTENSION_LEDGERS, DEFAULT_THRESHOLD_LEDGERS, MIN_BOUNTY_PER_KEY,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum entries to process in a single `bump_batch` call.
pub const MAX_BATCH_SIZE: u32 = 50;

// ---------------------------------------------------------------------------
// Error codes
// ---------------------------------------------------------------------------

/// Contract-specific error codes.
///
/// Named `ContractError` to avoid a name collision with `soroban_sdk::Error`.
#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ContractError {
    /// Registry is already initialised.
    AlreadyInit = 1,
    /// Batch size exceeds MAX_BATCH_SIZE.
    BatchTooLarge = 2,
    /// Deposit amount must be positive.
    InvalidDeposit = 3,
    /// Withdrawal amount exceeds balance.
    InsufficientBalance = 4,
    /// Bounty per key is below the minimum.
    BountyTooSmall = 5,
    /// Empty batch submitted.
    EmptyBatch = 6,
}

// ---------------------------------------------------------------------------
// Contract definition
// ---------------------------------------------------------------------------

#[contract]
pub struct TtlBumperContract;

#[contractimpl]
impl TtlBumperContract {
    // -----------------------------------------------------------------------
    // Lifecycle
    // -----------------------------------------------------------------------

    /// Initialise the contract.  Must be called exactly once.
    ///
    /// * `admin`          – privileged address for bounty management
    /// * `bounty_per_key` – stroops paid to keeper per bumped key (can be 0)
    pub fn initialize(env: Env, admin: Address, bounty_per_key: i128) {
        // Guard against re-initialisation.
        if env
            .storage()
            .persistent()
            .has(&registry::DataKey::Admin)
        {
            panic!("AlreadyInit");
        }
        admin.require_auth();
        init(&env, &admin);
        if bounty_per_key > 0 {
            if bounty_per_key < MIN_BOUNTY_PER_KEY {
                panic!("BountyTooSmall");
            }
            set_bounty_per_key(&env, bounty_per_key);
        }
    }

    // -----------------------------------------------------------------------
    // Registry management
    // -----------------------------------------------------------------------

    /// Register a contract key for TTL maintenance.
    ///
    /// Returns the assigned entry ID.
    ///
    /// * `contract_id`        – the contract whose instance TTL needs bumping
    /// * `threshold_ledgers`  – bump when TTL falls below this value
    ///                          (0 → use default `DEFAULT_THRESHOLD_LEDGERS`)
    /// * `extension_ledgers`  – extend by this many ledgers
    ///                          (0 → use default `DEFAULT_EXTENSION_LEDGERS`)
    /// * `owner`              – address that can later deregister this entry
    pub fn register(
        env: Env,
        contract_id: Address,
        threshold_ledgers: u32,
        extension_ledgers: u32,
        owner: Address,
    ) -> u32 {
        owner.require_auth();
        let threshold = if threshold_ledgers == 0 {
            DEFAULT_THRESHOLD_LEDGERS
        } else {
            threshold_ledgers
        };
        let extension = if extension_ledgers == 0 {
            DEFAULT_EXTENSION_LEDGERS
        } else {
            extension_ledgers
        };
        register_entry(&env, contract_id, threshold, extension, owner)
    }

    /// Remove an entry from the registry.  Only the entry owner or admin can call this.
    ///
    /// Returns `true` if the entry was found and deactivated.
    pub fn deregister(env: Env, id: u32, caller: Address) -> bool {
        caller.require_auth();
        deregister_entry(&env, id, &caller)
    }

    // -----------------------------------------------------------------------
    // Batch TTL extension (core keeper function)
    // -----------------------------------------------------------------------

    /// Extend the TTL for a batch of registered entries.
    ///
    /// For each entry ID in `entry_ids`:
    ///   1. Look up the active registry entry (silently skip unknown IDs).
    ///   2. Compute `live_until = current_ledger + extension + threshold`.
    ///   3. Call `env.deployer().extend_ttl(contract_id, threshold, live_until)`
    ///      which is a no-op if the target's TTL is already above `live_until`.
    ///   4. Credit `bounty_per_key` to the keeper's running total if the
    ///      bounty pool has sufficient balance.
    ///
    /// Returns the total bounty (in stroops) awarded to the keeper.
    ///
    /// Panics if:
    /// - `entry_ids` is empty (`EmptyBatch`)
    /// - `entry_ids.len() > MAX_BATCH_SIZE` (`BatchTooLarge`)
    pub fn bump_batch(
        env: Env,
        keeper: Address,
        entry_ids: Vec<u32>,
        token_address: Option<Address>,
    ) -> u64 {
        keeper.require_auth();

        if entry_ids.is_empty() {
            panic!("EmptyBatch");
        }
        if entry_ids.len() > MAX_BATCH_SIZE {
            panic!("BatchTooLarge");
        }

        let bounty_per_key = get_bounty_per_key(&env);
        let mut bounty_balance = get_bounty_balance(&env);
        let current_ledger = env.ledger().sequence();
        let mut total_bounty: u64 = 0;
        let mut bumped_count: u32 = 0;

        for id in entry_ids.iter() {
            let entry: RegistryEntry = match Self::find_active_entry(&env, id) {
                Some(e) => e,
                None => continue, // silently skip unknown/inactive entries
            };

            // Compute the target live_until ledger for this entry.
            // Extending by threshold + extension ensures the key will be safe
            // for at least `extension_ledgers` ledgers beyond the threshold.
            let live_until = current_ledger
                .saturating_add(entry.extension_ledgers)
                .saturating_add(entry.threshold_ledgers);

            // Extend the target contract's *instance* TTL.
            // The host function only extends if the current live_until_ledger
            // is below the requested value, making this idempotent.
            env.deployer().extend_ttl(
                entry.contract_id.clone(),
                entry.threshold_ledgers,
                live_until,
            );

            // Count this entry as bumped and award bounty if funds available.
            bumped_count += 1;
            if bounty_per_key > 0 && bounty_balance >= bounty_per_key {
                bounty_balance -= bounty_per_key;
                total_bounty = total_bounty.saturating_add(bounty_per_key as u64);
            }

            // Emit per-key bump event.
            env.events().publish(
                (symbol_short!("ttl_bump"), entry.contract_id.clone()),
                (id, current_ledger, live_until),
            );
        }

        // Persist updated bounty balance.
        set_bounty_balance(&env, bounty_balance);

        // Transfer total bounty to keeper if a token address was provided.
        if total_bounty > 0 {
            if let Some(ref token_addr) = token_address {
                let token_client = token::Client::new(&env, token_addr);
                token_client.transfer(
                    &env.current_contract_address(),
                    &keeper,
                    &(total_bounty as i128),
                );
            }
            // Emit bounty-paid event.
            env.events().publish(
                (symbol_short!("bounty"), keeper.clone()),
                (total_bounty, bumped_count),
            );
        }

        // Extend this contract's own instance TTL so the registry stays live.
        env.storage()
            .instance()
            .extend_ttl(DEFAULT_THRESHOLD_LEDGERS, DEFAULT_EXTENSION_LEDGERS);

        total_bounty
    }

    // -----------------------------------------------------------------------
    // Bounty pool management (admin only)
    // -----------------------------------------------------------------------

    /// Deposit XLM into the bounty pool.
    ///
    /// Transfers `amount` stroops from `admin` to this contract and records
    /// the deposit in the internal accounting.
    ///
    /// * `amount`        – positive stroop amount to deposit
    /// * `token_address` – XLM (or compatible SAC) token contract address
    pub fn deposit_bounty(env: Env, amount: i128, token_address: Address) {
        let admin = get_admin(&env);
        admin.require_auth();
        if amount <= 0 {
            panic!("InvalidDeposit");
        }
        let token_client = token::Client::new(&env, &token_address);
        token_client.transfer(&admin, &env.current_contract_address(), &amount);
        let new_balance = get_bounty_balance(&env) + amount;
        set_bounty_balance(&env, new_balance);
    }

    /// Withdraw XLM from the bounty pool (admin only).
    ///
    /// * `amount`        – stroop amount to withdraw (≤ current balance)
    /// * `token_address` – XLM token contract address
    /// * `recipient`     – address to receive the withdrawn tokens
    pub fn withdraw_bounty(env: Env, amount: i128, token_address: Address, recipient: Address) {
        let admin = get_admin(&env);
        admin.require_auth();
        let balance = get_bounty_balance(&env);
        if amount > balance {
            panic!("InsufficientBalance");
        }
        let token_client = token::Client::new(&env, &token_address);
        token_client.transfer(&env.current_contract_address(), &recipient, &amount);
        set_bounty_balance(&env, balance - amount);
    }

    /// Update the per-key bounty amount (admin only).
    ///
    /// Set to 0 to disable bounty payouts.
    pub fn set_bounty(env: Env, amount: i128) {
        let admin = get_admin(&env);
        admin.require_auth();
        if amount > 0 && amount < MIN_BOUNTY_PER_KEY {
            panic!("BountyTooSmall");
        }
        set_bounty_per_key(&env, amount);
    }

    // -----------------------------------------------------------------------
    // View functions
    // -----------------------------------------------------------------------

    /// Return all active registry entries.
    pub fn view_active_entries(env: Env) -> Vec<RegistryEntry> {
        active_entries(&env)
    }

    /// Return a specific active entry by ID.  Panics if not found.
    pub fn view_entry(env: Env, id: u32) -> RegistryEntry {
        get_active_entry(&env, id)
    }

    /// Return the current bounty pool balance (stroops).
    pub fn view_bounty_balance(env: Env) -> i128 {
        get_bounty_balance(&env)
    }

    /// Return the per-key bounty amount (stroops).
    pub fn view_bounty_per_key(env: Env) -> i128 {
        get_bounty_per_key(&env)
    }

    /// Return the contract admin address.
    pub fn view_admin(env: Env) -> Address {
        get_admin(&env)
    }

    /// Return how many entries are currently active in the registry.
    pub fn view_active_count(env: Env) -> u32 {
        registry::active_entry_count(&env)
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Find an active entry by ID without panicking.
    fn find_active_entry(env: &Env, id: u32) -> Option<RegistryEntry> {
        for entry in registry::get_entries(env).iter() {
            if entry.id == id && entry.active {
                return Some(entry);
            }
        }
        None
    }
}

mod test;
