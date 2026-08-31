//! Comprehensive security test-suite for the Reentrancy Guard middleware.
//!
//! This is the concrete validation the issue's *Definition of Done* requires:
//! a deliberately vulnerable mock contract is proven to be exploitable, and the
//! same attack is proven to be blocked once the middleware wraps it, while safe
//! patterns (read callbacks and distinct state variables) are proven to keep
//! producing zero false positives.
//!
//! Run with:
//!
//! ```text
//! cargo test --manifest-path wasm-plugins/security/reentrancy/Cargo.toml
//! ```

use stellar_soroban_reentrancy_guard::{
    guard::{AccessKind, GuardError, ReentrancyGuard, SlotId},
    mem::InMemoryStorage,
    vuln::Vault,
};

/// The mocked on-ledger state slot that the attacker's re-entry targets.
const BALANCE: SlotId = SlotId::from_u64(0x5000_0000_0000_0001);
/// A second, unrelated state slot used to prove per-variable isolation.
const OTHER_STATE: SlotId = SlotId::from_u64(0x5000_0000_0000_0002);

/// A write-lock stack that never exceeds this depth for the entire suite,
/// mirroring the `< 500` instruction budget (the scan is bounded by depth).
const MAX_STACK_DEPTH: usize = 8;

fn guard() -> ReentrancyGuard<InMemoryStorage> {
    ReentrancyGuard::new(InMemoryStorage::new())
}

#[test]
fn unguarded_mock_contract_is_exploitable() {
    // Establish the vulnerability: without the middleware the vault pays out
    // MORE than it ever held (200 from a 100 deposit).
    let mut vault = Vault::new();
    vault.deposit(BALANCE, 100);

    let result = vault.withdraw_unguarded(BALANCE, 100, true);

    assert!(vault.total_sent() > 100, "reentrancy drained funds beyond the deposit");
    assert_eq!(vault.total_sent(), 200);
    // The ledger write is corrupted once the re-entry has already drained it.
    assert_eq!(result, Err(crate_error::BalanceCorruption));
}

#[test]
fn guarded_mock_contract_blocks_reentrancy() {
    // The same attack, this time wrapped by the middleware.
    let mut vault = Vault::new();
    vault.deposit(BALANCE, 100);
    let mut g = guard();

    let err = vault.withdraw_guarded(&mut g, BALANCE, 100, true).unwrap_err();
    assert_eq!(err, crate_error::Guard(GuardError::ReentrancyDetected));

    // Revert semantics: nothing moved, nothing corrupted.
    assert_eq!(vault.total_sent(), 0);
    assert_eq!(vault.balance(BALANCE), 100);

    // The guard released its lock on the revert path, so a subsequent,
    // legitimate withdrawal still works (no lock leak / denial of service).
    let sent = vault.withdraw_guarded(&mut g, BALANCE, 100, false).unwrap();
    assert_eq!(sent, 100);
    assert_eq!(vault.total_sent(), 100);
    assert_eq!(vault.balance(BALANCE), 0);
}

#[test]
fn read_callback_is_never_a_false_positive() {
    // Legitimate, non-mutating read callbacks are the case the issue says MUST
    // produce zero false positives.
    let mut g = guard();

    let write = g.enter(BALANCE, AccessKind::Write).unwrap();
    assert!(!write.was_read_only);

    // While a write is in flight, a read on the SAME slot is allowed.
    let read = g.enter(BALANCE, AccessKind::Read).unwrap();
    assert!(read.was_read_only);
    g.exit(BALANCE, AccessKind::Read).unwrap();

    // The read must not have disturbed the active write lock.
    assert!(g.is_entered(BALANCE).unwrap());

    g.exit(BALANCE, AccessKind::Write).unwrap();
    assert!(!g.is_entered(BALANCE).unwrap());
}

#[test]
fn cross_contract_stack_tracks_multiple_distinct_slots() {
    // A realistic cross-contract chain mutates several distinct state
    // variables; none of these legitimate nested writes may be rejected.
    let mut g = guard();
    for depth in 0..MAX_STACK_DEPTH {
        let slot = SlotId::from_u64(0x6000_0000_0000_0000 + depth as u64);
        let entered = g.enter(slot, AccessKind::Write).unwrap();
        assert_eq!(entered.depth, depth, "stack depth must be tracked");
    }
    // A nested write to an ALREADY-MUTATING slot is still rejected, even deep.
    let err = g.enter(SlotId::from_u64(0x6000_0000_0000_0002), AccessKind::Write).unwrap_err();
    assert_eq!(err, GuardError::ReentrancyDetected);
    // Drain the stack.
    for depth in (0..MAX_STACK_DEPTH).rev() {
        let slot = SlotId::from_u64(0x6000_0000_0000_0000 + depth as u64);
        g.exit(slot, AccessKind::Write).unwrap();
    }
    assert!(!g.is_entered(SlotId::from_u64(0x6000_0000_0000_0000)).unwrap());
}

#[test]
fn distinct_state_variables_do_not_interfere() {
    let mut g = guard();
    g.enter(BALANCE, AccessKind::Write).unwrap();
    // Mutating an unrelated variable while `BALANCE` is locked is safe and
    // must remain allowed (no false positive across variables).
    let other = g.enter(OTHER_STATE, AccessKind::Write).unwrap();
    assert_eq!(other.depth, 1);
    g.exit(OTHER_STATE, AccessKind::Write).unwrap();
    g.exit(BALANCE, AccessKind::Write).unwrap();
}

#[test]
fn guard_is_reusable_across_transactions() {
    // A fresh transaction shares the same middleware store but starts with an
    // empty stack, so sequential transactions are never falsely blocked.
    let mut vault = Vault::new();
    let mut g = guard();
    for round in 1..=3u64 {
        vault.deposit(BALANCE, 100);
        let sent = vault.withdraw_guarded(&mut g, BALANCE, 100, false).unwrap();
        assert_eq!(sent, 100);
        assert!(!g.is_entered(BALANCE).unwrap(), "stack must be empty after tx {round}");
    }
    assert_eq!(vault.total_sent(), 300);
}

#[test]
fn configmap_selects_scope_without_false_positives() {
    use stellar_soroban_reentrancy_guard::ReentrancyGuardConfig;

    // Operator decodes a ConfigMap into JSON and passes it in.
    let cfg = ReentrancyGuardConfig::from_json(
        r#"{"enabled_namespaces":["payments"],"disabled_contracts":["trickyfx"]}"#,
    )
    .unwrap();

    // Enabled namespace is guarded; unrelated namespace is a no-op (zero
    // overhead / false positives for scopes that opted out).
    assert!(cfg.is_enabled_for("payments", "aaa"));
    assert!(!cfg.is_enabled_for("unrelated", "aaa"));

    // Explicit disable wins for the opted-out contract even inside `payments`.
    assert!(!cfg.is_enabled_for("payments", "trickyfx"));

    // The safe default (empty config) guards everywhere.
    assert!(ReentrancyGuardConfig::default().is_enabled_for("any", "any"));
}

/// Namespace for reusing library error variants inside the integration suite to
/// keep the assertions readable.
mod crate_error {
    pub use stellar_soroban_reentrancy_guard::vuln::VaultError::{
        BalanceCorruption, Guard as Guard,
    };
}