# kubectl-stellar Plugin

<p align="center">
  <img src="../assets/logo.png" alt="Stellar-K8s Logo" width="120" />
</p>

A kubectl plugin for managing StellarNode resources in Kubernetes clusters.

## Installation

### Build from Source

```bash
cargo build --release --bin kubectl-stellar
cp target/release/kubectl-stellar ~/.local/bin/kubectl-stellar
chmod +x ~/.local/bin/kubectl-stellar
```

### Install via Krew (when available)

```bash
kubectl krew install stellar
```

## Global Flags

| Flag | Description |
|------|-------------|
| `-n, --namespace` | Kubernetes namespace (defaults to current context) |
| `-o, --output` | Output format: `table` (default), `json`, `yaml` |
| `--dry-run` | Simulate the command without making any state-changing API calls |

### --dry-run

Use `--dry-run` to preview what a command would do without executing it. Safe to run against production clusters.

```bash
# Preview what 'debug' would do without exec-ing into the pod
kubectl stellar debug my-validator --dry-run

# All read-only commands (list, status, events) pass through normally
kubectl stellar list --dry-run
kubectl stellar status --dry-run
```



### List StellarNode Resources

List all StellarNode resources in the current namespace:

```bash
kubectl stellar list
```

List all StellarNode resources across all namespaces:

```bash
kubectl stellar list --all-namespaces
# or
kubectl stellar list -A
```

Output in JSON or YAML format:

```bash
kubectl stellar list -o json
kubectl stellar list -o yaml
```

### View Pod Logs

Get logs from pods associated with a StellarNode:

```bash
kubectl stellar logs <node-name>
```

Follow log output:

```bash
kubectl stellar logs <node-name> -f
```

Specify container name (if multiple containers):

```bash
kubectl stellar logs <node-name> -c <container-name>
```

Show last N lines:

```bash
kubectl stellar logs <node-name> --tail 50
```

Specify namespace:

```bash
kubectl stellar logs <node-name> -n <namespace>
```

### Check Sync Status

Check sync status of all StellarNode resources in the current namespace:

```bash
kubectl stellar status
# or
kubectl stellar sync-status
```

Check status of a specific node:

```bash
kubectl stellar status <node-name>
```

Check status across all namespaces:

```bash
kubectl stellar status -A
```

Output in JSON or YAML format:

```bash
kubectl stellar status -o json
kubectl stellar status -o yaml
```

### Summary

Show a high-level aggregate view of all managed StellarNodes and their health:

```bash
kubectl stellar summary
```

Includes across all namespaces:

```bash
kubectl stellar summary -A
```

Output in JSON or YAML format:

```bash
kubectl stellar summary -o json
kubectl stellar summary -o yaml
```

Example table output:

```
StellarNode Summary
========================================
  Total nodes : 5
  Healthy     : 4
  Synced      : 3
  Degraded    : 1
  Pending     : 0

By Type:
  Horizon         : 2
  SorobanRpc      : 1
  Validator       : 2

By Network:
  Mainnet         : 3
  Testnet         : 2
```

### Explain Stellar Error Codes

Explain a Stellar error code (e.g., `tx_bad_auth`, `op_no_destination`):

```bash
kubectl stellar explain tx_bad_auth
```

### Search Documentation

Search the built-in documentation for keywords:

```bash
kubectl stellar search "mTLS rotation"
```

Show the full content of matching documents:

```bash
kubectl stellar search "S3 backup config" --full
```

The search tool works completely offline by using a built-in index of all documentation files, Architecture Decision Records (ADRs), and guides.

## Snapshot Management

Manage VolumeSnapshots for StellarNode data PVCs. Requires the CSI snapshotter (`snapshot.storage.k8s.io/v1`) installed in your cluster.

### Create a Snapshot

```bash
kubectl stellar snapshot create <node-name>
```

Optionally specify a VolumeSnapshotClass:

```bash
kubectl stellar snapshot create my-validator --volume-snapshot-class csi-aws-vsc
```

The snapshot is named `<node-name>-data-<timestamp>` and labelled `stellar.org/snapshot-of=<node-name>`.

### List Snapshots

```bash
# All operator-managed snapshots in the current namespace
kubectl stellar snapshot list

# Snapshots for a specific node
kubectl stellar snapshot list my-validator

# Across all namespaces
kubectl stellar snapshot list -A

# JSON output
kubectl stellar snapshot list -o json
```

### Restore from a Snapshot

```bash
kubectl stellar snapshot restore <snapshot-name> <node-name>
```

This patches `spec.storage.snapshotRef.volumeSnapshotName` on the StellarNode so the operator uses the snapshot as the PVC data source on the next pod (re)creation. To trigger an immediate restore, also delete the existing data PVC:

```bash
kubectl stellar snapshot restore my-validator-data-20260101-120000 my-validator
kubectl delete pvc my-validator-data -n stellar
```

The operator reprovisioning the PVC from the snapshot and restarts the pod automatically.

> All snapshot sub-commands support `--dry-run` to preview actions without making any API calls.

---

## Examples

```bash
# List all nodes
kubectl stellar list

# Check if nodes are synced
kubectl stellar status

# View logs from a validator node
kubectl stellar logs my-validator -f

# Check status of a specific node in JSON format
kubectl stellar status my-horizon-node -o json
```

## Requirements

- kubectl installed and configured
- Stellar-K8s operator installed in your cluster
- StellarNode CRD available

## Troubleshooting

If you get "command not found" errors:

1. Ensure the plugin is in your PATH
2. The binary must be named `kubectl-stellar` (or `kubectl-stellar.exe` on Windows)
3. The binary must be executable

If you get "No pods found" errors:

1. Verify the StellarNode resource exists: `kubectl get stellarnodes`
2. Check that pods are running: `kubectl get pods -l app.kubernetes.io/name=stellar-node`
3. Ensure you're using the correct namespace
