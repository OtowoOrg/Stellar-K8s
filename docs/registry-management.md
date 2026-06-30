# Container Registry Management with Security Scanning

Declarative container registry management via the `StellarRegistry` CRD. Integrates
Trivy/Grype scanning, Cosign image signing, multi-region mirroring, garbage
collection, and admission control for vulnerable or unsigned images.

## Architecture

```
StellarRegistry CRD
    ├── Scanning (Trivy/Grype)
    ├── Signing (Cosign)
    ├── Admission Policy
    ├── Multi-region Mirrors (3+)
    ├── Garbage Collection
    └── Registry Proxy (Docker Hub)
```

## Quick Start

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarRegistry
metadata:
  name: main-registry
  namespace: stellar
spec:
  endpoint: registry.stellar.example.com
  scanning:
    enabled: true
    scanner: trivy
    endpoint: http://trivy.trivy.svc:4954
    maxCriticalCves: 0
    maxHighCves: 5
  signing:
    enabled: true
    cosignPublicKeyRef: cosign-public-key
    requireSignature: true
  admission:
    blockVulnerable: true
    blockUnsigned: true
  mirrors:
    - region: us-east-1
      endpoint: registry-us-east.stellar.example.com
    - region: eu-west-1
      endpoint: registry-eu-west.stellar.example.com
    - region: ap-southeast-1
      endpoint: registry-ap.stellar.example.com
  garbageCollection:
    enabled: true
    schedule: "0 2 * * 0"
    retentionDays: 30
  proxy:
    enabled: true
    upstream: https://registry-1.docker.io
    cacheTtlHours: 24
```

## Admission Control

The operator blocks pod deployments when:

- Images exceed CVE thresholds (critical/high counts)
- Images lack Cosign signatures when `requireSignature` is enabled

Use `check_admission()` from the registry controller or configure a validating
webhook that references `StellarRegistry` status.

## Compliance Reports

`status.complianceReport` includes:

- Total CVE count and highest severity
- Percentage of signed images
- Overall compliance boolean

## Grafana Dashboard

Import `monitoring/grafana/stellar-registry-dashboard.json` for vulnerability
metrics and compliance trends.

## Best Practices

1. Enable scanning on every registry before production use
2. Require Cosign signatures for all production images
3. Configure 3+ regional mirrors for availability
4. Run garbage collection weekly to reclaim storage
5. Use the registry proxy to avoid Docker Hub rate limits
6. Enable auto-patch for base images with known CVEs
