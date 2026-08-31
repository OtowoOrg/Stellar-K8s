//! Deliberately-vulnerable mock contract used to prove the middleware works.
//!
//! The mock models a simplified Soroban `Vault` with a shared, per-slot balance
//! and a `withdraw` flow whose "send funds" step performs an external
//! cross-contract invocation that the attacker controls.
//!
//! # The attack
//!
//! The classic reentrancy exploit: `withdraw` reads the balance, then calls out
//! to an attacker-controlled contract **before** committing its own balance
//! write. The attacker's callback re-enters `withdraw` and withdraws again
//! against the *still-uncommitted* (stale) balance. Without a guard the vault
//! pays out more than it holds; with the guard the second, nested mutation of
//! the same slot is reverted.
//!
//! # Usage
//!
//! Compare [`Vault::withdraw_unguarded`] (which `Ok(200)` in the attack, showing
//! `200` units escaping a `100`-unit vault) against [`Vault::withdraw_guarded`]
//! (which reverts the nested call and only ever moves `100`).

use crate::guard::{AccessKind, GuardError, ReentrancyGuard, SlotId};
use crate::mem::InMemoryStorage;
use alloc::collections::BTreeMap;
use alloc::fmt;

/// Errors surfaced by the mock vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultError {
    /// The cross-contract re-entrant call was rejected by the guard.
    Guard(GuardError),
    /// `withdraw` was asked for more than the slot holds (also the observable
    /// corruption an unguarded reentry produces).
    InsufficientBalance,
    /// Simulated arithmetic corruption observed after an unguarded reentry
    /// (balance would underflow).
    BalanceCorruption,
}

impl fmt::Display for VaultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Guard(e) => write!(f, "guarded vault reverted: {e}"),
            Self::InsufficientBalance => write!(f, "insufficient balance"),
            Self::BalanceCorruption => {
                write!(f, "balance corruption (reentry committed twice)")
            }
        }
    }
}

/// A simplistic vault holding one `u64` balance per state-variable slot.
///
/// It models the *sub-contract* whose state a reentrancy attacker would target;
/// `slot` is the state-variable identifier the guard locks on.
#[derive(Debug, Default)]
pub struct Vault {
    balances: BTreeMap<SlotId, u64>,
    /// Running total of funds actually sent, for asserting on the exploit.
    total_sent: u64,
}

impl Vault {
    /// A new, empty vault.
    pub fn new() -> Self {
        Self::default()
    }

    /// Fund `slot` with `amount` (the attacker's upfront deposit).
    pub fn deposit(&mut self, slot: SlotId, amount: u64) {
        let b = self.balances.entry(slot).or_insert(0);
        *b += amount;
    }

    /// Current balance of `slot`.
    pub fn balance(&self, slot: SlotId) -> u64 {
        *self.balances.get(&slot).unwrap_or(&0)
    }

    /// Total funds ever sent by the vault (for assertions).
    pub fn total_sent(&self) -> u64 {
        self.total_sent
    }

    /// **Unguarded** `withdraw`.
    ///
    /// Mirrors the textbook reentrancy bug: the balance is *sent* (and the
    /// ledger write committed) after an attacker-controlled cross-contract
    /// call. If that call re-enters `withdraw` before the outer ledger write,
    /// the nested call validates against the still-stale balance snapshot, so
    /// the same funds are paid out twice. The outer write then observes a
    /// corrupted (underflowed) balance.
    pub fn withdraw_unguarded(
        &mut self,
        slot: SlotId,
        amount: u64,
        reenter: bool,
    ) -> Result<u64, VaultError> {
        let balance_snapshot = self.balance(slot);
        self.require_funds(balance_snapshot, amount)?;

        // ---- attacker-controlled external cross-contract call happens here ----
        if reenter {
            // Re-enter before the balance write is committed: the nested call
            // validates against the same (still-uncommitted) snapshot.
            self.withdraw_unguarded(slot, amount, false)?;
        }
        // ---------------------------------------------------------------------

        // The funds have already been released. Commit now — but on the
        // reentered path the balance was already drained by the nested call.
        self.total_sent += amount;
        let b = self.balances.entry(slot).or_insert(0);
        let updated = (*b).checked_sub(amount).ok_or(VaultError::BalanceCorruption)?;
        *b = updated;
        Ok(amount)
    }

