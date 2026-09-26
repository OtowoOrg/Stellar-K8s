# mTLS Setup and Certificate Rotation Guide

This guide explains how to enable mTLS for the operator, how node certificates are provisioned, and how to rotate certificates safely.

## Scope

This repository supports Istio mesh mTLS and a separate application-certificate workflow:

- Inter-service pod-to-pod mTLS through Istio sidecars, enabled with Helm `mtls.enabled=true`.
- Operator and per-node application certificates, optionally rotated by cert-manager; these are
  independent of Istio proxy identities and do not themselves encrypt traffic.

### Enable Istio mesh mTLS

Prerequisite: Istio must be installed and its sidecar injection webhook available. Enable mesh
encryption with:

```bash
helm upgrade --install stellar-operator charts/stellar-operator \
  --namespace stellar-system --create-namespace --set mtls.enabled=true
```

This enables injection for the operator and managed StellarNode/read-pool pods, and applies a
STRICT `PeerAuthentication` only to resources managed by this operator. Do not enable the option
before Istio is ready: selected pods without sidecars will fail to communicate under STRICT mode.
See [End-to-End Encryption Architecture](security/e2e-encryption-architecture.md) for scope and
verification commands.

### Optional application-level certificates

Set `spec.certManager` on a `StellarNode` to delegate its optional application certificate to
cert-manager. The operator creates `<node-name>-mtls-cert` targeting `<node-name>-client-cert`,
which is mounted at `/etc/stellar/tls`. This certificate is not the identity used by Istio.

## Certificate and Secret Model

When mTLS is enabled, the operator manages these Kubernetes Secrets in the operator namespace:

- `stellar-operator-ca`
  - `tls.crt`: CA certificate
  - `tls.key`: CA private key
- `stellar-operator-server-cert`
  - `tls.crt`: operator REST API server certificate
  - `tls.key`: operator REST API server private key
  - `ca.crt`: CA certificate used for client trust

For each `StellarNode`, the operator also creates:

- `<node-name>-client-cert`
  - `tls.crt`
  - `tls.key`
  - `ca.crt`

The node workloads mount this secret at `/etc/stellar/tls` and use:

- `/etc/stellar/tls/tls.crt`
- `/etc/stellar/tls/tls.key`
- `/etc/stellar/tls/ca.crt`

## Prerequisites

- Running Kubernetes cluster
- Operator deployed in a namespace (examples below use `stellar-system`)
- `kubectl` access to that namespace
- REST API enabled (default in the chart)

## Enable mTLS

## Option A: CLI / local run

Run the operator with mTLS enabled:

```bash
stellar-operator run --namespace stellar-system --enable-mtls
```

Equivalent environment variable:

```bash
ENABLE_MTLS=true
```

## Option B: Kubernetes deployment

If your deployment does not already pass `--enable-mtls`, add it to the operator container args.

Example patch:

```bash
kubectl -n stellar-system patch deployment stellar-operator \
  --type='json' \
  -p='[
    {"op":"add","path":"/spec/template/spec/containers/0/args/-","value":"--enable-mtls"}
  ]'
```

If your deployment name differs, replace `stellar-operator` with the actual deployment name.

## Verify mTLS Provisioning

Check CA and server secrets:

```bash
kubectl -n stellar-system get secret stellar-operator-ca
kubectl -n stellar-system get secret stellar-operator-server-cert
```

Check data keys:

```bash
kubectl -n stellar-system get secret stellar-operator-server-cert -o jsonpath='{.data}'
```

You should see `tls.crt`, `tls.key`, and `ca.crt`.

Check node certificate secret (for a node named `validator-1`):

```bash
kubectl -n stellar-system get secret validator-1-client-cert
```

## How Rotation Works

## Operator server certificate rotation

- The operator checks server cert expiry hourly.
- Rotation threshold is controlled by `CERT_ROTATION_THRESHOLD_DAYS`.
- Default threshold is `30` days.
- When rotation happens, the operator reloads in-memory TLS config without full process restart.

Set custom threshold:

```bash
kubectl -n stellar-system set env deployment/stellar-operator CERT_ROTATION_THRESHOLD_DAYS=14
```

## Node certificate behavior

- Per-node certs are ensured on reconcile.
- If a `<node-name>-client-cert` secret is missing, reconcile recreates it (self-signed, via
  `mtls::ensure_node_cert`).
- The operator itself does not proactively rotate a self-signed `<node-name>-client-cert` on a
  timer — it only regenerates the secret if it is deleted.
