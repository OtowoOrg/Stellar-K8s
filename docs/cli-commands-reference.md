# CLI Commands Reference

Complete reference for all Stellar-K8s CLI commands.

## Overview

The `stellar-operator` CLI provides commands for managing Stellar infrastructure on Kubernetes, including deployment, monitoring, troubleshooting, and performance analysis.

## Commands

### run

Run the operator reconciliation loop.

```bash
stellar-operator run [OPTIONS]
```

**Options:**
- `--namespace <NAMESPACE>`: Kubernetes namespace (default: default)
- `--watch-namespace <NAMESPACE>`: Restrict to specific namespace
- `--enable-mtls`: Enable mutual TLS for REST API
- `--dry-run`: Simulate without applying changes
- `--scheduler`: Run latency-aware scheduler mode
- `--scheduler-name <NAME>`: Scheduler name (default: stellar-scheduler)
- `--dump-config`: Print configuration and exit
- `--preflight-only`: Run preflight checks only

**Examples:**
```bash
stellar-operator run --namespace stellar-system
stellar-operator run --enable-mtls --namespace stellar-system
stellar-operator run --dry-run
```

### webhook

Run the admission webhook server.

```bash
stellar-operator webhook [OPTIONS]
```

**Options:**
- `--bind <ADDRESS>`: Listen address (default: 0.0.0.0:8443)
- `--cert-path <PATH>`: TLS certificate path
- `--key-path <PATH>`: TLS private key path
- `--log-level <LEVEL>`: Log level (default: info)
- `--log-format <FORMAT>`: Log format (json, pretty)

**Examples:**
```bash
stellar-operator webhook --bind 0.0.0.0:8443 \
  --cert-path /tls/tls.crt \
  --key-path /tls/tls.key
```

### benchmark

Run the StellarBenchmark controller.

```bash
stellar-operator benchmark [OPTIONS]
```

**Options:**
- `--namespace <NAMESPACE>`: Namespace to watch (default: default)
- `--log-level <LEVEL>`: Log level (default: info)

**Examples:**
```bash
stellar-operator benchmark --namespace stellar-system
```

### benchmark-compare

Compare performance metrics between two clusters.

```bash
stellar-operator benchmark-compare [OPTIONS]
```

**Options:**
- `--cluster-a-context <CONTEXT>`: Kubernetes context for Cluster A
- `--cluster-b-context <CONTEXT>`: Kubernetes context for Cluster B
- `--cluster-a-prometheus <URL>`: Prometheus URL for Cluster A
- `--cluster-b-prometheus <URL>`: Prometheus URL for Cluster B
- `--cluster-a-label <LABEL>`: Display label for Cluster A
- `--cluster-b-label <LABEL>`: Display label for Cluster B
- `--namespace <NAMESPACE>`: Namespace to query (default: stellar-system)
- `--duration <SECONDS>`: Collection duration (default: 60)
- `--interval <SECONDS>`: Sampling interval (default: 5)
- `--output <PATH>`: Output file path
- `--format <FORMAT>`: Output format (table, html, json, pdf)
- `--live <BOOL>`: Show real-time updates (default: true)
- `--metrics <LIST>`: Metrics to compare (comma-separated)

**Examples:**
```bash
# Compare two contexts
stellar-operator benchmark-compare \
  --cluster-a-context prod \
  --cluster-b-context staging

# Compare Prometheus instances
stellar-operator benchmark-compare \
  --cluster-a-prometheus http://prom-a:9090 \
  --cluster-b-prometheus http://prom-b:9090

# Export to HTML
stellar-operator benchmark-compare \
  --cluster-a-context prod \
  --cluster-b-context staging \
  --output report.html \
  --format html
```

### version

Show version and build information.

```bash
stellar-operator version
```

**Output:**
```
Stellar-K8s Operator v0.1.0
Build Date: 2024-01-15
Git SHA: abc123def456
Rust Version: 1.93.0
```

### info

Show cluster information for a namespace.

```bash
stellar-operator info [OPTIONS]
```

**Options:**
- `--namespace <NAMESPACE>`: Namespace to query (default: default)

**Examples:**
```bash
stellar-operator info --namespace stellar-system
```

### check-crd

Verify StellarNode CRD installation.

```bash
stellar-operator check-crd
```

**Output:**
```
✓ StellarNode CRD is installed
✓ Version: v1alpha1
✓ All required fields present
```

### prune-archive

Prune old history archive checkpoints.

```bash
stellar-operator prune-archive [OPTIONS]
```

**Options:**
- `--namespace <NAMESPACE>`: Namespace
- `--node-name <NAME>`: StellarNode name
- `--keep-checkpoints <N>`: Number of checkpoints to keep
- `--dry-run`: Preview without deleting

**Examples:**
```bash
stellar-operator prune-archive \
  --namespace stellar-system \
  --node-name validator-1 \
  --keep-checkpoints 100
```

### diff

Show difference between desired and live cluster state.

```bash
stellar-operator diff [OPTIONS]
```

**Options:**
- `--namespace <NAMESPACE>`: Namespace
- `--node-name <NAME>`: StellarNode name
- `--output <FORMAT>`: Output format (text, json)

**Examples:**
```bash
stellar-operator diff \
  --namespace stellar-system \
  --node-name validator-1
```

