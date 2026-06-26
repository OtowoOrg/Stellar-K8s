# mTLS Setup and Certificate Rotation Guide

This guide explains how to enable mTLS for the operator, how node certificates are provisioned, how to rotate certificates safely, and how to manage webhook TLS certificates.

## Scope

This repository currently manages TLS in three places:

- Operator REST API mTLS (server cert + CA, with automatic server cert rotation)
- StellarNode workload certs (per-node client cert secret, recreated on reconcile if missing)
- Admission webhook TLS certificates (cert-manager-managed, with automatic renewal)

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
- Existing node cert secrets are not proactively rotated by a timer.
- If a `<node-name>-client-cert` secret is missing, reconcile recreates it.

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

## Client trust failures after CA changes

- Ensure all leaf certs were reissued from the new CA.
- Ensure consumers trust the new `ca.crt`.
- Restart components holding old TLS material in memory.

## Webhook TLS Certificates

### Overview

The admission webhook uses TLS to secure communication with the Kubernetes API server. Certificates are managed by cert-manager, which provides automatic renewal.

### Certificates Managed by cert-manager

In the `stellar-webhook` namespace:
- `stellar-webhook-certs`: Kubernetes Secret containing the webhook's TLS certificate and private key (managed by cert-manager's Certificate resource).
- `selfsigned-issuer`: Default ClusterIssuer for development (self-signed CA). For production, use a trusted internal or public CA.
- `stellar-webhook-cert`: Certificate custom resource that defines the webhook's certificate properties.

### Certificate Properties

The Certificate resource includes these DNS names for the webhook service:
- `stellar-webhook`
- `stellar-webhook.stellar-webhook`
- `stellar-webhook.stellar-webhook.svc`
- `stellar-webhook.stellar-webhook.svc.cluster.local`

### Automatic Renewal

cert-manager automatically renews certificates when they are within 30 days of expiry (default). This can be configured in the Certificate spec with `renewBefore`.

### Manual Rotation

If you need to rotate the webhook certificate manually:

1. Delete the existing certificate secret:
   ```bash
   kubectl delete secret stellar-webhook-certs -n stellar-webhook
   ```

2. cert-manager will automatically issue a new certificate and update the secret.

3. Verify the new certificate:
   ```bash
   kubectl get secret stellar-webhook-certs -n stellar-webhook -o jsonpath='{.data.tls\.crt}' | base64 -d | openssl x509 -noout -text
   ```

### Using a Custom Issuer

For production, replace the self-signed issuer with your own trusted CA:

1. Create a ClusterIssuer or Issuer for your CA (see [cert-manager documentation](https://cert-manager.io/docs/configuration/)).
2. Update the webhook Certificate's `issuerRef` to use your custom issuer.

### Troubleshooting Webhook Certificates

- **Certificate not issued**: Check cert-manager logs:
  ```bash
  kubectl logs -n cert-manager -l app=cert-manager
  ```
- **Kubernetes API server cannot connect to webhook**: Verify the `caBundle` in ValidatingWebhookConfiguration is correct (cert-manager injects this automatically).
- **Certificate expired**: Delete the secret to trigger immediate renewal.

## Security Recommendations

- Restrict read access to Secrets (`stellar-operator-ca`, server cert, node certs, `stellar-webhook-certs`).
- Back up CA material in a secure secrets system before planned rotation.
- Prefer short cert lifetimes and scheduled rotation windows.
- Audit access to TLS secrets and operator logs.
- For production, use a trusted internal CA instead of the default self-signed issuer for webhook certificates.
