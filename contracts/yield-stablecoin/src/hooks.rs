//! # Compliance Hooks
//!
//! Programmable compliance gate that is called before **every** mint, burn,
//! and transfer operation.  Two independent axes of restriction are enforced:
//!
//! * **Sanctions / blacklist** – an admin-managed set of addresses that are
//!   permanently blocked from receiving or sending tokens.  Attempting to mint
//!   to, burn from, or transfer to/from a blacklisted address panics with
//!   [`ComplianceError::Blacklisted`].
//!
//! * **Account freeze** – a lighter administrative action that halts *all*
//!   inbound and outbound flows for a specific address without permanently
//!   removing it from the system.  A frozen account can be thawed; a
//!   blacklisted address cannot be un-blacklisted through the normal admin
//!   path (it requires a super-admin quorum – see `lib.rs`).
//!
//! ## Storage layout
//!
//! Both flags are stored in **persistent** storage under per-address keys so
//! they survive ledger TTL expiration of the instance entry.  The keys are:
//!
//! | Key variant              | Type  | Meaning                        |
//! |--------------------------|-------|--------------------------------|
//! | `DataKey::Blacklisted(a)`| `bool`| Address is on the sanctions list |
//! | `DataKey::Frozen(a)`     | `bool`| Address is temporarily frozen  |
//!
//! Absence of a key is semantically equivalent to `false` (not restricted).

use soroban_sdk::{contracterror, Address, Env};

use crate::storage::DataKey;

// ── Errors ────────────────────────────────────────────────────────────────────

/// Errors surfaced exclusively by the compliance layer.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ComplianceError {
    /// The address appears on the on-chain sanctions / blacklist.
    Blacklisted = 100,
    /// The address has been administratively frozen; all flows are halted.
    Frozen = 101,
}

// ── Public gate functions ─────────────────────────────────────────────────────

/// Assert that `addr` may **send** tokens (not frozen, not blacklisted).
///
/// Called before every `transfer`, `burn`, and `approve` that debits the
/// sender's balance.
///
/// # Panics
/// Panics with [`ComplianceError::Blacklisted`] or [`ComplianceError::Frozen`]
/// if the address is restricted.
pub fn assert_can_send(env: &Env, addr: &Address) {
    check_blacklist(env, addr);
    check_freeze(env, addr);
}

/// Assert that `addr` may **receive** tokens (not frozen, not blacklisted).
///
/// Called before every `transfer` and `mint` that credits the recipient's
/// balance.
///
/// # Panics
/// Panics with [`ComplianceError::Blacklisted`] or [`ComplianceError::Frozen`]
/// if the address is restricted.
pub fn assert_can_receive(env: &Env, addr: &Address) {
    check_blacklist(env, addr);
    check_freeze(env, addr);
}

/// Combined gate for operations that both debit *and* credit (e.g. transfers).
///
/// Checks sender first so the more permanent restriction (blacklist) is
/// reported before the lighter one (freeze) when both apply.
#[inline]
pub fn assert_transfer_allowed(env: &Env, from: &Address, to: &Address) {
    assert_can_send(env, from);
    assert_can_receive(env, to);
}

// ── Blacklist management ──────────────────────────────────────────────────────

/// Returns `true` if `addr` is on the sanctions list.
pub fn is_blacklisted(env: &Env, addr: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&DataKey::Blacklisted(addr.clone()))
        .unwrap_or(false)
}

/// Add `addr` to the on-chain sanctions list.
///
/// Idempotent — adding an already-blacklisted address is a no-op.
pub fn blacklist(env: &Env, addr: &Address) {
    env.storage()
        .persistent()
        .set(&DataKey::Blacklisted(addr.clone()), &true);
}

/// Remove `addr` from the on-chain sanctions list.
///
/// Requires super-admin multi-sig authorization (enforced in `lib.rs` before
/// calling this function).  Idempotent.
pub fn unblacklist(env: &Env, addr: &Address) {
    env.storage()
        .persistent()
        .remove(&DataKey::Blacklisted(addr.clone()));
}

// ── Freeze management ─────────────────────────────────────────────────────────

/// Returns `true` if `addr` is currently frozen.
pub fn is_frozen(env: &Env, addr: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&DataKey::Frozen(addr.clone()))
        .unwrap_or(false)
}

/// Freeze `addr`, halting all inbound and outbound flows.
///
/// Idempotent.
pub fn freeze(env: &Env, addr: &Address) {
    env.storage()
        .persistent()
        .set(&DataKey::Frozen(addr.clone()), &true);
}

/// Lift the freeze on `addr`, restoring normal operation.
///
/// Idempotent — thawing an already-active address is a no-op.
pub fn thaw(env: &Env, addr: &Address) {
    env.storage()
        .persistent()
        .remove(&DataKey::Frozen(addr.clone()));
}

// ── Private helpers ───────────────────────────────────────────────────────────

#[inline]
fn check_blacklist(env: &Env, addr: &Address) {
    if is_blacklisted(env, addr) {
        env.panic_with_error(&ComplianceError::Blacklisted);
    }
}

#[inline]
fn check_freeze(env: &Env, addr: &Address) {
    if is_frozen(env, addr) {
        env.panic_with_error(&ComplianceError::Frozen);
    }
}
