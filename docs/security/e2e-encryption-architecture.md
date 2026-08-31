# End-to-End Inter-Service Encryption Architecture & Certificate Management

This document defines the Zero-Trust End-to-End (E2E) Encryption architecture for inter-service communication across Stellar Core, Horizon, Soroban RPC, and companion services within the `Stellar-K8s` ecosystem (issue #1281).

---

## 1. Zero-Trust Networking Model

In accordance with modern security standards, all network traffic traversing cluster nodes or pod boundaries must be encrypted in transit using Mutual TLS (mTLS).

```text
                                 ┌───────────────────────┐
                                 │   Stellar Operator    │
                                 └───────────┬───────────┘
                                             │ (cert-manager CRDs / Vault PKI)
                                             ▼
 ┌──────────────────────┐   mTLS    ┌──────────────────────┐   mTLS    ┌──────────────────────┐
 │     Stellar Core     │ ◄───────► │       Horizon        │ ◄───────► │     Soroban RPC      │
 └──────────────────────┘           └──────────────────────┘           └──────────────────────┘
            ▲                                                                      ▲
            │                                 mTLS                                 │
            └──────────────────────────────────────────────────────────────────────┘
```

---

## 2. Certificate Authority Hierarchy & cert-manager Integration

Certificates for inter-service mTLS are issued by `cert-manager`, driven from the
**per-`StellarNode` custom resource**, not by a fixed set of names. This is implemented in
`src/controller/mtls.rs` (`ensure_cert_manager_certificate`) and activated by setting
`spec.certManager` on a `StellarNode`:

```yaml
spec:
  certManager:
    issuerRef:
      name: stellar-inter-service-ca
      kind: ClusterIssuer   # or Issuer
      group: cert-manager.io
    duration: 2160h
    renewBefore: 720h
```

### Key Components:
- **`Issuer` / `ClusterIssuer`**: any cert-manager issuer you configure and reference via
  `spec.certManager.issuerRef` — this repo does not create one for you automatically for the
  per-node flow (bring your own issuer, e.g. an internal CA `Issuer` or a Vault PKI issuer).
- **`Certificate` resource**: named `<node-name>-mtls-cert`, targeting the Secret
  `<node-name>-client-cert` — the same Secret the pod mounts at `/etc/stellar/tls`.
- **Key Parameters** (defaults if unset): duration and renew-before are whatever you set in
  `spec.certManager.duration` / `renewBefore`; there is no repo-wide fixed 90-day/15-day default
  enforced by the operator itself (cert-manager applies its own defaults if you omit them).

> **A second, separate mechanism exists and is easy to confuse with the above:** the Helm chart
> template `charts/stellar-operator/templates/cert-manager-mtls.yaml` (gated by
> `.Values.mtls.enabled`) creates a self-signed `Issuer` named `stellar-inter-service-ca` plus
> three fixed `Certificate` resources (`stellar-core-mtls-cert`, `horizon-mtls-cert`,
> `soroban-rpc-mtls-cert`) writing to Secrets `stellar-core-mtls-secret`, `horizon-mtls-secret`,
> `soroban-rpc-mtls-secret`. **No StellarNode workload in this repository mounts those secret
> names.** Enabling it produces `Certificate` objects that nothing currently consumes. The
> per-`StellarNode` flow described above is the one actually wired into pod volumes and into
> rotation-triggered restarts (§3) — treat the Helm template as an unfinished/manual-integration
> starting point, not a working feature, until something mounts its output secrets.

---

## 3. Certificate Rotation & Restart Behavior

To keep pods in sync with rotated certificates:
1. `cert-manager` rotates `<node-name>-client-cert` in place when its `renewBefore` window is
   reached (per your `spec.certManager` configuration).
2. On every reconcile, the operator compares that Secret's `resourceVersion` against the value it
   observed on the previous reconcile (`mtls::check_and_restart_on_cert_rotation` in
   `src/controller/reconciler.rs`, called right after certificate issuance). This state is kept in
   the operator process's memory, not in the cluster, so it resets on operator restart (the next
   reconcile after a restart will not fire a restart for a rotation that already happened, but
   will catch the next one normally).
3. If the resourceVersion changed, the operator patches a `stellar.org/cert-rotated-at`
   annotation on the pod template of the owning StatefulSet (validators) or Deployment
   (Horizon/Soroban RPC). Kubernetes performs a standard rolling restart in response, so pods pick
   up the new certificate from the mounted Secret one at a time, with no downtime window where
   the whole workload is unavailable at once.
4. There is **no live, in-process certificate reload** — services do not watch the mounted volume
   and swap the TLS context in memory. Rotation takes effect via pod replacement, not hot reload.
   Treat "watch mounted TLS secret volumes" as future work, not current behavior.

### Certificate expiry monitoring — not yet wired

`src/security/cert_rotation.rs` contains a real, unit-tested `ExpiryMonitor` that can classify
certificates into warning/critical/emergency buckets and render a `stellar_cert_expiry_days`
Prometheus gauge line (`ExpiryMonitor::render_prometheus`), plus a `CertRotationController` that
can drive renewal against a pluggable `PkiBackend` (a real Vault PKI HTTP client, and a real
`rcgen`-based local CA backend). **None of this is currently invoked from the reconcile loop or
exposed on any metrics endpoint** — it is tested, working logic that nothing in the running
operator calls yet. If you need certificate-expiry alerting today, monitor the cert-manager
`Certificate` resources' own `status.conditions` and cert-manager's own Prometheus metrics
instead (`kubectl get certificate -o yaml`, or cert-manager's `certmanager_certificate_expiration_timestamp_seconds`
metric if cert-manager's Prometheus integration is enabled).

---

## 4. Known Limitation: stellar-core TLS Termination Is Unverified

When mTLS is enabled, the ConfigMap for validator nodes writes `HTTP_PORT_SECURE=true`,
`TLS_CERT_FILE`, and `TLS_KEY_FILE` into `stellar-core.cfg` (`src/controller/resources.rs`).
These config keys have **not been verified against a real stellar-core build** — stellar-core's
admin/HTTP endpoint does not have documented native HTTPS termination in upstream releases as of
this writing. This configuration may be a no-op depending on your stellar-core version. Node
client certificates (`<node-name>-client-cert`) are still correctly issued and mounted regardless
of this caveat; only whether stellar-core itself terminates TLS on its HTTP port is unconfirmed.
A sidecar/mesh TLS-termination proxy in front of stellar-core would close this gap but is
explicitly out of scope for this pass (no live cluster available to validate it against).

---

## 5. Verification & Diagnostics

Verify mTLS secret creation and cert-manager status for a node named `horizon-1`:

```bash
# Check cert-manager Certificates
kubectl -n stellar-system get certificate

# Inspect the node's TLS secret (created by cert-manager when spec.certManager is set,
# or by the operator's self-signed fallback otherwise)
kubectl -n stellar-system get secret horizon-1-client-cert -o yaml

# Confirm a rotation was detected and a restart was triggered
kubectl -n stellar-system get statefulset|deployment horizon-1 \
  -o jsonpath='{.spec.template.metadata.annotations.stellar\.org/cert-rotated-at}'
```
