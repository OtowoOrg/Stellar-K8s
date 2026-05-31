# Advanced Secret Management with External KMS

Declarative secret management via the `SecretPolicy` CRD with AWS KMS, Azure Key Vault,
GCP Cloud KMS integrations, automatic rotation, and cross-cluster sync.

## Architecture

```text
┌──────────────────────────────────────────────────────────────────┐
│                     SecretPolicy Controller                       │
│                                                                   │
│  SecretPolicy CRD ──► KMS Backend (AWS/Azure/GCP)                │
│         │                    │                                    │
│         ▼                    ▼                                    │
│  Secret Rotator ──► Version Store (rollback)                     │
│         │                    │                                    │
│         ▼                    ▼                                    │
│  Cross-Cluster Sync ──► Immutable Audit Log (hash chain)         │
└──────────────────────────────────────────────────────────────────┘
```

## SecretPolicy CRD

```yaml
apiVersion: stellar.org/v1alpha1
kind: SecretPolicy
metadata:
  name: validator-seed-policy
  namespace: stellar
spec:
  secretName: validator-seed
  provider: aws
  aws:
    keyId: "arn:aws:kms:us-east-1:123456789:key/abc-def"
    region: us-east-1
  rotation:
    interval: "720h"
    zeroDowntime: true
    versionRetention: 5
  sync:
    targetClusters: ["cluster-dr", "cluster-backup"]
    syncInterval: "5m"
  audit:
    enabled: true
    anomalyDetection: true
  encryptInTransit: true
```

## KMS Providers

| Provider | Module | Encryption Algorithm |
|----------|--------|---------------------|
| AWS KMS | `security/kms` | AES_256_GCM |
| Azure Key Vault | `security/kms` | RSA-OAEP-256 |
| GCP Cloud KMS | `security/kms` | GOOGLE_SYMMETRIC_ENCRYPTION |

## Features

- **Automatic rotation** with zero-downtime dual-key overlap
- **Version rollback** to any retained previous version
- **Cross-cluster sync** via ClusterRegistry kubeconfig references
- **Immutable audit trail** with SHA-256 hash chain integrity
- **Anomaly detection** for unusual decrypt access patterns

## Metrics

- `stellar_secret_rotations_total` — rotation events per policy
- `stellar_secret_sync_drift` — clusters with version drift
- `stellar_secret_access_anomalies` — detected access anomalies

## Security Guarantees

- Secret plaintext is never logged by the operator
- Audit log entries are hash-chained for tamper detection
- In-transit encryption enforced for cross-cluster sync operations
- Integrates with existing Vault Agent and ESO paths for validator seeds
