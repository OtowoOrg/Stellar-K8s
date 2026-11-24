# Configuration Samples

This directory contains ready-to-apply sample manifests for testing and development.

| File | Purpose |
|------|---------|
| `minimal-validator.yaml` | Minimal testnet validator for CI/quick start |
| `test-stellarnode.yaml` | Full-featured validator with NodePort services |
| `disk-scaling-example.yaml` | Demonstrates proactive disk auto-expansion |
| `scp-analytics-example.yaml` | SCP message streaming to Kafka |
| `snapshot-bootstrap-csi.yaml` | CSI volume snapshot bootstrap |
| `snapshot-bootstrap-backup.yaml` | Backup-based snapshot bootstrap |
| `traffic-policy-example.yaml` | Traffic shaping policies |
| `sample-maintenance-plan.yaml` | Declarative planned maintenance orchestration |
| `sample-dryrun-validator.yaml` | Dry-run reconciliation verification and seed rotation |
| `sample-policy-bundle.yaml` | Signed CEL runtime admission policy bundle |
| `example-benchmark.yaml` | Benchmark runner manifest |
| `example_nodeport_config.yaml` | NodePort service configuration |

## Quick Start

```bash
# Install the StellarNode CRD (required)
kubectl apply -f ../crd/stellarnode-crd.yaml

# For benchmark samples, also install canonical benchmark CRDs:
kubectl apply -f ../crd/stellarbenchmark-crd.yaml
kubectl apply -f ../crd/stellarbenchmarkreport-crd.yaml

# Apply the minimal sample
kubectl apply -f minimal-validator.yaml
```

> **Note**: For production manifests see the `examples/` directory in the repo root.
