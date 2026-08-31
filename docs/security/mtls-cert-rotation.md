# mTLS Certificate Rotation Automation

This document describes automated certificate rotation for inter-service mTLS between Stellar Core, Horizon, and Soroban RPC using cert-manager.

## Architecture

- **Issuer**: `stellar-inter-service-ca` (self-signed) issues leaf certs.
- **Certificates**: `stellar-core-mtls-cert`, `horizon-mtls-cert`, `soroban-rpc-mtls-cert`
- **Secrets**: `stellar-core-mtls-secret`, `horizon-mtls-secret`, `soroban-rpc-mtls-secret`
- **Duration**: 90 days (`2160h`) with renewal 15 days before expiry (`360h`)
- **Mounts**: `/etc/stellar/tls`, `/etc/horizon/tls`, `/etc/soroban/tls`

## Zero-Downtime Rotation

1. cert-manager watches `renewBefore: 360h` and creates a new cert 15 days before expiry.
2. Secrets are updated in place; kubelet syncs volume mounts within ~60s.
3. Pods with `reloader.stakater.com/auto: "true"` are rolled automatically, or reload in-process via `RustlsConfig::reload_from_config`.
4. Prometheus metric `stellar_cert_expiry_days` alerts:
   - 30 days: warning
   - 7 days: critical
   - 1 day: emergency

## Enable mTLS

```yaml
# values.yaml
mtls:
  enabled: true
```

```bash
helm upgrade --install stellar-operator charts/stellar-operator --set mtls.enabled=true -n stellar-system
```

## Verify Rotation

```bash
# Check certificates
kubectl -n stellar-system get certificate
kubectl -n stellar-system describe certificate stellar-core-mtls-cert

# Check expiry
kubectl -n stellar-system get secret stellar-core-mtls-secret -o jsonpath='{.data.tls\.crt}' | base64 -d | openssl x509 -noout -dates

# Force rotation test (delete secret, cert-manager recreates)
kubectl -n stellar-system delete secret stellar-core-mtls-secret
kubectl -n stellar-system get secret stellar-core-mtls-secret --watch
```

## Manual Rotation

```bash
# Rotate without downtime — cert-manager handles automatically
# To force immediate renewal:
kubectl -n stellar-system patch certificate stellar-core-mtls-cert --type merge -p '{"spec":{"renewBefore":"1h"}}'
# Or trigger via cmctl:
cmctl renew stellar-core-mtls-cert -n stellar-system
```

## Monitoring

- Grafana dashboard: `Certificate Expiry`
- Alert: `CertExpiryWarning` fires when `stellar_cert_expiry_days < 30`
- Logs: operator emits `cert_rotation` structured log with `correlation_id`

## Troubleshooting

- Secret not created: `kubectl describe certificate <name>` shows issuer errors.
- Pod not reloading: ensure volume mount and reloader annotation, or restart deployment.
- See `docs/mtls-guide.md` and `docs/security/e2e-encryption-architecture.md` for full runbooks.
