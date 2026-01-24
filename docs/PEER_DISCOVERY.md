# Dynamic Peer Discovery for Stellar Validators

## Overview

The Stellar-K8s operator now includes a **dynamic peer discovery** system that automatically discovers other StellarNode resources in the cluster and updates a shared ConfigMap with validated peer addresses. This enables Stellar validators to dynamically connect to each other without requiring static peer configuration.

## Architecture

### Components

1. **Peer Discovery Watcher** (`peer_discovery.rs`)
   - Continuously monitors StellarNode resources in the namespace
   - Discovers active validator peers (IP:port combinations)
   - Updates a shared ConfigMap with the latest peer list
   - Triggers rolling pod restarts when peers change

2. **Shared ConfigMap** (`stellar-peers`)
   - Namespace-scoped ConfigMap containing `KNOWN_PEERS` configuration
   - Updates automatically when validators are created, deleted, or suspended
   - Stores metadata about discovery (timestamp, peer count, active validators)

3. **Rolling Update Mechanism**
   - Patches StatefulSet pod templates with restart annotations
   - Kubernetes automatically performs rolling restart
   - Ensures validators pick up new peer configuration

## Features

✅ **Real-time Discovery**
- Watches StellarNode resources across the cluster
- Periodic checks (30-second interval) ensure fresh peer data
- Excludes suspended nodes and non-validator types

✅ **Intelligent Peer Detection**
- Only discovers from ready, running pods
- Uses StatefulSet stable DNS for peer addressing
- Extracts peer IP from running pods
- Respects custom peer port configuration

✅ **Efficient Updates**
- Only triggers rolling updates when peer list actually changes
- Avoids unnecessary pod restarts
- Logs all changes for debugging

✅ **Self-Excluding**
- Automatically excludes the local node from peer list
- Prevents self-referential peer connections

## Configuration

### ValidatorConfig Extension

The `ValidatorConfig` CRD now includes an optional `peerPort` field:

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: validator-1
spec:
  nodeType: Validator
  network: Testnet
  version: "v21.0.0"
  validatorConfig:
    seedSecretRef: validator-seed
    quorumSet: |
      [QUORUM_SET]
      THRESHOLD_PERCENT = 66
      VALIDATORS = ["$validator2"]
    enableHistoryArchive: true
    historyArchiveUrls:
      - "https://history.stellar.org"
    peerPort: 11625  # Optional, defaults to 11625
```

### ConfigMap Structure

The operator maintains a ConfigMap named `stellar-peers` in the same namespace:

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: stellar-peers
  namespace: default
  labels:
    app.kubernetes.io/name: stellar-node
    app.kubernetes.io/component: peer-discovery
    app.kubernetes.io/managed-by: stellar-operator
data:
  KNOWN_PEERS: |
    192.168.1.10:11625
    192.168.1.11:11625
    192.168.1.12:11625
  discovery_metadata: "discovered_at=2025-01-24T12:34:56Z,peer_count=3,active_validators=4"
```

## How It Works

### Discovery Process

1. **Initialization**
   - When the operator starts, peer discovery watcher task is spawned
   - Watcher runs in background independent of reconciliation loop

2. **Periodic Discovery** (Every 30 seconds)
   ```
   ┌─────────────────────────────────────────────┐
   │ Discover Peers                              │
   │ - List all StellarNode resources            │
   │ - Filter validators only                    │
   │ - Skip suspended/non-ready nodes            │
   │ - Extract pod IP:port for each validator    │
   └──────────────┬──────────────────────────────┘
                  │
                  ▼
   ┌─────────────────────────────────────────────┐
   │ Compare with Previous Peers                 │
   │ - If no changes → sleep 30s                 │
   │ - If changed → update ConfigMap             │
   └──────────────┬──────────────────────────────┘
                  │
                  ▼
   ┌─────────────────────────────────────────────┐
   │ Update ConfigMap                            │
   │ - Apply "stellar-peers" ConfigMap           │
   │ - Record metadata                           │
   │ - Log changes                               │
   └──────────────┬──────────────────────────────┘
                  │
                  ▼
   ┌─────────────────────────────────────────────┐
   │ Trigger Rolling Update                      │
   │ - Patch StatefulSet pod templates           │
   │ - Add restart annotation (timestamp)        │
   │ - Kubernetes handles rolling restart        │
   └─────────────────────────────────────────────┘
   ```