- If `spec.certManager` is set on the `StellarNode`, cert-manager owns issuance and rotation of
  `<node-name>-client-cert` instead (via the `Certificate` CR the operator creates). **On every
  reconcile the operator now checks whether that Secret's `resourceVersion` changed since the
  previous reconcile** (`mtls::check_and_restart_on_cert_rotation`, called from the reconcile loop
  right after `ensure_node_cert`/`ensure_cert_manager_certificate`). If it changed — meaning
  cert-manager rotated the certificate — the operator bumps a `stellar.org/cert-rotated-at`
  annotation on the workload's pod template (StatefulSet for validators, Deployment for
  Horizon/Soroban RPC), which Kubernetes uses to trigger a rolling restart so pods pick up the
  new certificate. This is what makes "certificates rotate without downtime" actually true today:
  rotation happens through a rolling restart (old pods keep serving on their still-valid
  certificate until replaced one at a time), not a live in-process reload.
  - The rotation-detection state (last-seen resourceVersion per node) is kept in the operator
    process's memory. On operator restart it starts empty, so the very first reconcile after a
    restart will not trigger a restart even if the cert had rotated earlier — the *next* rotation
    after that will be caught normally. This is a deliberate, safe default (see the doc comment on
    `maybe_restart_on_cert_rotation` in `src/controller/mtls.rs`), not a residual bug.

## Manual Rotation Runbooks

## Rotate operator server certificate now

Delete only the server cert secret; keep CA unchanged:

```bash
kubectl -n stellar-system delete secret stellar-operator-server-cert
```

Then restart operator pod (or wait for reconciliation/startup logic to recreate it):

```bash
kubectl -n stellar-system rollout restart deployment/stellar-operator
kubectl -n stellar-system rollout status deployment/stellar-operator
```

## Rotate a node certificate now

For node `validator-1`:

```bash
kubectl -n stellar-system delete secret validator-1-client-cert
```

Trigger reconcile by touching the node annotation:

```bash
kubectl -n stellar-system annotate stellarnode validator-1 mtls.rotate-ts="$(date +%s)" --overwrite
```

Confirm secret recreation:

```bash
kubectl -n stellar-system get secret validator-1-client-cert
```

## Rotate the CA (full trust rollover)

CA rotation invalidates all certificates issued by the old CA. Plan a maintenance window.

Suggested sequence:

1. Scale down workloads that depend on strict mutual trust.
2. Delete CA, server cert, and node cert secrets.
3. Restart operator so it recreates CA/server cert.
4. Trigger reconcile for all `StellarNode` resources so node certs are recreated.
5. Scale workloads back up and verify health.

Commands:

```bash
kubectl -n stellar-system delete secret stellar-operator-ca stellar-operator-server-cert
kubectl -n stellar-system delete secret -l app.kubernetes.io/managed-by=stellar-operator
kubectl -n stellar-system rollout restart deployment/stellar-operator
kubectl -n stellar-system rollout status deployment/stellar-operator
```

If your node cert secrets do not carry a reliable label selector, delete them by explicit name (`<node>-client-cert`) instead.

## Validation Checklist

- Operator pod is `Running` and ready.
- `stellar-operator-ca` exists with `tls.crt` and `tls.key`.
- `stellar-operator-server-cert` exists with `tls.crt`, `tls.key`, `ca.crt`.
- Each managed `StellarNode` has `<node-name>-client-cert`.
- Node pods have mounted `/etc/stellar/tls` volume.
- REST API and node endpoints continue to pass readiness/liveness checks.

## Troubleshooting

## Missing `ca.crt` / `tls.crt` / `tls.key`

- Recreate the affected secret by deleting it and triggering reconcile.
- Check operator logs for certificate generation errors.

```bash
kubectl -n stellar-system logs deploy/stellar-operator --tail=200
```

## Rotation not happening

- Verify `ENABLE_MTLS=true`.
- Verify `CERT_ROTATION_THRESHOLD_DAYS` value.
- Confirm the running leader instance is healthy (rotation runs on the leader path).
- For node certs specifically: rotation-triggered restarts only happen for nodes with
  `spec.certManager` configured (cert-manager owns rotation). Self-signed
  `<node-name>-client-cert` secrets are not rotated on a timer at all — see
  [Node certificate behavior](#node-certificate-behavior).
- If the operator process restarted recently, the first reconcile after restart cannot detect a
  rotation that happened before the restart (the in-memory "last known resourceVersion" cache is
  empty). Wait for the next actual rotation, or check `kubectl -n stellar-system get secret
  <node-name>-client-cert -o jsonpath='{.metadata.resourceVersion}'` before and after a manual
  `cert-manager` renewal to confirm the Secret itself is changing.

## Mesh mTLS and application certificates

Istio encrypts traffic between injected pod proxies and manages their identities. The
`<node-name>-client-cert` and `stellar-operator-server-cert` are separate application-level
certificates. Stellar Core's native HTTPS settings remain version-dependent; do not rely on them
for pod-to-pod encryption. Mesh mode covers selected in-cluster workload traffic, not loopback
traffic or external endpoints.

## Client trust failures after CA changes

- Ensure all leaf certs were reissued from the new CA.
- Ensure consumers trust the new `ca.crt`.
- Restart components holding old TLS material in memory.

## Security Recommendations

- Restrict read access to Secrets (`stellar-operator-ca`, server cert, node certs).
- Back up CA material in a secure secrets system before planned rotation.
- Prefer short cert lifetimes and scheduled rotation windows.
- Audit access to TLS secrets and operator logs.
