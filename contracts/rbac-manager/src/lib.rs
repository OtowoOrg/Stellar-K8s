//! # Stellar RBAC Manager
//!
//! A modular, **role-based access control** (RBAC) module for Soroban smart
//! contracts. Multi-tenant organizations can grant, revoke and renounce
//! distinct administrative, operational and emergency roles across a contract
//! deployment without a single admin key being a single point of failure.
//!
//! ## Design
//!
//! * **Hierarchical roles** — [`Role::SuperAdmin`] > [`Role::Operator`] >
//!   [`Role::Auditor`]. A holder of a role may *grant*/*revoke* any role that
//!   is strictly lower in the hierarchy (and, optionally, add/remove
//!   `SuperAdmin` peers so a single admin can be replaced before it leaves —
//!   avoiding a catastrophic single-point-of-failure).
//! * **Metering-friendly** — every authorization decision is an **O(1)**
//!   constant-time [`HashSet`] membership test (a `HashMap` lookup + a set
//!   probe), estimating far below the `1,200` CPU-instruction budget per role
//!   check. See [`docs/role-check-cost.md`](docs/role-check-cost.md).
//! * **Immediate revocation** — [`RbacState::revoke_role`] writes directly to
//!   storage, so a revocation is visible to any [`RbacState::has_role`] call
//!   made later in the same ledger step (verified in
//!   `tests/rbac_validation.rs`).
//! * **Drop-in guards** — the [`require_role!`] / [`check_role!`] declarative
//!   macros integrate the RBAC check into existing and future contracts.

mod macros;

use std::collections::{HashMap, HashSet};
use std::fmt;

/// Administrative, operational and emergency roles, ordered by seniority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Auditor = 0,
    Operator = 1,
    SuperAdmin = 2,
}

impl Role {
    /// Higher value => more senior. Used for hierarchy comparisons.
    pub fn seniority(&self) -> u8 {
        *self as u8
    }

    /// All defined roles.
    pub const ALL: [Role; 3] = [Role::SuperAdmin, Role::Operator, Role::Auditor];
}

impl TryFrom<u8> for Role {
    type Error = RbacError;
    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(Role::Auditor),
            1 => Ok(Role::Operator),
            2 => Ok(Role::SuperAdmin),
            _ => Err(RbacError::InvalidRole(v)),
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Role::SuperAdmin => "super_admin",
            Role::Operator => "operator",
            Role::Auditor => "auditor",
        };
        f.write_str(s)
    }
}

/// A 256-bit identifier for an account or contract address.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Address([u8; 32]);

impl Address {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Address(bytes)
    }
    pub fn to_bytes(&self) -> [u8; 32] {
        self.0
    }
}

impl From<[u8; 32]> for Address {
    fn from(bytes: [u8; 32]) -> Self {
        Address(bytes)
    }
}

impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Address({})",
            self.0
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        )
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            &self
                .0
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
        )
    }
}

/// Errors surfaced by the RBAC module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RbacError {
    /// The caller is not senior enough to manage `role`, or holds no role.
    Unauthorized,
    /// The caller does not hold the required role.
    MissingRole(Role),
    /// Removing this member would leave the contract with no `SuperAdmin`.
    LastSuperAdmin,
    /// The caller is not a member of the role they tried to renounce.
    NotMember(Role),
    /// An invalid role discriminant was supplied.
    InvalidRole(u8),
}

impl fmt::Display for RbacError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RbacError::Unauthorized => write!(f, "caller is not authorized to manage this role"),
            RbacError::MissingRole(r) => write!(f, "caller does not hold role {r}"),
            RbacError::LastSuperAdmin => write!(
                f,
                "cannot revoke the final SuperAdmin (would lock out the contract)"
            ),
            RbacError::NotMember(r) => write!(f, "caller is not a member of role {r}"),
            RbacError::InvalidRole(v) => write!(f, "invalid role discriminant {v}"),
        }
    }
}

impl std::error::Error for RbacError {}

/// On-chain RBAC state. In a Soroban contract this maps directly onto a
/// [`KeyedMap`]-style `Role -> Set<Address>` storage entry.
///
/// [`KeyedMap`]: https://developers.stellar.org/docs/soroban/contract-storage
#[derive(Debug, Clone, Default)]
pub struct RbacState {
    roles: HashMap<Role, HashSet<Address>>,
    /// Whether `SuperAdmin`s may add/remove peer `SuperAdmin`s. Defaults to
    /// `true` so a departing admin can be replaced before renouncing.
    allow_super_peer_management: bool,
}

