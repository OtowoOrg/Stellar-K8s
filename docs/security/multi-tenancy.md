# Enterprise RBAC and Multi-Tenancy Integration Guide

This guide describes how platform teams enforce **role-based access control (RBAC)** and **namespace isolation** when multiple business units share a single Stellar-K8s cluster.

## Role hierarchy

Stellar-K8s defines three enterprise roles. Map each role to an identity provider (IdP) group and bind it to Kubernetes RBAC as shown in [`examples/rbac/tenant-policy.yaml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/examples/rbac/tenant-policy.yaml).

| Role | Scope | Kubernetes binding | Primary responsibilities |
| --- | --- | --- | --- |
| **SuperAdmin** | Cluster | `ClusterRole` / `ClusterRoleBinding` | CRD lifecycle, cross-tenant policy, quota overrides, DR drills |
| **Operator** | Namespace (tenant) | `Role` / `RoleBinding` | Deploy and patch `StellarNode` resources, manage PVCs and secrets in tenant namespace |
| **Auditor** | Namespace (tenant) | `Role` / `RoleBinding` | Read-only access to nodes, pods, events, and metrics; no mutation verbs |

### Permission boundaries

**SuperAdmin** may:

- Create and delete tenant namespaces and platform-wide `NetworkPolicy` objects
- Manage `stellarnodes.stellar.org`, `stellarbenchmarks.stellar.org`, and `benchmarkreports.stellar.org` in any namespace
- View cluster nodes and persistent volumes for capacity planning
- Delegate tenant-scoped roles (cannot impersonate tenant Operators for day-to-day node edits unless explicitly granted)

**Operator** may:

- Full CRUD on `StellarNode` and `StellarBenchmark` CRs **within assigned tenant namespace only**
- Manage pods, services, configmaps, secrets, and PVCs in that namespace
- **Cannot** read secrets in other tenants, modify cluster-scoped RBAC, or patch operator deployment in `stellar-system`

**Auditor** may:

- `get`, `list`, `watch` on Stellar CRDs, pod logs, and events in assigned namespace
- **Cannot** create, update, patch, or delete any resource

### Contract-level permission boundaries (Soroban)

On-chain tenant contracts should mirror the Kubernetes role model. The operator's admission webhook can validate that contract admin keys match the tenant IdP group before allowing node deployment.

## Kubernetes multi-tenant isolation

Apply the reference manifests:

```bash
kubectl apply -f examples/rbac/tenant-policy.yaml
kubectl apply --dry-run=client -f examples/rbac/tenant-policy.yaml  # syntax check
```

Each tenant namespace includes:

1. **Pod Security** — `restricted` enforcement profile
2. **ResourceQuota** — caps pods, PVCs, CPU/memory, and `StellarNode` count
3. **NetworkPolicy** — default-deny cross-tenant ingress; allow operator scrape (TCP 9100) and Stellar peer port (11625) from `stellar-system`
4. **RBAC** — separate Operator and Auditor bindings per tenant

Label tenant namespaces consistently:

```yaml
metadata:
  labels:
    stellar.org/tenant: alpha
    pod-security.kubernetes.io/enforce: restricted
```

## Soroban runtime role verification

Use explicit role enums and storage-backed membership checks. Never trust caller-supplied role strings without validation against persistent state.

```rust
use soroban_sdk::{contract, contracterror, contracttype, Address, Env, Symbol};

#[contracttype]
#[derive(Clone)]
pub enum Role {
    SuperAdmin,
    Operator,
    Auditor,
}

#[contracttype]
pub enum DataKey {
    Member(Address),
    Role(Address),
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TenantError {
    Unauthorized = 1,
    UnknownMember = 2,
}

#[contract]
pub struct TenantRbac;

#[contractimpl]
impl TenantRbac {
    /// SuperAdmin assigns a role to a member address.
    pub fn grant_role(env: Env, admin: Address, member: Address, role: Role) {
        admin.require_auth();
        Self::require_role(&env, &admin, Role::SuperAdmin);
        env.storage().persistent().set(&DataKey::Role(member), &role);
    }

    /// Operators may invoke privileged contract functions.
    pub fn operator_action(env: Env, caller: Address) -> Result<(), TenantError> {
        caller.require_auth();
        Self::require_role(&env, &caller, Role::Operator)?;
        Ok(())
    }

    fn require_role(env: &Env, who: &Address, required: Role) -> Result<(), TenantError> {
        let stored: Role = env
            .storage()
            .persistent()
            .get(&DataKey::Role(who.clone()))
            .ok_or(TenantError::UnknownMember)?;
        if !role_at_least(stored, required) {
            return Err(TenantError::Unauthorized);
        }
        Ok(())
    }
}

fn role_at_least(actual: Role, required: Role) -> bool {
    matches!(
        (actual, required),
        (Role::SuperAdmin, _)
            | (Role::Operator, Role::Operator | Role::Auditor)
            | (Role::Auditor, Role::Auditor)
    )
}
```

### Integration with Stellar-K8s admission

1. Store tenant contract ID in `StellarNode` annotations (`stellar.org/tenant-contract`).
2. Configure the Wasm validation plugin to reject nodes whose on-chain tenant membership does not match the Kubernetes namespace label.
3. Auditors scrape metrics only; route mutating API calls through GitOps with Operator group membership.

## Operational checklist

- [ ] IdP groups mapped: `stellar-platform-superadmins`, `stellar-tenant-{name}-operators`, `stellar-tenant-{name}-auditors`
- [ ] NetworkPolicy verified: cross-tenant pod-to-pod traffic blocked
- [ ] ResourceQuota sized for expected validator + Horizon footprint
- [ ] Prometheus RBAC allows Auditors read-only access to Grafana dashboards
- [ ] Soroban tenant contract deployed and admin keys in HSM or sealed secrets

## Related documentation

- [Pod Security Standards](../security/pss.md)
- [Network isolation](../network-isolation.md)
- [Metric reference](../observability/metric-reference.md)
- [Incident response](../operations/incident-response.md)
