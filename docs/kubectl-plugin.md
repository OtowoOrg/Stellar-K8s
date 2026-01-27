# kubectl-stellar Plugin

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

## Usage

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
