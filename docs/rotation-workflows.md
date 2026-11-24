# Secrets and Certificate Rotation Workflows

This document is the central reference for all rotation workflows in Stellar-K8s.
It covers when and how to rotate each type of credential or certificate managed by the operator.

## Overview

| Credential | Rotation method | Frequency | Guide |
|---|---|---|---|
| Database passwords (Horizon / Core) | Automated (cron-based) | Quarterly or monthly | [Secret Rotation](secret-rotation.md) |
| Operator REST API server certificate | Automated (hourly check) | When within threshold | [mTLS Guide](mtls-guide.md) |
| StellarNode client certificates | On-demand (delete secret) | As needed | [mTLS Guide](mtls-guide.md) |
| CA certificate | Manual (maintenance window) | Annually or on compromise | [CA Rotation](#ca-certificate-rotation) |
| Webhook TLS certificate | Automated (cert-manager) | cert-manager managed | [Webhook Cert Rotation](#webhook-tls-certificate-rotation) |
| Network passphrase secret | Manual or on Secret change | On network config change | [Passphrase Rotation](#network-passphrase-secret-rotation) |
| Validator seed (`STELLAR_SEED`) | Manual | Per security policy | [Credentials Reference](security/credentials-and-secrets.md) |

---

## Database Credential Rotation

Database password rotation is fully automated when `secretRotation.enabled: true` is set on a
`StellarNode` resource. See [Secret Rotation](secret-rotation.md) for complete configuration
details, cron schedules, rollback behavior, and Prometheus metrics.

**Quick enable:**

```yaml
spec:
  secretRotation:
    enabled: true
    schedule: "0 0 1 */3 *"   # quarterly
    passwordLength: 32
    auditLoggingEnabled: true
```

---

## mTLS Certificate Rotation

The operator issues and rotates two classes of TLS certificate when `--enable-mtls` is active:

- **Operator REST API server certificate** — checked hourly and rotated if expiry is within
  `CERT_ROTATION_THRESHOLD_DAYS` (default `30`). Rotation reloads the in-memory TLS config
  without a process restart using the dual-key strategy.
- **StellarNode client certificates** — recreated on reconcile if the secret is missing;
  proactive rotation is triggered by deleting the `<node-name>-client-cert` secret.

See [mTLS Setup and Certificate Rotation Guide](mtls-guide.md) for the full runbook including
CA rotation and troubleshooting steps.

**Tune the rotation threshold:**

```bash
kubectl -n stellar-system set env deployment/stellar-operator \
  CERT_ROTATION_THRESHOLD_DAYS=14
```

---

## CA Certificate Rotation

CA rotation invalidates all leaf certificates. Plan a maintenance window and follow the steps
documented in [mTLS Guide — Rotate the CA](mtls-guide.md#rotate-the-ca-full-trust-rollover).

---

## Webhook TLS Certificate Rotation

The admission webhook (`stellar-webhook`) uses cert-manager to provision and auto-renew its
TLS certificate. The cert-manager `Certificate` resource references `selfsigned-issuer` (or
your production `ClusterIssuer`) and cert-manager handles renewal automatically before expiry.

### Verify current certificate status

```bash
kubectl -n stellar-webhook get certificate stellar-webhook-cert -o wide
kubectl -n stellar-webhook describe certificate stellar-webhook-cert
```

A healthy certificate shows `Ready` in the `READY` column.

### Force immediate renewal

```bash
kubectl -n stellar-webhook delete secret stellar-webhook-certs
```

cert-manager detects the missing Secret and issues a new certificate within seconds.
The webhook pods pick up the new TLS material without a restart because the Secret is
mounted as a volume and Kubernetes refreshes mounted Secret volumes periodically
(default every `60s`–`2m`).

If pods do not reload the updated certificate within ~2 minutes, trigger a rollout:

```bash
kubectl -n stellar-webhook rollout restart deployment/stellar-webhook
kubectl -n stellar-webhook rollout status deployment/stellar-webhook
```

### Rotate to a production issuer

Replace `selfsigned-issuer` in `webhook.yaml` (or via Helm values) with a `ClusterIssuer`
backed by your PKI:

```yaml
# In your Helm values override:
# (no built-in value exists — patch the Certificate manifest directly)
```

Edit the `Certificate` resource:

```bash
kubectl -n stellar-webhook edit certificate stellar-webhook-cert
# Change: issuerRef.name: selfsigned-issuer → your-production-issuer
# Change: issuerRef.kind: ClusterIssuer  → ClusterIssuer (or Issuer)
```

Update the `ValidatingWebhookConfiguration` CA bundle after issuer change:

```bash
# cert-manager injects the CA automatically via the annotation:
# cert-manager.io/inject-ca-from: stellar-webhook/stellar-webhook-cert
# No manual caBundle update is needed when cert-manager is managing the webhook.
```

---

## Network Passphrase Secret Rotation

The `StellarNode` CRD supports referencing a Kubernetes Secret for the network passphrase
via `spec.passphrase_secret_ref`. This allows rotating the passphrase without editing the
`StellarNode` resource directly.

### Using a passphrase Secret

```yaml
apiVersion: stellar.io/v1alpha1
kind: StellarNode
metadata:
  name: my-validator
  namespace: stellar-system
spec:
  network: Custom
  passphrase_secret_ref: my-network-passphrase   # name of a Secret in the same namespace
```

The referenced Secret must contain the key `NETWORK_PASSPHRASE`:

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: my-network-passphrase
  namespace: stellar-system
type: Opaque
stringData:
  NETWORK_PASSPHRASE: "My Custom Network ; July 2026"
```

### Rotating the passphrase

1. Create or update the Secret with the new passphrase value:

   ```bash
   kubectl -n stellar-system create secret generic my-network-passphrase \
     --from-literal=NETWORK_PASSPHRASE="My Custom Network ; August 2026" \
     --dry-run=client -o yaml | kubectl apply -f -
   ```

2. The operator detects the Secret's resource version change via `observed_passphrase_secret_version`
   in the `StellarNode` status and triggers reconciliation automatically.

3. Verify reconciliation completed:

   ```bash
   kubectl -n stellar-system get stellarnode my-validator -o jsonpath='{.status.conditions}'
   ```

4. Check that the updated passphrase is reflected in the pod environment:

   ```bash
   kubectl -n stellar-system exec deploy/my-validator -- \
     printenv NETWORK_PASSPHRASE
   ```

### RBAC for passphrase secrets

Ensure the operator ServiceAccount can read the passphrase secret:

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: stellar-operator-passphrase
  namespace: stellar-system
rules:
  - apiGroups: [""]
    resources: ["secrets"]
    resourceNames: ["my-network-passphrase"]
    verbs: ["get", "watch"]
```

---

## Rotation Monitoring

All automated rotation events are recorded in the operator audit log. Query them with:

```bash
kubectl logs -n stellar-system \
  -l app=stellar-operator \
  --tail=200 | grep -E "rotation|AUDIT|rotate"
```

Prometheus metrics for rotation operations:

```
stellar_operator_secret_rotations_total
stellar_operator_secret_rotation_duration_seconds
stellar_operator_secret_rotation_last_success_timestamp
```

---

## Related Documentation

- [Secret Rotation](secret-rotation.md) — automated database credential rotation
- [mTLS Guide](mtls-guide.md) — mTLS setup and certificate rotation runbooks
- [Credentials and Secrets](security/credentials-and-secrets.md) — central secret reference
- [Secret Management Guide](secret-management-guide.md) — secret storage strategies
- [Secret Management with KMS](secret-management-kms.md) — AWS/Azure/GCP KMS integration