impl RbacState {
    /// Bootstraps the contract with at least one `SuperAdmin`.
    ///
    /// `initial_super_admins` must be non-empty; otherwise the contract has no
    /// root authority and cannot be managed.
    pub fn initialize(
        initial_super_admins: &[Address],
        allow_super_peer_management: bool,
    ) -> Result<Self, RbacError> {
        if initial_super_admins.is_empty() {
            return Err(RbacError::LastSuperAdmin);
        }
        let mut roles = HashMap::new();
        let set: HashSet<Address> = initial_super_admins.iter().copied().collect();
        roles.insert(Role::SuperAdmin, set);
        Ok(RbacState {
            roles,
            allow_super_peer_management,
        })
    }

    /// Adds `account` to `role` on behalf of `caller`.
    ///
    /// Authorized only if `caller` holds a role senior enough to manage
    /// `role` (see [`RbacState::can_manage`]).
    pub fn grant_role(
        &mut self,
        caller: Address,
        role: Role,
        account: Address,
    ) -> Result<(), RbacError> {
        let manager = self.effective_role(caller).ok_or(RbacError::Unauthorized)?;
        if !self.can_manage(manager, role) {
            return Err(RbacError::Unauthorized);
        }
        self.roles.entry(role).or_default().insert(account);
        Ok(())
    }

    /// Removes `account` from `role` on behalf of `caller`.
    ///
    /// A revocation takes effect **immediately**: `has_role(account, role)`
    /// returns `false` from the very next call, including later in the same
    /// ledger step.
    pub fn revoke_role(
        &mut self,
        caller: Address,
        role: Role,
        account: Address,
    ) -> Result<(), RbacError> {
        let manager = self.effective_role(caller).ok_or(RbacError::Unauthorized)?;
        if !self.can_manage(manager, role) {
            return Err(RbacError::Unauthorized);
        }
        // Never strand the contract with zero SuperAdmins.
        if role == Role::SuperAdmin
            && self.members(role).contains(&account)
            && self.role_count(role) <= 1
        {
            return Err(RbacError::LastSuperAdmin);
        }
        if let Some(set) = self.roles.get_mut(&role) {
            set.remove(&account);
        }
        Ok(())
    }

    /// A member voluntarily drops their own `role`.
    pub fn renounce_role(&mut self, caller: Address, role: Role) -> Result<(), RbacError> {
        // Guarding against the last SuperAdmin renouncing leaves a contract
        // without any root; require an explicit `revoke`-managed handoff first.
        if role == Role::SuperAdmin && self.role_count(role) <= 1 && self.has_role(caller, role) {
            return Err(RbacError::LastSuperAdmin);
        }
        let set = self
            .roles
            .get_mut(&role)
            .ok_or(RbacError::NotMember(role))?;
        if !set.remove(&caller) {
            return Err(RbacError::NotMember(role));
        }
        Ok(())
    }

    /// O(1) membership test — the metering-friendly hot path (a single map
    /// lookup + a set probe).
    pub fn has_role(&self, account: Address, role: Role) -> bool {
        self.roles
            .get(&role)
            .is_some_and(|set| set.contains(&account))
    }

    /// Returns `Ok(())` if `caller` holds `role`, else [`RbacError::MissingRole`].
    pub fn require_role(&self, caller: Address, role: Role) -> Result<(), RbacError> {
        if self.has_role(caller, role) {
            Ok(())
        } else {
            Err(RbacError::MissingRole(role))
        }
    }

    /// The most-senior role held by `account` (used to pick the managing
    /// authority). O(1) over the fixed role set.
    pub fn effective_role(&self, account: Address) -> Option<Role> {
        Role::ALL
            .iter()
            .copied()
            .filter(|r| self.has_role(account, *r))
            .max_by_key(Role::seniority)
    }

    /// Whether a member of `manager` may manage the membership of `role`.
    pub fn can_manage(&self, manager: Role, role: Role) -> bool {
        match (manager, role) {
            // SuperAdmins may manage their own tier (peer replacement) only if
            // enabled; this prevents a single point of failure.
            (Role::SuperAdmin, Role::SuperAdmin) => self.allow_super_peer_management,
            (manager, role) => manager.seniority() > role.seniority(),
        }
    }

