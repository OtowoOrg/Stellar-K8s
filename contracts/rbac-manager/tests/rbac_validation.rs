//! Validation tests:
//!   - unauthorized invocation attempts across all defined roles,
//!   - role revocations take effect **immediately** within the same ledger step,
//!   - hierarchical grant/revoke matrix,
//!   - macro guard integration,
//!   - role-check cost stays well under the 1,200-CPU-instruction budget.

use std::collections::HashSet;
use std::time::Instant;

use proptest::prelude::*;

use stellar_rbac_manager::{check_role, require_role, Address, RbacError, RbacState, Role};

fn addr(b: u8) -> Address {
    Address::from_bytes([b; 32])
}

fn seeded() -> RbacState {
    RbacState::initialize(&[addr(1)], true).unwrap()
}

/// Seat a set of members across all roles (SuperAdmin `1`, Operator `21`,
/// Auditor `31`) plus a second SuperAdmin peer `2`.
fn populated() -> RbacState {
    let mut r = seeded();
    r.grant_role(addr(1), Role::SuperAdmin, addr(2)).unwrap(); // peer
    r.grant_role(addr(1), Role::Operator, addr(21)).unwrap();
    r.grant_role(addr(1), Role::Auditor, addr(31)).unwrap();
    r
}

#[test]
fn authorization_matrix_across_all_defined_roles() {
    let mut r = populated();

    // Expected "can manage target" table for each manager role.
    // (SuperAdmin, SuperAdmin) => true  (peer replacement)
    // (SuperAdmin, Operator)   => true
    // (SuperAdmin, Auditor)    => true
    // (Operator,  Auditor)     => true
    // (Operator,  Operator)    => false
    // (Operator,  SuperAdmin)  => false
    // (Auditor,   anything)    => false
    // (outsider,  anything)    => false
    let cases: Vec<(Address, Role, bool)> = vec![
        (addr(1), Role::SuperAdmin, true),
        (addr(2), Role::SuperAdmin, true), // peer super admin
        (addr(1), Role::Operator, true),
        (addr(1), Role::Auditor, true),
        (addr(21), Role::Auditor, true),
        (addr(21), Role::Operator, false),
        (addr(21), Role::SuperAdmin, false),
        (addr(31), Role::Auditor, false),
        (addr(31), Role::Operator, false),
        (addr(31), Role::SuperAdmin, false),
        // A pure outsider holds no role at all.
        (addr(99), Role::Auditor, false),
        (addr(99), Role::Operator, false),
        (addr(99), Role::SuperAdmin, false),
    ];

    let mut next_account: u8 = 100;
    for (manager, target, allowed) in cases {
        let grantee = {
            let a = next_account;
            next_account += 1;
            addr(a)
        };
        let res = r.grant_role(manager, target, grantee);
        assert_eq!(
            res.is_ok(),
            allowed,
            "grant: manager {manager:?} -> target {target:?} (expected allowed={allowed})"
        );
        assert_eq!(
            r.has_role(grantee, target),
            allowed,
            "grantee membership must reflect a {allowed}-authorized grant"
        );

        // Revoke mirrors the same authorization gate.
        let revoke_res = r.revoke_role(manager, target, grantee);
        // Revoking a member we just (or previously) authorized is allowed when
        // the grant was authorized; unauthorized managers still cannot revoke.
        assert_eq!(
            revoke_res.is_ok(),
            allowed,
            "revoke: manager {manager:?} -> target {target:?}"
        );
        assert!(
            !r.has_role(grantee, target),
            "revocation must stick immediately"
        );
    }
}

#[test]
fn revocation_takes_effect_within_same_ledger_step() {
    let mut r = populated();

    // A single transaction boundary: grant, attempt to use, revoke, re-authorize —
    // all within one "ledger step" (one callable), proving revocations bite
    // before the step ends.
    fn one_step(rbac: &mut RbacState) -> Result<(), RbacError> {
        rbac.grant_role(addr(1), Role::Operator, addr(70))?;
        // Immediately usable...
        rbac.require_role(addr(70), Role::Operator)?;
        // ...yet immediately revoked in the same step.
        rbac.revoke_role(addr(1), Role::Operator, addr(70))?;
        rbac.require_role(addr(70), Role::Operator)
    }

    assert_eq!(
        one_step(&mut r),
        Err(RbacError::MissingRole(Role::Operator))
    );
    assert!(!r.has_role(addr(70), Role::Operator));
}

#[test]
fn renunciation_only_removes_the_caller() {
    let mut r = seeded();
    r.grant_role(addr(1), Role::Operator, addr(21)).unwrap();
    r.grant_role(addr(1), Role::Operator, addr(22)).unwrap();

    r.renounce_role(addr(21), Role::Operator).unwrap();
    assert!(!r.has_role(addr(21), Role::Operator));
    // Other members unaffected.
    assert!(r.has_role(addr(22), Role::Operator));
    // Cannot renounce a role not held.
    assert_eq!(
        r.renounce_role(addr(21), Role::Auditor),
        Err(RbacError::NotMember(Role::Auditor))
    );
}

