# Secret Management Guide

Declarative secret management via the `StellarSecret` CRD with dynamic credentials,
automatic rotation, KMS encryption, and multi-backend support.

## Features

- Dynamic database credentials with TTL
- Automatic rotation every 30 days (configurable via `720h` interval)
- AWS KMS, Azure Key Vault, GCP KMS, and Vault backends
- Complete audit logging with anomaly detection
- Zero-downtime rotation with version history and rollback
- Secret injection as environment variables or files

## Quick Start

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarSecret
metadata:
  name: postgres-dynamic
spec:
  secretName: postgres-creds
  backend: vault
  provider: Vault
  dynamic:
    enabled: true
    ttl: 1h
  rotation:
    interval: 720h
    zeroDowntime: true
```

## Grafana Dashboard

Import `monitoring/grafana/stellar-secret-dashboard.json` for rotation metrics,
sync drift, and access anomaly alerts.

## Compliance

Export audit trails via `stellar-operator export-compliance` for auditor review.
