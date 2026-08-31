# mTLS Setup and Certificate Rotation Guide

This guide explains how to enable mTLS for the operator, how node certificates are provisioned, and how to rotate certificates safely.

## Scope

This repository currently manages mTLS in two places:

- Operator REST API mTLS (server cert + CA, with automatic server cert rotation)
- StellarNode workload certs (per-node client cert secret, recreated on reconcile if missing, and
  now also automatically rolled when cert-manager rotates the secret — see
  [How Rotation Works](#how-rotation-works) below)
- Inter-service mTLS mesh between Stellar Core, Horizon, Soroban RPC, and companion services via cert-manager (issue #1281; see [End-to-End Encryption Architecture](security/e2e-encryption-architecture.md))

### Two mechanisms for inter-service mTLS — which one to use

There are two independent ways a `Certificate` can get created for inter-service mTLS in this
repo, and they are **not** the same mechanism:

1. **Per-`StellarNode` CR-driven flow (authoritative, this is what the operator actually
   reconciles)** — implemented in `src/controller/mtls.rs` and driven by
   `StellarNode.spec.certManager`. When set, the operator creates a cert-manager `Certificate`
   named `<node-name>-mtls-cert` targeting the Secret `<node-name>-client-cert` — the same Secret
   the pod already mounts at `/etc/stellar/tls`, and the same Secret the operator watches for
   rotation on every reconcile (see below). This is the supported, tested path.
2. **Static Helm chart template** (`charts/stellar-operator/templates/cert-manager-mtls.yaml`,
   gated by `.Values.mtls.enabled`) — creates a self-signed `Issuer` plus three `Certificate`
   resources with fixed names (`stellar-core-mtls-cert`, `horizon-mtls-cert`,
   `soroban-rpc-mtls-cert`) writing to Secrets `stellar-core-mtls-secret`,
   `horizon-mtls-secret`, `soroban-rpc-mtls-secret`. **No workload in this repository mounts
   these secret names or references them anywhere** — they are not the secrets StellarNode pods
   use. Enabling `.Values.mtls.enabled` produces cert-manager `Certificate` objects that nothing
   currently consumes; treat this template as a starting point for a hand-rolled setup outside
   the StellarNode CR flow, not as a working feature. If you want mTLS for a node, set
   `spec.certManager` on that `StellarNode` instead.

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

## Known limitation: stellar-core TLS termination is unverified

When mTLS is enabled, the ConfigMap for validator nodes
(`src/controller/resources.rs`) writes `HTTP_PORT_SECURE=true`, `TLS_CERT_FILE`, and
`TLS_KEY_FILE` into `stellar-core.cfg`. **These config keys have not been verified against a real
stellar-core build** — stellar-core's admin/HTTP endpoint does not have documented native HTTPS
termination in upstream stellar-core releases as of this writing. Treat this configuration as
best-effort/forward-looking rather than a confirmed working feature: it may be a no-op on your
stellar-core version, in which case the validator's HTTP endpoint continues to serve plaintext
even with `MTLS_ENABLED=true` set elsewhere. The `<node-name>-client-cert` material is still
correctly issued and mounted regardless; only the "does stellar-core itself terminate TLS on its
HTTP port" behavior is unconfirmed. If you need verified in-transit encryption for traffic to
stellar-core's HTTP port today, terminate TLS in front of it yourself (e.g. a sidecar or mesh
proxy) — this repository does not ship one; deliberately out of scope for this pass (see the
project tracking issue for #1392).

## Client trust failures after CA changes

- Ensure all leaf certs were reissued from the new CA.
- Ensure consumers trust the new `ca.crt`.
- Restart components holding old TLS material in memory.

## Security Recommendations

- Restrict read access to Secrets (`stellar-operator-ca`, server cert, node certs).
- Back up CA material in a secure secrets system before planned rotation.
- Prefer short cert lifetimes and scheduled rotation windows.
- Audit access to TLS secrets and operator logs.
