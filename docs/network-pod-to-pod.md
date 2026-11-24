# Pod-to-Pod Network Segmentation

This document describes the default-deny NetworkPolicies that control **pod-to-pod
communication** within the Stellar-K8s operator's namespaces, the required service
links that are explicitly allowed, and the rationale for each rule (issue #1320).

The model is **zero-trust**: every namespace controlled by the chart starts from a
default-deny posture, and only the specific endpoints a workload must reach are
enumerated as explicit allow rules. A matching allow rule supersedes the default-deny
for exactly that path; everything else remains blocked.

---

## Policy Inventory

| Policy | Template | Namespaces applied to | policyTypes |
|--------|----------|----------------------|-------------|
| `stellar-operator-default-deny` | `templates/networkpolicy.yaml` | Release namespace (e.g. `stellar-system`) | Ingress + Egress |
| `stellar-webhook-default-deny` | `templates/networkpolicy.yaml` | `stellar-webhook` | Ingress + Egress |
| `default-deny` (per namespace) | `templates/network-pod-policy.yaml` | Each entry in `security.networkPolicy.defaultDenyNamespaces` | Ingress + Egress |
| `deny-non-<network>-ingress/egress` | `templates/network-isolation.yaml` | Mainnet/Testnet namespaces (when `networkIsolation.enabled`) | Ingress / Egress |
| per-node policies | reconciler (`build_network_policy`) | Each `StellarNode` pod | Ingress + Egress |

> The default deny is applied to the release namespace and webhook namespace by
> `networkpolicy.yaml` unconditionally (when `security.networkPolicy.enabled`).
> Additional namespaces that host Stellar nodes or supporting services can be added
> to `security.networkPolicy.defaultDenyNamespaces` to receive the same baseline:

```yaml
security:
  networkPolicy:
    enabled: true
    defaultDenyNamespaces:
      - stellar
      - stellar-mainnet
      - stellar-testnet
```

---

## Required Service-to-Service Communication

The following table lists every **explicitly allowed** pod-to-pod edge that the operator
needs to function. Each row corresponds to an allow rule emitted under the operator's
default-deny egress, conditioned on the feature being enabled.

### Operator (release namespace)

| Destination | Port(s) | Protocol | When allowed | Value to enable |
|-------------|---------|----------|--------------|-----------------|
| Kubernetes API server (`kube-system`) | 443 | TCP | always | — |
| Cluster DNS (`kube-system`/kube-dns) | 53 | TCP/UDP | always | — |
| Prometheus (`monitoring`) — ingress for metrics scrape | 9090 | TCP | always (ingress) | `security.networkPolicy.enabled` |
| Rate-limit Redis (same namespace) | 6379 | TCP | distributed rate limiting | `rateLimiting.distributed.enabled` |
| Vault PKI (same namespace) | 8200 | TCP | cert management via Vault | `certManagement.backend: vault-pki` |
| OTel collector (same namespace) | 4317, 4318 | TCP | OTLP trace export | `otel.collector.enabled` |
| Kafka bootstrap (namespace `kafka`) | 9092, 9093 | TCP | SCP analytics | `scpAnalytics.enabled` |

### Admission webhook (`stellar-webhook`)

| Destination | Port(s) | Protocol | When allowed |
|-------------|---------|----------|--------------|
| Kubernetes API server (ingress, HTTPS calls) | 443 | TCP | always |
| Prometheus (`monitoring`) — ingress metrics scrape | 9090 | TCP | always |
| Kubernetes API server (egress) | 443 | TCP | always |
| Cluster DNS | 53 | TCP/UDP | always |

### Per-namespace `default-deny` baseline

For each namespace in `security.networkPolicy.defaultDenyNamespaces`, the baseline
allows only:

| Direction | Destination | Port(s) | Protocol | Purpose |
|-----------|-------------|---------|----------|---------|
| Ingress | same-namespace pods | any | — | intra-namespace pod links (further constrained by per-node policies) |
| Egress | kube-dns | 53 | TCP/UDP | service discovery |
| Egress | kube-apiserver | 443 | TCP | reconciliation / status |
| Egress | `monitoring` | any | — | push metrics to Prometheus |

---

## Rationale

1. **Least privilege.** A compromised pod can only reach the few endpoints it
   legitimately uses. Horizontal movement across namespaces or between unrelated
   services is denied at the kernel (iptables/ebpf) layer, independent of
   application authentication.

2. **Auditability.** Every required edge is listed explicitly in one place, so the
   runtime network dependencies of the operator are a reviewable artifact rather
   than an implicit consequence of Service definitions.

3. **Defence in depth.** The per-node policies (reconciler), namespace isolation
   policies, and these default-deny baselines are independent layers. If one is
   misconfigured or deleted, the others still constrain traffic.

4. **Fail-closed.** Because `podSelector: {}` with `policyTypes: [Ingress, Egress]`
   is applied per namespace, any new pod or service added later is denied by default
   until an explicit allow rule is introduced.

---

## Implementation Notes (Helm)

- All policies are gated by `security.networkPolicy.enabled` (default `true`).
- The explicit allow rules in `networkpolicy.yaml` are emitted **conditionally**, so
  the rendered output only contains rules that match the deployed feature set. This
  keeps the allow-list as small as possible and the golden/render drift low.
- Rules targeting external dependencies whose pod labels are not owned by the chart
  (Redis, Vault, Kafka) match by `namespaceSelector` only, scoped to the namespace
  where the service resides.
- Selector labels follow `stellar-operator.selectorLabels` (`app.kubernetes.io/name` /
  `app.kubernetes.io/instance`) plus `app.kubernetes.io/component` for sidecars such as
  the OTel collector.

---

## Related Documents

- [Zero-Trust Network Policies for Stellar Node Isolation](network-policy-zero-trust.md)
- [Network Isolation Architecture](network-isolation.md)
- [Network Policy Templates](network-policy-templates.md)
- [Network Topology Management](network-topology-management.md)
