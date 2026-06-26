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
| `example-benchmark.yaml` | Benchmark runner manifest |
| `example_nodeport_config.yaml` | NodePort service configuration |

## Quick Start

```bash
# Install the CRD first
kubectl apply -f ../crd/stellarnode-crd.yaml

# Apply the minimal sample
kubectl apply -f minimal-validator.yaml
```

> **Note**: For production manifests see the `examples/` directory in the repo root.