    /// **Guarded** `withdraw` (the middleware fix).
    ///
    /// Wraps the same mutating flow with [`ReentrancyGuard::enter`] /
    /// [`ReentrancyGuard::exit`]. The nested, attacker-driven re-entry targets
    /// the *same* state-variable slot and is therefore detected and reverted,
    /// so the outer call aborts before committing anything.
    pub fn withdraw_guarded(
        &mut self,
        guard: &mut ReentrancyGuard<InMemoryStorage>,
        slot: SlotId,
        amount: u64,
        reenter: bool,
    ) -> Result<u64, VaultError> {
        // Acquire the write lock on the state variable we are about to mutate.
        guard
            .enter(slot, AccessKind::Write)
            .map_err(VaultError::Guard)?;

        let balance = self.balance(slot);
        self.require_funds(balance, amount).inspect_err(|_| {
            // Release the lock on the failure path so the guard stays usable.
            guard.exit(slot, AccessKind::Write).ok();
        })?;

        // ---- external cross-contract call / attacker re-entry happens here ----
        if reenter {
            // This nested mutation of the same slot is rejected by the guard,
            // which propagates as Err and aborts the outer call (full revert).
            self.withdraw_guarded(guard, slot, amount, false).inspect_err(|_| {
                guard.exit(slot, AccessKind::Write).ok();
            })?;
        }
        // ---------------------------------------------------------------------

        let b = self.balances.entry(slot).or_insert(0);
        *b -= amount;
        self.total_sent += amount;
        guard.exit(slot, AccessKind::Write).map_err(VaultError::Guard)?;
        Ok(amount)
    }

    fn require_funds(&self, balance: u64, amount: u64) -> Result<(), VaultError> {
        if amount > balance {
            return Err(VaultError::InsufficientBalance);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BALANCE_SLOT: SlotId = SlotId::from_u64(1001);

    #[test]
    fn unguarded_vault_is_exploited() {
        let mut vault = Vault::new();
        vault.deposit(BALANCE_SLOT, 100);
        // Attacker withdraws 100; the external call re-enters and withdraws 100
        // again against the stale, uncommitted balance.
        let result = vault.withdraw_unguarded(BALANCE_SLOT, 100, true);
        // 100 was paid by the nested call and another 100 by the outer call, so
        // 200 escaped a 100-unit vault, and the ledger is left corrupted.
        assert_eq!(vault.total_sent(), 200);
        assert!(vault.total_sent() > 100, "attacker extracted more than the deposit");
        assert_eq!(result, Err(VaultError::BalanceCorruption));
    }

    #[test]
    fn guarded_vault_blocks_reentrancy() {
        let mut vault = Vault::new();
        vault.deposit(BALANCE_SLOT, 100);

        let mut guard = ReentrancyGuard::new(InMemoryStorage::new());
        let err = vault
            .withdraw_guarded(&mut guard, BALANCE_SLOT, 100, true)
            .unwrap_err();
        assert_eq!(err, VaultError::Guard(GuardError::ReentrancyDetected));

        // Nothing was moved and the ledger is intact.
        assert_eq!(vault.total_sent(), 0);
        assert_eq!(vault.balance(BALANCE_SLOT), 100);

        // The guard released its lock on the revert path, so a legitimate,
        // sequential (non-nested) withdrawal still succeeds afterwards.
        let sent = vault.withdraw_guarded(&mut guard, BALANCE_SLOT, 100, false).unwrap();
        assert_eq!(sent, 100);
        assert_eq!(vault.total_sent(), 100);
        assert_eq!(vault.balance(BALANCE_SLOT), 0);
    }

    #[test]
    fn guarded_vault_allows_no_reentry_attack_then_clean_withdraw() {
        let mut vault = Vault::new();
        vault.deposit(BALANCE_SLOT, 100);
        let mut guard = ReentrancyGuard::new(InMemoryStorage::new());

        // Non-malicious withdrawal: no reentry, succeeds and sends exactly 100.
        let sent = vault.withdraw_guarded(&mut guard, BALANCE_SLOT, 100, false).unwrap();
        assert_eq!(sent, 100);
        assert_eq!(vault.total_sent(), 100);
        assert_eq!(vault.balance(BALANCE_SLOT), 0);
    }
}
