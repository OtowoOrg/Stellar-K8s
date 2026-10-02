//! # Decentralized WebAssembly Upgrade Proxy
//!
//! A Soroban smart contract that provides a standardised, secure, and
//! transparent framework for managing the lifecycle of upgradeable on-chain
//! applications on the Stellar network.
//!
//! ## Overview
//!
//! Upgrading a Soroban contract requires replacing the WASM hash installed at
//! its address via `env.deployer().update_current_contract_wasm`.  This proxy
//! contract wraps that call behind a strict governance layer:
//!
//! 1. **Propose** — the admin submits a new WASM binary.  It is uploaded to
//!    the ledger and a 7-day (604 800 s) countdown starts.
//! 2. **Inspect** — during the 7-day window the community may examine the
//!    incoming binary before it becomes active.
//! 3. **Abort** — if a vulnerability is discovered, the admin *or* the DAO
//!    council can call `abort_upgrade` to cancel the pending proposal.
//! 4. **Execute** — once the window has elapsed the admin calls
//!    `execute_upgrade` to atomically swap the on-chain bytecode.
//!
//! ## Design notes
//!
//! Soroban has no delegatecall-style proxy: a contract can only replace its
//! own installed WASM.  This contract therefore acts as *the* upgradeable
//! contract address — callers interact with it directly, and the upgrade
//! governance state (admin, pending proposal) is stored in the same persistent
//! storage that survives every WASM swap.
//!
//! Storage keys are defined in [`storage`] and all carry an `UpgradeProxy`
//! prefix so they cannot collide with keys used by future implementation
//! versions.
//!
//! ## Storage layout
//!
//! | Key                      | Type              | Lifecycle   |
//! |--------------------------|-------------------|-------------|
//! | `UpgradeProxyAdmin`      | `Address`         | Persistent  |
//! | `UpgradeProxyDaoCouncil` | `Address`         | Persistent  |
//! | `UpgradeProxyPending`    | `PendingUpgrade`  | Persistent  |
//!
//! ## Security review checklist
//!
//! * Only `admin` may call `propose_upgrade` — checked via `require_auth`.
//! * Only `admin` *or* `dao_council` may call `abort_upgrade` — both checked.
//! * Only `admin` may call `execute_upgrade` — checked via `require_auth`.
//! * `execute_upgrade` panics if the 7-day timelock has not elapsed.
//! * At most one pending upgrade exists at a time; concurrent proposals are
//!   rejected with `UpgradeAlreadyPending`.
//! * The timelock uses `env.ledger().timestamp()` (Unix seconds) rather than
//!   ledger sequence numbers, making it robust against ledger-velocity changes.

#![no_std]

pub mod error;
pub mod storage;
pub mod timelock;

use error::ProxyError;
use storage::DataKey;
use timelock::{PendingUpgrade, assert_elapsed, execute_after_timestamp};

use soroban_sdk::{
    contract, contractimpl, symbol_short, Address, Bytes, BytesN, Env,
};

// ---------------------------------------------------------------------------
// Contract declaration
// ---------------------------------------------------------------------------

/// The Upgrade Proxy contract struct.  All public functions are attached via
/// `#[contractimpl]`.
#[contract]
pub struct UpgradeProxyContract;

// ---------------------------------------------------------------------------
// Public interface
// ---------------------------------------------------------------------------

#[contractimpl]
impl UpgradeProxyContract {
    // -----------------------------------------------------------------------
    // Initialisation (call exactly once from `__constructor` or deploy tx)
    // -----------------------------------------------------------------------

