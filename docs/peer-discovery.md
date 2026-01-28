# Dynamic Peer Discovery

## Overview

The Stellar-K8s operator now includes a **dynamic peer discovery** system that automatically discovers other StellarNode resources in the cluster and maintains a shared ConfigMap with the latest peer information. This enables validators to dynamically discover and connect to other peers without manual configuration.

## Architecture

### Components

1. **PeerDiscoveryManager** - Watches all StellarNode resources and maintains peer state
2. **Shared ConfigMap** - Stores the current list of peers in a well-known location
3. **Peer Watcher** - Monitors StellarNode creation, updates, and deletion events
4. **Config Reload Trigger** - Signals validators to reload configuration when peers change

### Data Flow

```
StellarNode Resources
        ↓
PeerDiscoveryManager (watches all nodes)
        ↓
Extract peer info (IP, port, namespace, name)
        ↓
Update shared ConfigMap (stellar-peers)
        ↓
Trigger config-reload on healthy validators
```

## Configuration

### Default Settings

The peer discovery system uses sensible defaults:

```rust
PeerDiscoveryConfig {
    config_namespace: "stellar-system",      // Where peers ConfigMap is stored
    config_map_name: "stellar-peers",        // Name of the ConfigMap
    peer_port: 11625,                        // Stellar Core peer port
}
```

### Customization

To customize peer discovery settings, modify the configuration in `src/main.rs`:

```rust
let peer_discovery_config = controller::PeerDiscoveryConfig {
    config_namespace: "my-namespace".to_string(),
    config_map_name: "my-peers".to_string(),
    peer_port: 11625,
};
```

## Shared ConfigMap Structure

The peer discovery system maintains a ConfigMap at `stellar-system/stellar-peers` with the following structure:

### peers.json

Contains a JSON array of all discovered peers:

```json
[
  {
    "name": "validator-1",
    "namespace": "stellar-nodes",
    "nodeType": "Validator",
    "ip": "10.0.1.5",
    "port": 11625,
    "peerString": "10.0.1.5:11625"
  },
  {
    "name": "validator-2",
    "namespace": "stellar-nodes",
    "nodeType": "Validator",
    "ip": "10.0.1.6",
    "port": 11625,
    "peerString": "10.0.1.6:11625"
  }
]
```

### peers.txt

Simple text format (one peer per line):

```
10.0.1.5:11625
10.0.1.6:11625
10.0.1.7:11625
```

### peer_count

Total number of discovered peers:

```
3
```

## How It Works

### 1. Peer Discovery

The `PeerDiscoveryManager` continuously watches all StellarNode resources:

- **Filters**: Only includes Validator nodes (Horizon and SorobanRpc are excluded)
- **Suspended Nodes**: Automatically excluded from peer list
- **IP Resolution**: Extracts IP from Service ClusterIP or LoadBalancer status

### 2. Peer Information Extraction

For each validator node, the system extracts:

- **Name**: StellarNode resource name
- **Namespace**: StellarNode namespace
- **IP Address**: From the associated Service
  - Prefers ClusterIP for internal communication
  - Falls back to LoadBalancer IP if available
- **Port**: Configurable (default: 11625)
- **Node Type**: Always "Validator" for peer discovery

### 3. ConfigMap Updates

When peers change (new node, node deleted, IP changed):

1. The peer set is updated in memory
2. The shared ConfigMap is patched with new peer list
3. All healthy validators are signaled to reload configuration

### 4. Configuration Reload

For each healthy validator:

1. Find the pod IP
2. Send HTTP command to Stellar Core: `http://{pod-ip}:11626/http-command?admin=true&command=config-reload`
3. Stellar Core reloads configuration without restarting

## Integration with Reconciliation

The peer discovery is integrated into the main reconciliation loop:

```rust
// After health check passes
if node.spec.node_type == NodeType::Validator && health_result.healthy {
    if let Err(e) = peer_discovery::trigger_peer_config_reload(client, node).await {
        warn!("Failed to trigger peer config reload: {}", e);
    }
}
```

This ensures:
- Config reload only happens when the node is healthy
- Validators automatically pick up new peers
- No manual intervention required

## Usage Examples

### Accessing Peer Information

Validators can access the peer list by mounting the ConfigMap:

```yaml
apiVersion: v1
kind: Pod
metadata:
  name: validator-pod
spec:
  containers:
  - name: stellar-core
    volumeMounts:
    - name: peers
      mountPath: /etc/stellar/peers
  volumes:
  - name: peers
    configMap:
      name: stellar-peers
      items:
      - key: peers.txt
        path: peers.txt
      - key: peers.json
        path: peers.json
```

### Querying Peers Programmatically

```rust
use stellar_k8s::controller::{get_peers_from_config_map, PeerDiscoveryConfig};

let config = PeerDiscoveryConfig::default();
let peers = get_peers_from_config_map(&client, &config).await?;

for peer in peers {
    println!("Peer: {} at {}", peer.name, peer.to_peer_string());
}
```

### Monitoring Peer Count

The `peer_count` field in the ConfigMap can be monitored:

```bash
kubectl get configmap stellar-peers -n stellar-system -o jsonpath='{.data.peer_count}'
```

## Failure Handling

### Service Not Ready

If a validator's Service doesn't have an IP yet:
- The peer is skipped in that reconciliation cycle
- It will be picked up in the next cycle when the Service is ready
- No errors are logged (expected during startup)

### Pod Not Ready

If a validator pod is not healthy:
- Config reload is not triggered
- The peer remains in the ConfigMap
- Config reload will be attempted in the next reconciliation cycle

### Network Issues

If the HTTP command to trigger config-reload fails:
- A warning is logged
- The peer remains in the ConfigMap
- The next reconciliation cycle will retry

## Monitoring and Debugging

### Check Peer Discovery Status

```bash
# View the shared peers ConfigMap
kubectl get configmap stellar-peers -n stellar-system -o yaml

# Check peer count
kubectl get configmap stellar-peers -n stellar-system -o jsonpath='{.data.peer_count}'

# View peers as JSON
kubectl get configmap stellar-peers -n stellar-system -o jsonpath='{.data.peers\.json}' | jq
```

### View Operator Logs

```bash
# Watch peer discovery logs
kubectl logs -f deployment/stellar-operator -n stellar-system | grep "peer discovery"

# View all peer-related events
kubectl logs -f deployment/stellar-operator -n stellar-system | grep -i peer
```

### Verify Config Reload

```bash
# Check if config-reload was triggered
kubectl logs -f deployment/stellar-operator -n stellar-system | grep "config-reload"

# Check validator pod logs for config reload
kubectl logs -f <validator-pod> -n stellar-nodes | grep "config-reload"
```

## Performance Considerations

### Watcher Efficiency

- The peer discovery watcher runs in a separate task
- Uses Kubernetes watch API for efficient event streaming
- Minimal CPU/memory overhead

### ConfigMap Updates

- ConfigMap is only updated when peers actually change
- Uses strategic merge patch for efficiency
- Batches multiple peer changes into single update

### Config Reload Frequency

- Config reload is only triggered for healthy validators
- Happens once per reconciliation cycle (default: 60 seconds when ready)
- Stellar Core handles reload efficiently without restart

## Troubleshooting

### Peers Not Discovered

**Symptom**: ConfigMap is empty or missing peers

**Causes**:
1. Validators are suspended
2. Services don't have IPs yet
3. Peer discovery manager crashed

**Solution**:
```bash
# Check if validators are running
kubectl get stellarnodes -A

# Check service IPs
kubectl get svc -A -l app.kubernetes.io/component=stellar-node

# Check operator logs
kubectl logs deployment/stellar-operator -n stellar-system | grep "peer discovery"
```

### Config Reload Not Triggering

**Symptom**: Peers are discovered but validators don't pick them up

**Causes**:
1. Validator pod is not healthy
2. Pod IP is not available
3. HTTP command endpoint is not responding

**Solution**:
```bash
# Check validator health
kubectl get stellarnodes -A -o wide

# Check pod status
kubectl get pods -A -l app.kubernetes.io/component=stellar-node

# Test HTTP endpoint manually
kubectl exec <validator-pod> -- curl http://localhost:11626/http-command?admin=true&command=info
```

### ConfigMap Not Updating

**Symptom**: New validators are created but not added to ConfigMap

**Causes**:
1. Peer discovery manager has insufficient permissions
2. ConfigMap namespace doesn't exist
3. Watcher is lagging

**Solution**:
```bash
# Verify RBAC permissions
kubectl get clusterrole stellar-operator -o yaml | grep -A 20 "configmaps"

# Create namespace if missing
kubectl create namespace stellar-system

# Check watcher status in logs
kubectl logs deployment/stellar-operator -n stellar-system | grep "watcher"
```

## Security Considerations

### RBAC Requirements

The operator needs permissions to:
- List and watch StellarNode resources (all namespaces)
- Get Service resources (all namespaces)
- Get Pod resources (all namespaces)
- Create/patch ConfigMap in `stellar-system` namespace

### Network Access

- Peer discovery uses internal Kubernetes DNS
- Config reload uses pod IP (internal network)
- No external network access required

### Data Privacy

- Peer information is stored in a ConfigMap (not encrypted by default)
- Consider using encryption at rest for sensitive deployments
- Peer IPs are internal cluster IPs (not exposed externally)

## Future Enhancements

Potential improvements for future versions:

1. **Peer Filtering**: Filter peers by network, region, or custom labels
2. **Peer Metrics**: Export peer count and discovery latency as metrics
3. **Peer Validation**: Verify peer connectivity before adding to list
4. **Peer Prioritization**: Prioritize peers by latency or availability
5. **Custom Peer Sources**: Support external peer discovery sources
6. **Peer Rotation**: Implement peer rotation for load balancing

## Related Documentation

- [Stellar Core Peers Configuration](https://developers.stellar.org/docs/run-core-node/core-configuration#peers)
- [Kubernetes ConfigMap Documentation](https://kubernetes.io/docs/concepts/configuration/configmap/)
- [kube-rs Watcher Documentation](https://docs.rs/kube/latest/kube/runtime/watcher/)