3. **Pod Configuration**
   - Validators mount the ConfigMap as a volume
   - Stellar Core reads `KNOWN_PEERS` during startup/configuration reload
   - Pod restart ensures new configuration is loaded

### Peer Address Resolution

For each validator:

1. **Check StatefulSet Status**
   - Verify at least 1 replica is configured
   - Skip nodes with 0 replicas (suspended)

2. **Find Ready Pod**
   - List pods matching `app.kubernetes.io/instance={node-name}`
   - Find pod with `Ready=True` condition
   - Extract pod IP from pod status

3. **Format Peer Address**
   - Combine pod IP with configured peer port (default: 11625)
   - Result: `{POD_IP}:{PEER_PORT}` (e.g., `10.244.0.5:11625`)

## Usage Example

### Deploy Multiple Validators

```yaml
---
apiVersion: v1
kind: Secret
metadata:
  name: validator1-seed
type: Opaque
stringData:
  STELLAR_CORE_SEED: "SBXYZ..." # Validator 1 seed

---
apiVersion: v1
kind: Secret
metadata:
  name: validator2-seed
type: Opaque
stringData:
  STELLAR_CORE_SEED: "SBABCD..." # Validator 2 seed

---
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: validator-1
spec:
  nodeType: Validator
  network: Testnet
  version: "v21.0.0"
  storage:
    storageClass: "standard"
    size: "100Gi"
  validatorConfig:
    seedSecretRef: validator1-seed
    quorumSet: |
      [QUORUM_SET]
      THRESHOLD_PERCENT = 50
      VALIDATORS = ["validator-2"]
    enableHistoryArchive: true
    historyArchiveUrls:
      - "https://history.stellar.org"
    peerPort: 11625

---
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: validator-2
spec:
  nodeType: Validator
  network: Testnet
  version: "v21.0.0"
  storage:
    storageClass: "standard"
    size: "100Gi"
  validatorConfig:
    seedSecretRef: validator2-seed
    quorumSet: |
      [QUORUM_SET]
      THRESHOLD_PERCENT = 50
      VALIDATORS = ["validator-1"]
    enableHistoryArchive: true
    historyArchiveUrls:
      - "https://history.stellar.org"
    peerPort: 11625
```

### Monitor Peer Discovery

```bash
# Watch the peers ConfigMap in real-time
kubectl get configmap stellar-peers -o jsonpath='{.data.KNOWN_PEERS}' && echo ""

# Check discovery metadata
kubectl get configmap stellar-peers -o jsonpath='{.data.discovery_metadata}' && echo ""

# Monitor operator logs
kubectl logs -f -l app.kubernetes.io/name=stellar-operator -l app.kubernetes.io/component=operator

# Check rolling updates (look for pod restarts)
kubectl get pods -l app.kubernetes.io/name=stellar-node -w
```

## API Reference

### `discover_peers()`

```rust
pub async fn discover_peers(
    client: &Client,
    namespace: &str,
    exclude_node: Option<&str>,
) -> Result<PeerDiscoveryResult>
```

Discovers all active validator peers in the given namespace.

**Parameters:**
- `client`: Kubernetes client
- `namespace`: Namespace to search for StellarNode resources
- `exclude_node`: Optional node name to exclude from results

**Returns:** `PeerDiscoveryResult` containing:
- `peers`: List of discovered peer addresses
- `active_validator_count`: Number of validators found
- `changed`: Whether the peer list changed

### `ensure_peers_config_map()`

```rust
pub async fn ensure_peers_config_map(
    client: &Client,
    namespace: &str,
    discovery_result: &PeerDiscoveryResult,
) -> Result<bool>
```

Creates or updates the shared ConfigMap with discovered peers.

**Returns:** `true` if peers changed, `false` if no update needed

### `trigger_rolling_update()`

```rust
pub async fn trigger_rolling_update(
    client: &Client,
    namespace: &str,
) -> Result<()>
```

Triggers rolling pod restart for all affected validators.

### `watch_peers()`

```rust
pub async fn watch_peers(
    client: Client,
    namespace: String,
)
```

Background task that continuously monitors and updates peers.
Runs in a separate spawned task.

## Error Handling

The implementation is robust against failures:

- **Pod listing failures**: Skipped, continues with other nodes
- **ConfigMap update failures**: Logged, retry on next cycle (30s)
- **Pod not ready**: Skipped, included once Ready=True
- **StatefulSet not found**: Skipped gracefully