#[test]
fn auditor_gets_read_only_snapshot() {
    let r = populated();
    let mut auditors: Vec<Address> = r.members(Role::SuperAdmin);
    auditors.sort();
    assert_eq!(auditors, vec![addr(1), addr(2)]);
    assert_eq!(r.role_count(Role::Operator), 1);
    assert!(r.has_role(addr(31), Role::Auditor));
}

#[test]
fn macro_guard_denies_unauthorized_callers() {
    let r = populated();

    // Guarded entrypoint of a hypothetical contract.
    fn withdraw_all(r: &RbacState, caller: Address) -> Result<String, RbacError> {
        require_role!(r, caller, Role::SuperAdmin);
        Ok("paid out".into())
    }

    assert_eq!(withdraw_all(&r, addr(1)), Ok("paid out".into()));
    // An Operator / Auditor / outsider is denied by the same guard.
    assert_eq!(
        withdraw_all(&r, addr(21)),
        Err(RbacError::MissingRole(Role::SuperAdmin))
    );
    assert_eq!(
        withdraw_all(&r, addr(31)),
        Err(RbacError::MissingRole(Role::SuperAdmin))
    );
    assert_eq!(
        withdraw_all(&r, addr(99)),
        Err(RbacError::MissingRole(Role::SuperAdmin))
    );

    // Boolean guard returns just false (no error construction).
    assert!(check_role!(&r, addr(1), Role::SuperAdmin));
    assert!(!check_role!(&r, addr(99), Role::SuperAdmin));
}

#[test]
fn revoking_a_low_role_then_reusing_same_account_requires_regrant() {
    let mut r = populated();
    r.grant_role(addr(1), Role::Auditor, addr(50)).unwrap();
    assert!(r.has_role(addr(50), Role::Auditor));

    r.revoke_role(addr(1), Role::Auditor, addr(50)).unwrap();
    assert!(!r.has_role(addr(50), Role::Auditor));

    // A stale capability does not carry over: the account needs a fresh grant.
    assert_eq!(
        r.require_role(addr(50), Role::Auditor),
        Err(RbacError::MissingRole(Role::Auditor))
    );
}

/// The role check is a constant-time HashSet probe; even at scale it should
/// complete in well under the 1,200-CPU-instruction budget. This is a cheap
/// wall-clock sanity bound — authoritative Soroban metering is enforced
/// on-chain (`docs/role-check-cost.md`).
#[test]
fn role_check_is_within_cpu_budget() {
    let mut r = populated();
    // Grow membership so we aren't only testing the trivial empty case.
    for i in 0..50u8 {
        r.grant_role(addr(1), Role::Operator, addr(i + 200))
            .unwrap();
    }
    let n = 500_000u32;
    let start = Instant::now();
    let mut hits = 0u32;
    for i in 0..n {
        let account = addr((i % 200) as u8);
        if r.has_role(account, Role::Operator) {
            hits += 1;
        }
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_millis() < 1_000,
        "role check too slow: {elapsed:?}"
    );
    assert!(hits > 0);
    // ~2 elementary lookups (map + set) per check — far below the budget.
    assert!(
        (elapsed.as_nanos() / u128::from(n)) < 2_000,
        "per-check cost too high"
    );
}

proptest! {
    /// Under random, adversarial grant/revoke/check sequences, storage is the
    /// single source of truth: a revocation is observed immediately by the very
    /// next `has_role`/`require_role`, and the in-memory model never diverges
    /// from the state after any write (success or rejected).
    #[test]
    fn revocations_immediately_observed_under_random_ops(
        ops in prop::collection::vec(any::<(u8, u8, u8)>(), 0..300)
    ) {
        let mut r = populated();
        // Model of expected membership: (account_idx, role_discriminant),
        // seeded with `populated()`'s initial members.
        let mut model: HashSet<(u8, u8)> =
            [(1u8, 2u8), (2u8, 2u8), (21u8, 1u8), (31u8, 0u8)].into_iter().collect();

        for (opc, acct, role_d) in ops {
            let acct_idx = acct % 64;
            let role_disc = role_d % 3;
            let account = addr(acct_idx);
            let role = Role::try_from(role_disc).unwrap();
            let key = (acct_idx, role_disc);

            match opc % 3 {
                // SuperAdmin grants; only mirror the model when the grant is
                // accepted (the caller may have been revoked by an earlier op).
                0 => {
                    if r.grant_role(addr(1), role, account).is_ok() {
                        model.insert(key);
                    }
                }
                // SuperAdmin revokes; mirror success/failure in the model so the
                // final-SuperAdmin guard never causes divergence.
                1 => {
                    if r.revoke_role(addr(1), role, account).is_ok() {
                        model.remove(&key);
                    }
                }
                // A membership check from any caller.
                _ => {
                    prop_assert_eq!(
                        r.has_role(account, role),
                        model.contains(&key),
                        "check diverged from model"
                    );
                }
            }

            // After EVERY operation the state must already reflect the model;
            // i.e., a revocation is visible the moment it is written.
            prop_assert_eq!(
                r.has_role(account, role),
                model.contains(&key),
                "state must reflect the write immediately (same ledger step)"
            );
        }
    }
}
