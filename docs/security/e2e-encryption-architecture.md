# End-to-End Inter-Service Encryption Architecture & Certificate Management

This document describes the supported pod-to-pod encryption path for Stellar Core, Horizon, Soroban RPC, and read-pool workloads managed by Stellar-K8s.

---

## 1. Zero-Trust Networking Model

When Istio is installed, setting Helm `mtls.enabled=true` enables sidecar injection for the operator and its managed node workloads, and creates a selector-scoped `PeerAuthentication` in `STRICT` mode. Istio proxies mutually authenticate and encrypt pod-to-pod traffic. Traffic inside a pod, to non-injected workloads, and to external endpoints is not covered by this policy.

```text
Stellar Core pod      Horizon pod      Soroban RPC pod
[app | Envoy] <==== Istio mTLS ====> [app | Envoy]
                     STRICT policy
```

---

## 2. Mesh enablement and identity

Install Istio and ensure its sidecar injection webhook is available, then enable the chart option:

```bash
helm upgrade --install stellar-operator charts/stellar-operator --namespace stellar-system --set mtls.enabled=true
```

The chart passes `--enable-mtls` to the operator, injects its sidecar, and applies
`PeerAuthentication/stellar-node-mtls` in `STRICT` mode. The policy selects pods labeled
`stellar.org/mtls-mode=strict`; the reconciler adds injection to main and read-pool workloads.
Istio provisions and rotates proxy identities. The cert-manager certificates
described below are optional application-level certificates, not mesh identities.

### Optional application-level certificates

Certificates for application-level use are issued by `cert-manager`, driven from the
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

> The former static Helm certificate template was removed because no managed workload consumed its
> output Secrets. `.Values.mtls.enabled` now configures Istio injection and STRICT peer
> authentication; it does not create cert-manager Certificates.

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

## 4. Application TLS limitation

When mTLS is enabled, the ConfigMap for validator nodes writes `HTTP_PORT_SECURE=true`,
`TLS_CERT_FILE`, and `TLS_KEY_FILE` into `stellar-core.cfg` (`src/controller/resources.rs`).
These config keys have **not been verified against a real stellar-core build** — stellar-core's
admin/HTTP endpoint does not have documented native HTTPS termination in upstream releases as of
this writing. This configuration may be a no-op depending on your stellar-core version. Node
client certificates (`<node-name>-client-cert`) are still correctly issued and mounted regardless
of this caveat; only whether stellar-core itself terminates TLS on its HTTP port is unconfirmed.
Istio sidecars provide the pod-to-pod encryption layer independently of native stellar-core TLS.
The Istio control plane and injection webhook must be healthy before STRICT mode is enabled.

---

## 5. Verification & Diagnostics

Verify the mesh policy and injected proxies:

```bash
kubectl -n stellar-system get peerauthentication.security.istio.io stellar-node-mtls -o yaml
kubectl -n stellar-system get pod -l stellar.org/mtls-mode=strict \\
  -o jsonpath='{range .items[*]}{.metadata.name}{"\t"}{.spec.containers[*].name}{"\n"}{end}'
```

Optional application certificate issuance for `horizon-1` can be checked separately:

```bash
kubectl -n stellar-system get certificate horizon-1-mtls-cert
kubectl -n stellar-system get secret horizon-1-client-cert -o yaml
```