## Performance Considerations

- **Discovery Interval**: 30 seconds (configurable via source code)
- **API Calls**: Minimal - single list operation per namespace per cycle
- **ConfigMap Updates**: Only when peers actually change
- **Pod Restarts**: Only when ConfigMap is updated

## RBAC Permissions

The operator already has all necessary permissions via the `stellar-operator` ClusterRole:

```yaml
# StellarNode CRD permissions
- apiGroups: ["stellar.org"]
  resources: ["stellarnodes"]
  verbs: ["get", "list", "watch"]

# Pod read permissions (for IP extraction)
- apiGroups: [""]
  resources: ["pods"]
  verbs: ["get", "list", "watch"]

# StatefulSet read permissions (for status check)
- apiGroups: ["apps"]
  resources: ["statefulsets"]
  verbs: ["get", "list", "watch"]

# ConfigMap management permissions
- apiGroups: [""]
  resources: ["configmaps"]
  verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
```

## Troubleshooting

### Peers not being discovered

**Check operator logs:**
```bash
kubectl logs -f deployment/stellar-operator
```

**Look for:**
- `Starting peer discovery watcher` - indicates task started
- `Peer discovery detected changes` - shows when peers are updated
- Error messages with `Peer discovery failed`

### ConfigMap not updating

**Verify ConfigMap exists:**
```bash
kubectl get configmap stellar-peers
kubectl describe configmap stellar-peers
```

**Check operator permissions:**
```bash
kubectl auth can-i get configmaps --as=system:serviceaccount:default:stellar-operator
kubectl auth can-i update configmaps --as=system:serviceaccount:default:stellar-operator
```

### Pods not restarting

**Check StatefulSet annotations:**
```bash
kubectl get statefulset validator-1 -o yaml | grep -A5 annotations
```

**Verify operator has update permissions:**
```bash
kubectl auth can-i patch statefulsets --as=system:serviceaccount:default:stellar-operator
```

### Peer addresses incorrect

**Verify pod IP:**
```bash
kubectl get pods -o wide -l app.kubernetes.io/instance=validator-1
```

**Check peer port configuration:**
```bash
kubectl get stellarnode validator-1 -o yaml | grep -A10 validatorConfig
```

## Future Enhancements

Potential improvements for consideration:

1. **Configurable Discovery Interval**
   - Currently hardcoded to 30 seconds
   - Could expose via environment variable

2. **Per-Namespace Watchers**
   - Current implementation watches single namespace
   - Could extend to watch multiple namespaces

3. **Quorum Set Auto-Update**
   - Could automatically update quorum set based on discovered validators
   - Requires careful validation to prevent liveness issues

4. **Peer Health Checking**
   - Could verify peer reachability before adding to ConfigMap
   - Would add latency but improve reliability

5. **Metrics Export**
   - Export peer discovery metrics (count, changes, errors)
   - Enable observability dashboards

## References

- **Stellar Core Configuration**: https://developers.stellar.org/docs/run-core-node/core-configuration
- **Stellar Peer Connections**: https://developers.stellar.org/docs/run-core-node/core-configuration#peers
- **kube-rs Documentation**: https://docs.rs/kube/latest/kube/
- **Kubernetes StatefulSet Rolling Updates**: https://kubernetes.io/docs/concepts/workloads/controllers/statefulset/#rolling-updates

## Implementation Notes

### Design Decisions

1. **ConfigMap-based Peer Distribution**
   - Simpler than custom CRD
   - Works with standard Kubernetes tooling
   - Easy to inspect and debug

2. **Pod Restart via Annotation**
   - Cleaner than pod deletion
   - Kubernetes handles the rolling update
   - Respects pod disruption budgets if configured

3. **Background Watch Loop**
   - Independent from reconciliation loop
   - Prevents peer updates from blocking node reconciliation
   - Enables parallel discovery across the cluster

4. **Namespace-scoped**
   - Each namespace has its own peer discovery
   - Prevents cross-namespace peer connections
   - Simplifies multi-tenancy

## Contributing

When extending peer discovery, consider:

1. **Backward Compatibility**: Ensure changes work with existing configurations
2. **Logging**: Add detailed trace-level logs for debugging
3. **Error Handling**: Gracefully handle partial failures
4. **Testing**: Add unit tests for new logic
5. **Documentation**: Update this guide with new features