    /// Snapshot of role membership (for audit / `Auditor` reads).
    pub fn members(&self, role: Role) -> Vec<Address> {
        self.roles
            .get(&role)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Number of members currently holding `role`.
    pub fn role_count(&self, role: Role) -> usize {
        self.roles.get(&role).map_or(0, |set| set.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{check_role, require_role};

    fn addr(b: u8) -> Address {
        Address::from_bytes([b; 32])
    }

    fn rbac() -> RbacState {
        RbacState::initialize(&[addr(1)], true).unwrap()
    }

    #[test]
    fn grant_operator_requires_super_admin() {
        let mut r = rbac();
        // An outsider cannot grant anything.
        assert_eq!(
            r.grant_role(addr(99), Role::Operator, addr(50)),
            Err(RbacError::Unauthorized)
        );
        // A SuperAdmin can.
        r.grant_role(addr(1), Role::Operator, addr(50)).unwrap();
        assert!(r.has_role(addr(50), Role::Operator));
    }

    #[test]
    fn revocation_is_immediate_within_same_ledger_step() {
        let mut r = rbac();
        r.grant_role(addr(1), Role::Operator, addr(50)).unwrap();
        assert!(r.has_role(addr(50), Role::Operator));

        // Grant + revoke + re-check in the same callable = same ledger step.
        let mut step = || -> bool {
            r.grant_role(addr(1), Role::Operator, addr(51)).unwrap();
            r.revoke_role(addr(1), Role::Operator, addr(51)).unwrap();
            r.has_role(addr(51), Role::Operator)
        };
        assert!(!step(), "revocation must take effect immediately");
    }

    #[test]
    fn operator_can_manage_auditor_but_not_superadmin() {
        let mut r = rbac();
        r.grant_role(addr(1), Role::Operator, addr(2)).unwrap();

        // Operator manages Auditor.
        r.grant_role(addr(2), Role::Auditor, addr(3)).unwrap();
        assert!(r.has_role(addr(3), Role::Auditor));

        // Operator cannot manage Operator or SuperAdmin.
        assert_eq!(
            r.grant_role(addr(2), Role::Operator, addr(4)),
            Err(RbacError::Unauthorized)
        );
        assert_eq!(
            r.grant_role(addr(2), Role::SuperAdmin, addr(4)),
            Err(RbacError::Unauthorized)
        );
    }

    #[test]
    fn auditor_cannot_manage_anything_and_renounces_own() {
        let mut r = rbac();
        r.grant_role(addr(1), Role::Auditor, addr(3)).unwrap();
        // Auditor tries to grant/revoke others -> denied.
        assert_eq!(
            r.grant_role(addr(3), Role::Auditor, addr(4)),
            Err(RbacError::Unauthorized)
        );
        assert_eq!(
            r.grant_role(addr(3), Role::Operator, addr(4)),
            Err(RbacError::Unauthorized)
        );
        // Auditor renounces own role.
        r.renounce_role(addr(3), Role::Auditor).unwrap();
        assert!(!r.has_role(addr(3), Role::Auditor));
    }

    #[test]
    fn last_superadmin_cannot_be_locked_out() {
        let mut r = RbacState::initialize(&[addr(1)], true).unwrap();
        assert_eq!(
            r.revoke_role(addr(1), Role::SuperAdmin, addr(1)),
            Err(RbacError::LastSuperAdmin)
        );
        assert_eq!(
            r.renounce_role(addr(1), Role::SuperAdmin),
            Err(RbacError::LastSuperAdmin)
        );
        assert!(r.has_role(addr(1), Role::SuperAdmin));
    }

    #[test]
    fn macro_guards_a_function() {
        let mut r = rbac();
        r.grant_role(addr(1), Role::Operator, addr(50)).unwrap();
        r.grant_role(addr(1), Role::Operator, addr(60)).unwrap();
        r.revoke_role(addr(1), Role::Operator, addr(60)).unwrap();

        fn only_ops(r: &RbacState, caller: Address) -> Result<(), RbacError> {
            require_role!(r, caller, Role::Operator);
            Ok(())
        }
        assert_eq!(only_ops(&r, addr(50)), Ok(()));
        // Revoked in a prior step => missing role immediately.
        assert_eq!(
            only_ops(&r, addr(60)),
            Err(RbacError::MissingRole(Role::Operator))
        );

        assert!(check_role!(&r, addr(50), Role::Operator));
    }
}