    /// Initialise the proxy governance state.
    ///
    /// Must be called exactly once, ideally atomically at deploy time (from a
    /// `__constructor` in the deploying contract) to prevent front-running.
    ///
    /// * `admin`       — privileged address that may propose and execute
    ///                   upgrades.  Typically a DAO multi-sig.
    /// * `dao_council` — emergency address that may abort a pending upgrade
    ///                   in addition to the admin.  Should use a different
    ///                   key-set from `admin`.
    pub fn initialize(
        env: Env,
        admin: Address,
        dao_council: Address,
    ) -> Result<(), ProxyError> {
        if env
            .storage()
            .persistent()
            .has(&DataKey::UpgradeProxyAdmin)
        {
            return Err(ProxyError::AlreadyInitialized);
        }

        admin.require_auth();

        env.storage()
            .persistent()
            .set(&DataKey::UpgradeProxyAdmin, &admin);
        env.storage()
            .persistent()
            .set(&DataKey::UpgradeProxyDaoCouncil, &dao_council);

        env.events().publish(
            (symbol_short!("init"),),
            (admin, dao_council),
        );

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Upgrade lifecycle
    // -----------------------------------------------------------------------

    /// Upload new WASM bytecode and start the mandatory 7-day review window.
    ///
    /// Only the `admin` may propose an upgrade.  At most one upgrade may be
    /// pending at a time — call `abort_upgrade` first to replace an existing
    /// proposal.
    ///
    /// Returns the content hash of the uploaded WASM, which can be used to
    /// verify the binary off-chain.
    pub fn propose_upgrade(
        env: Env,
        new_wasm: Bytes,
    ) -> Result<BytesN<32>, ProxyError> {
        let admin = Self::load_admin(&env)?;
        admin.require_auth();

        // Reject concurrent proposals.
        if env
            .storage()
            .persistent()
            .has(&DataKey::UpgradeProxyPending)
        {
            return Err(ProxyError::UpgradeAlreadyPending);
        }

        // Upload the WASM binary to the ledger and obtain its content hash.
        let wasm_hash: BytesN<32> = env.deployer().upload_contract_wasm(new_wasm);

        let execute_after = execute_after_timestamp(&env)?;
        let now = env.ledger().timestamp();

        let pending = PendingUpgrade {
            wasm_hash: wasm_hash.clone(),
            proposed_at: now,
            execute_after,
        };

        env.storage()
            .persistent()
            .set(&DataKey::UpgradeProxyPending, &pending);

        // Emit an event so indexers / community tooling can surface the
        // incoming binary for inspection immediately.
        env.events().publish(
            (symbol_short!("propose"),),
            (wasm_hash.clone(), execute_after),
        );

        Ok(wasm_hash)
    }

    /// Cancel a pending upgrade before it takes effect.
    ///
    /// Both the `admin` and the `dao_council` may call this function, giving
    /// the DAO a veto right independent of the admin key.  This is the
    /// primary mechanism for responding to critical vulnerabilities discovered
    /// during the 7-day review window.
    ///
    /// # Errors
    ///
    /// * `NotInitialized`   — contract has not been initialised.
    /// * `Unauthorized`     — `caller` is neither admin nor DAO council.
    /// * `NoPendingUpgrade` — no upgrade is currently queued.
    pub fn abort_upgrade(env: Env, caller: Address) -> Result<(), ProxyError> {
        caller.require_auth();

        let admin = Self::load_admin(&env)?;
        let council = Self::load_council(&env)?;

        if caller != admin && caller != council {
            return Err(ProxyError::Unauthorized);
        }

        if !env
            .storage()
            .persistent()
            .has(&DataKey::UpgradeProxyPending)
        {
            return Err(ProxyError::NoPendingUpgrade);
        }

        // Retrieve the pending upgrade for the event payload before removing.
        let pending: PendingUpgrade = env
            .storage()
            .persistent()
            .get(&DataKey::UpgradeProxyPending)
            .unwrap(); // safe: `has` returned true above

        env.storage()
            .persistent()
            .remove(&DataKey::UpgradeProxyPending);

        env.events().publish(
            (symbol_short!("abort"),),
            (caller, pending.wasm_hash),
        );

        Ok(())
    }

    /// Apply a pending upgrade once the 7-day review window has fully elapsed.
    ///
    /// Only the `admin` may trigger the actual bytecode swap.  If the timelock
    /// has not yet expired this call returns `Err(ProxyError::TimelockNotElapsed)`.
    ///
    /// Per Soroban semantics, `update_current_contract_wasm` takes effect at
    /// the *end* of the current top-level invocation — the currently executing
    /// code finishes under the old WASM, and all subsequent invocations use the
    /// new WASM.  Existing storage is untouched because Soroban keys storage by
    /// contract address, not by installed bytecode.
    ///
    /// Returns the content hash of the newly activated WASM.
    pub fn execute_upgrade(env: Env) -> Result<BytesN<32>, ProxyError> {
        let admin = Self::load_admin(&env)?;
        admin.require_auth();

        let pending: PendingUpgrade = env
            .storage()
            .persistent()
            .get(&DataKey::UpgradeProxyPending)
            .ok_or(ProxyError::NoPendingUpgrade)?;

        // Enforce the 7-day timelock — this is the critical security gate.
        assert_elapsed(&env, &pending)?;

        // Clear the pending slot before performing the swap so that any
        // re-entrant path (not possible today in Soroban but defensive) would
        // not find a stale proposal.
        env.storage()
            .persistent()
            .remove(&DataKey::UpgradeProxyPending);

        // Swap the on-chain bytecode.  Takes effect after this invocation.
        env.deployer()
            .update_current_contract_wasm(pending.wasm_hash.clone());

        env.events().publish(
            (symbol_short!("execute"),),
            pending.wasm_hash.clone(),
        );

        Ok(pending.wasm_hash)
    }

    // -----------------------------------------------------------------------
    // Read-only queries
    // -----------------------------------------------------------------------

    /// Return the current pending upgrade, or `None` if no upgrade is queued.
    pub fn pending_upgrade(env: Env) -> Option<PendingUpgrade> {
        env.storage()
            .persistent()
            .get(&DataKey::UpgradeProxyPending)
    }

    /// Return the current admin address.
    pub fn admin(env: Env) -> Result<Address, ProxyError> {
        Self::load_admin(&env)
    }

    /// Return the current DAO council address.
    pub fn dao_council(env: Env) -> Result<Address, ProxyError> {
        Self::load_council(&env)
    }

    /// Return the number of seconds remaining until the pending upgrade can
    /// be executed.  Returns `0` if no upgrade is pending or the timelock has
    /// already elapsed.
    pub fn timelock_remaining(env: Env) -> u64 {
        let now = env.ledger().timestamp();
        let pending: Option<PendingUpgrade> = env
            .storage()
            .persistent()
            .get(&DataKey::UpgradeProxyPending);

        match pending {
            Some(p) if p.execute_after > now => p.execute_after - now,
            _ => 0,
        }
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Load the admin address, returning `Err(NotInitialized)` if absent.
    fn load_admin(env: &Env) -> Result<Address, ProxyError> {
        env.storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::UpgradeProxyAdmin)
            .ok_or(ProxyError::NotInitialized)
    }

    /// Load the DAO council address, returning `Err(NotInitialized)` if absent.
    fn load_council(env: &Env) -> Result<Address, ProxyError> {
        env.storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::UpgradeProxyDaoCouncil)
            .ok_or(ProxyError::NotInitialized)
    }
}