### generate-runbook

Generate a troubleshooting runbook for a StellarNode.

```bash
stellar-operator generate-runbook <NODE_NAME> [OPTIONS]
```

**Options:**
- `--namespace <NAMESPACE>`: Namespace (default: default)
- `--output <PATH>`: Output file path (default: stdout)

**Examples:**
```bash
stellar-operator generate-runbook my-validator \
  --namespace stellar-system \
  --output runbook.md
```

### incident-report

Generate an incident report for a specific time window.

```bash
stellar-operator incident-report [OPTIONS]
```

**Options:**
- `--namespace <NAMESPACE>`: Namespace
- `--start-time <TIME>`: Start time (RFC3339)
- `--end-time <TIME>`: End time (RFC3339)
- `--output <PATH>`: Output file path

**Examples:**
```bash
stellar-operator incident-report \
  --namespace stellar-system \
  --start-time 2024-01-15T10:00:00Z \
  --end-time 2024-01-15T11:00:00Z \
  --output incident.zip
```

### simulator

Local simulator for development and testing.

```bash
stellar-operator simulator up [OPTIONS]
```

**Options:**
- `--cluster-name <NAME>`: Kind cluster name (default: stellar-sim)
- `--namespace <NAMESPACE>`: Namespace (default: stellar-system)
- `--use-k3s`: Use k3s instead of kind

**Examples:**
```bash
stellar-operator simulator up
stellar-operator simulator up --cluster-name my-cluster
stellar-operator simulator up --use-k3s
```

### completions

Generate shell completion scripts.

```bash
stellar-operator completions <SHELL>
```

**Shells:**
- bash
- zsh
- fish
- powershell
- elvish

**Examples:**
```bash
# Bash
stellar-operator completions bash > /etc/bash_completion.d/stellar-operator

# Zsh
stellar-operator completions zsh > ~/.zsh/completion/_stellar-operator

# Fish
stellar-operator completions fish > ~/.config/fish/completions/stellar-operator.fish
```

## Global Options

Available for all commands:

- `--offline`: Skip background version check
- `--help`: Show help information
- `--version`: Show version information

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `OPERATOR_NAMESPACE` | Default namespace | default |
| `WATCH_NAMESPACE` | Namespace to watch | - |
| `ENABLE_MTLS` | Enable mTLS | false |
| `DRY_RUN` | Dry-run mode | false |
| `RUN_SCHEDULER` | Scheduler mode | false |
| `SCHEDULER_NAME` | Scheduler name | stellar-scheduler |
| `WEBHOOK_BIND` | Webhook bind address | 0.0.0.0:8443 |
| `WEBHOOK_CERT_PATH` | Webhook cert path | - |
| `WEBHOOK_KEY_PATH` | Webhook key path | - |
| `LOG_LEVEL` | Log level | info |
| `LOG_FORMAT` | Log format | json |
| `CLUSTER_A_CONTEXT` | Cluster A context | - |
| `CLUSTER_B_CONTEXT` | Cluster B context | - |
| `CLUSTER_A_PROMETHEUS` | Cluster A Prometheus | - |
| `CLUSTER_B_PROMETHEUS` | Cluster B Prometheus | - |
| `STELLAR_OFFLINE` | Offline mode | false |

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error |
| 2 | Invalid arguments |
| 3 | Kubernetes API error |
| 4 | Configuration error |

## Examples by Use Case

### Development

```bash
# Start local simulator
stellar-operator simulator up

# Run operator in dry-run mode
stellar-operator run --dry-run --namespace stellar-system

# Generate completions
stellar-operator completions bash > /etc/bash_completion.d/stellar-operator
```

### Production Deployment

```bash
# Run operator with mTLS
stellar-operator run \
  --enable-mtls \
  --namespace stellar-system \
  --watch-namespace stellar-prod

# Run webhook server
stellar-operator webhook \
  --bind 0.0.0.0:8443 \
  --cert-path /tls/tls.crt \
  --key-path /tls/tls.key
```

### Monitoring and Troubleshooting

```bash
# Check cluster info
stellar-operator info --namespace stellar-system

# Generate runbook
stellar-operator generate-runbook validator-1 \
  --namespace stellar-system \
  --output runbook.md

# Create incident report
stellar-operator incident-report \
  --namespace stellar-system \
  --start-time 2024-01-15T10:00:00Z \
  --end-time 2024-01-15T11:00:00Z \
  --output incident.zip
```

### Performance Testing

```bash
# Run benchmark
stellar-operator benchmark --namespace stellar-system

# Compare clusters
stellar-operator benchmark-compare \
  --cluster-a-context prod \
  --cluster-b-context staging \
  --duration 300 \
  --output comparison.html
```

### Maintenance

```bash
# Prune archives
stellar-operator prune-archive \
  --namespace stellar-system \
  --node-name validator-1 \
  --keep-checkpoints 100

# Check state diff
stellar-operator diff \
  --namespace stellar-system \
  --node-name validator-1
```

## Related Documentation

- [Benchmark Compare Guide](./benchmark-compare.md)
- [Troubleshooting](./troubleshooting.md)
- [Monitoring Setup](./monitoring-setup.md)
- [Development Guide](./development.md)
