# Stellar RBAC Manager

A modular **role-based access control** module for Soroban smart contracts.
Multi-tenant organizations can assign, revoke and renounce administrative,
operational and emergency roles across a deployment — so no single admin key is
a single point of failure.

This is the smart-contract module for the Stellar-K8s project, matching the
spec layout (`contracts/rbac-manager/src/lib.rs` + `macros.rs`) and aligned
with Soroban's minimal-fee model (every authorisation is a constant-time
membership check, see [`docs/role-check-cost.md`](docs/role-check-cost.md)).

> **Note on layout.** The parent operator workspace pins its own toolchain and
> profiles, so this crate is a standalone workspace root. Build from the crate
> directory.

## Roles (hierarchical)

`SuperAdmin` > `Operator` > `Auditor`

| Role | Can grant / revoke | Notes |
| --- | --- | --- |
| **SuperAdmin** | `SuperAdmin` (peers, optional), `Operator`, `Auditor` | Root authority, bootstrapped at `initialize` |
| **Operator** | `Auditor` | Day-to-day operational role |
| **Auditor** | *(none)* | Read-only observer |

Any member can **renounce** their own role; the final `SuperAdmin` is protected
from being revoked/renounced out (no single point of failure).

## Public API

- `initialize(super_admins, allow_super_peer_management)` — bootstrap (≥1 root).
- `grant_role(caller, role, account)` — assign a role (hierarchy-gated).
- `revoke_role(caller, role, account)` — remove a role. **Takes effect
  immediately**, even within the same ledger step.
- `renounce_role(caller, role)` — drop your own role.
- `has_role(account, role)` / `require_role(caller, role)` — O(1) checks.
- `members(role)` / `role_count(role)` — audit read path.

## Drop-in guards (`macros.rs`)

Add the check to any contract entrypoint:

```rust
use stellar_rbac_manager::{require_role, check_role, Role, RbacError};

// As a `?`-style guard (early-returns `RbacError::MissingRole`):
fn pay(rbac: &RbacState, caller: Address) -> Result<i128, RbacError> {
    require_role!(rbac, caller, Role::Operator);
    // ... only Operators reach here
    Ok(42)
}

// As a pure boolean (lowest cost, no error object):
if check_role!(rbac, caller, Role::Auditor) { /* audit branch */ }
```

## Build & test

```bash
cd contracts/rbac-manager
cargo build
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Toolchain: any stable `Rust 1.88+` (verified on `1.92`).

## Tests (`tests/rbac_validation.rs` + unit tests)

- **Authorization matrix across all defined roles** — every
  `(manager, target)` pair is exhaustively checked (grant *and* revoke),
  including an outsider with no role.
- **Immediate same-ledger-step revocation** — grant → use → revoke → use, all
  inside one callable; the post-revoke check fails.
- **Hierarchy** — Operators manage Auditors but not Operators/SuperAdmins;
  Auditors manage nothing.
- **Renunciation** — only removes the caller; other members unaffected.
- **Final-SuperAdmin protection** — the last root cannot be revoked/renounced.
- **Macro integration** — `require_role!` guards deny every non-`SuperAdmin`.
- **Cost sanity** — 500k role checks complete in microseconds per check.
- **Property test** — 300-op random grant/revoke/check sequences never let
  state diverge from the membership model; revocations bind immediately.