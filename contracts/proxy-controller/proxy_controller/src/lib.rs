//! Reusable timelocked upgrade-governance state machine for Soroban
//! contracts, built on top of `env.deployer().update_current_contract_wasm`.
//!
//! Soroban has no delegatecall-style proxy: a contract can only ever replace
//! its own installed Wasm. So instead of deploying a separate "proxy"
//! address in front of an "implementation" address (the EVM pattern), this
//! crate is a library that gets compiled *into* every version of an
//! upgradeable contract. Each version links this module, exposes thin
//! wrapper functions for `propose_upgrade` / `execute_upgrade` /
//! `cancel_upgrade`, and therefore carries the same governance rules forward
//! from version to version. See `mock_v1` and `mock_v2` for a worked example,
//! and `../README.md` for the full migration guide and security analysis.
#![no_std]

pub mod deployer;


/// Minimum delay, in seconds, between a bytecode proposal being submitted
/// and it becoming eligible for live replacement.
pub const TIMELOCK_SECONDS: u64 = 48 * 60 * 60;

/// Storage keys used internally by the upgrade governance state machine.
///
/// Every variant is prefixed with `Proxy` so it cannot collide with a host
/// contract's own storage keys. Soroban serializes fieldless
/// `#[contracttype]` enum variants by name, so as long as the host
/// contract's own key enum never defines a variant literally named
/// `ProxyAdmin`, `ProxySecurityCouncil` or `ProxyPendingUpgrade`, the two key
/// spaces are guaranteed not to overlap -- regardless of what else the host
/// contract stores, in what order it declares its own variants, or how many
/// upgrades it goes through. This is what lets storage survive a Wasm swap
/// unmodified: every version of the host contract must keep these exact
/// variant names in this module (fixed by this crate, not the host), and
/// must never reuse them for its own data.
#[contracttype]
#[derive(Clone)]

enum ProxyDataKey {
    ProxyAdmin,
    ProxySecurityCouncil,
    ProxyPendingUpgrade,
}

/// A bytecode upgrade that has been proposed but not yet applied.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingUpgrade {
    pub wasm_hash: BytesN<32>,
    pub proposed_at: u64,
    pub execute_after: u64,
}

#[contractevent(topics = ["proxy_controller", "propose"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProposeUpgradeEvent {
    pub wasm_hash: BytesN<32>,
    pub execute_after: u64,
}

#[contractevent(topics = ["proxy_controller", "cancel"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CancelUpgradeEvent {
    pub caller: Address,
}

#[contractevent(topics = ["proxy_controller", "upgrade"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecuteUpgradeEvent {
    pub wasm_hash: BytesN<32>,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ProxyError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    UpgradeAlreadyPending = 4,
    NoPendingUpgrade = 5,
    TimelockNotElapsed = 6,
}

/// Initializes the upgrade governance state. Must be called exactly once.
///
/// For production deployments, call this from the host contract's
/// `__constructor` (invoked atomically at deploy time) rather than from a
/// standalone `initialize` function, so the admin/security-council
/// assignment cannot be front-run by a third party racing the legitimate
/// deployer's setup transaction.
pub fn init(env: &Env, admin: &Address, security_council: &Address) -> Result<(), ProxyError> {
    if env.storage().instance().has(&ProxyDataKey::ProxyAdmin) {
        return Err(ProxyError::AlreadyInitialized);
    }
    env.storage()
        .instance()
        .set(&ProxyDataKey::ProxyAdmin, admin);
    env.storage()
        .instance()
        .set(&ProxyDataKey::ProxySecurityCouncil, security_council);
    Ok(())
}

pub fn admin(env: &Env) -> Option<Address> {
    env.storage().instance().get(&ProxyDataKey::ProxyAdmin)
}

pub fn security_council(env: &Env) -> Option<Address> {
    env.storage()
        .instance()
        .get(&ProxyDataKey::ProxySecurityCouncil)
}

pub fn pending_upgrade(env: &Env) -> Option<PendingUpgrade> {
    env.storage()
        .instance()
        .get(&ProxyDataKey::ProxyPendingUpgrade)
}

/// Submits new WASM bytecode and starts the 48-hour timelock.
///
/// Only the configured admin may propose an upgrade. Only one upgrade may be
/// pending at a time; cancel the existing proposal first to replace it.
pub fn propose_upgrade(env: &Env, new_wasm: Bytes) -> Result<BytesN<32>, ProxyError> {
    let admin = admin(env).ok_or(ProxyError::NotInitialized)?;
    admin.require_auth();

    if pending_upgrade(env).is_some() {
        return Err(ProxyError::UpgradeAlreadyPending);
    }

    let wasm_hash = deployer::upload(env, new_wasm);
    let now = env.ledger().timestamp();
    let pending = PendingUpgrade {
        wasm_hash: wasm_hash.clone(),
        proposed_at: now,
        execute_after: now + TIMELOCK_SECONDS,
    };
    env.storage()
        .instance()
        .set(&ProxyDataKey::ProxyPendingUpgrade, &pending);
    ProposeUpgradeEvent {
        wasm_hash: pending.wasm_hash.clone(),
        execute_after: pending.execute_after,
    }
    .publish(env);
    Ok(wasm_hash)
}

/// Cancels a pending upgrade before it takes effect.
///
/// Callable by the admin, or, as an emergency backstop, by the security
/// council multi-sig -- either party can veto a proposal during the review
/// window, which is the "emergency cancellation" required for premature or
/// malicious proposals.
pub fn cancel_upgrade(env: &Env, caller: Address) -> Result<(), ProxyError> {
    caller.require_auth();

    let admin = admin(env).ok_or(ProxyError::NotInitialized)?;
    let council = security_council(env).ok_or(ProxyError::NotInitialized)?;
    if caller != admin && caller != council {
        return Err(ProxyError::Unauthorized);
    }

    if pending_upgrade(env).is_none() {
        return Err(ProxyError::NoPendingUpgrade);
    }
    env.storage()
        .instance()
        .remove(&ProxyDataKey::ProxyPendingUpgrade);
    CancelUpgradeEvent { caller }.publish(env);
    Ok(())
}

/// Applies a pending upgrade once the timelock has elapsed.
///
/// Only the admin may trigger the actual bytecode swap, and only once
/// `env.ledger().timestamp()` has reached the proposal's `execute_after`.
pub fn execute_upgrade(env: &Env) -> Result<BytesN<32>, ProxyError> {
    let admin = admin(env).ok_or(ProxyError::NotInitialized)?;
    admin.require_auth();

    let pending = pending_upgrade(env).ok_or(ProxyError::NoPendingUpgrade)?;
    if env.ledger().timestamp() < pending.execute_after {
        return Err(ProxyError::TimelockNotElapsed);
    }

    env.storage()
        .instance()
        .remove(&ProxyDataKey::ProxyPendingUpgrade);
    deployer::apply(env, pending.wasm_hash.clone());
    ExecuteUpgradeEvent {
        wasm_hash: pending.wasm_hash.clone(),
    }
    .publish(env);
    Ok(pending.wasm_hash)
}
