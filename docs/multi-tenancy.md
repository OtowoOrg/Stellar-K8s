# Multi-tenancy

The `Tenant` resource provisions an isolated namespace for one tenant. The
operator should apply the namespace labels, quota, and network policy derived
from the resource before reporting the tenant as ready.

## Onboarding

Apply the example in `examples/tenant-isolation.yaml`, replacing the tenant ID,
namespace, and quota values for the customer. The quota is applied to both
requests and limits for CPU and memory. A tenant owns exactly one namespace in
`v1alpha1`; use separate `Tenant` resources for separate isolation boundaries.

## Isolation guarantees

Each tenant namespace receives a stable `tenant.stellar.org/id` label and a
default-deny ingress/egress policy. Traffic is allowed only between namespaces
with the same tenant label. Cluster administrators remain responsible for
enforcing admission policies that prevent tenants from changing namespace
labels or bypassing network policy with privileged workloads.

Deletion cleanup is opt-in through `spec.cleanupOnDelete`. Keep it disabled when
the namespace or its data is managed by an external retention process.