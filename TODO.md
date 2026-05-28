# TODO: Multi-tenancy support (Stellar-K8s)

## Step 1 — Repo inspection (done)

- Reviewed existing `src/controller/network_isolation.rs` and Helm templates (`charts/stellar-operator/templates/network-isolation.yaml`, `rbac.yaml`, existing CRDs).

## Step 2 — Tenant CRD + status design (in progress)

- Added Rust CRD types: `src/crd/tenant.rs`.
- Exposed CRD module in `src/crd/mod.rs`.
- Added Helm CRDs: `charts/stellar-operator/templates/crd-tenant.yaml` (Tenant + TenantUsage).

## Step 3 — Tenant onboarding/offboarding controller (pending)

- New controller to create/label namespaces and apply tenant-scoped isolation objects.
- Add finalizers for safe cleanup.

## Step 4 — Resource quota enforcement controller (pending)

- New controller to create/update `ResourceQuota` (and optional `LimitRange`) per tenant namespace.

## Step 5 — Tenant-specific RBAC controller (pending)

- New controller to create tenant Roles/RoleBindings and tenant service accounts.

## Step 6 — Tenant network isolation controller (pending)

- New controller (or extend existing patterns) to enforce tenant isolation via `NetworkPolicy`.

## Step 7 — Usage tracking + billing metrics (pending)

- New controller/collector to aggregate resource usage into a `TenantUsage` CRD.

## Step 8 — Admin dashboard (pending)

- Extend dashboard handlers/UI to list/manage tenants.

## Step 9 — Wiring + documentation (pending)

- Wire controllers into operator main loop.
- Update chart values/docs with tenant configuration knobs.
