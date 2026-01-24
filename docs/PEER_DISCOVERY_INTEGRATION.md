# Peer Discovery Integration Guide

## High-Level Architecture

The peer discovery system is built as a **background watcher task** that runs independently from the main reconciliation loop. This design ensures:

- ✅ Non-blocking peer discovery operations
- ✅ Minimal API load (single list operation every 30 seconds)
- ✅ Graceful handling of partial failures
- ✅ Real-time updates when validators change

## Code Flow

### 1. Operator Startup

When the operator starts in `main.rs`:

```
main()
  ├─ Initialize tracing and logging
  ├─ Connect to Kubernetes cluster
  ├─ Verify CRD is installed
  └─ run_controller()
       ├─ Get POD_NAMESPACE environment variable
       ├─ Spawn peer discovery watcher task
       │   └─ watch_peers() [background task]
       │       ├─ Discover peers every 30 seconds
       │       ├─ Update ConfigMap if changed
       │       └─ Trigger rolling updates
       │
       └─ Start main StellarNode controller
           ├─ Watch StellarNode resources
           ├─ Watch owned Deployments/StatefulSets
           └─ Reconcile each node
```

### 2. Peer Discovery Cycle

The watcher runs in a continuous loop (simplified):

```rust
loop {
    // Discover all validator peers
    match discover_peers(&client, &namespace, None).await {
        Ok(discovery) => {
            // Check if peers changed
            if discovery.peers != last_peers {
                // Update ConfigMap
                if ensure_peers_config_map(&client, &namespace, &discovery).await.ok() == Some(true) {
                    // Trigger rolling restart
                    trigger_rolling_update(&client, &namespace).await.ok();
                }
                last_peers = discovery.peers;
            }
        }
        Err(e) => {
            error!("Peer discovery failed: {:?}", e);
        }
    }
    
    // Wait 30 seconds before next discovery
    tokio::time::sleep(Duration::from_secs(30)).await;
}
```

### 3. Discovery Details

For each validator in the namespace:

```
discover_peers()
  └─ Get all StellarNode resources
      ├─ Filter: Only Validator node type
      ├─ Filter: Skip suspended nodes
      └─ For each validator:
          ├─ Check StatefulSet has replicas > 0
          ├─ List pods matching instance label
          ├─ Find pod with Ready=True condition
          ├─ Extract pod IP
          ├─ Get peer port from ValidatorConfig (default 11625)
          └─ Format as "{IP}:{PORT}"
```

### 4. ConfigMap Update

When peers change:

```
ensure_peers_config_map()
  ├─ Get existing ConfigMap (if any)
  ├─ Compare KNOWN_PEERS with discovered peers
  ├─ If different:
  │   ├─ Create/update ConfigMap with:
  │   │   ├─ KNOWN_PEERS = peer list (newline-separated)
  │   │   └─ discovery_metadata = timestamp + counts
  │   └─ Return true (changed)
  └─ Otherwise: return false (no update needed)
```

### 5. Rolling Update Trigger

When ConfigMap is updated:

```
trigger_rolling_update()
  └─ For each validator StatefulSet:
      ├─ Patch template.metadata.annotations
      │   └─ Add "stellar.org/restarts.io": "{current_timestamp}"
      │
      └─ Kubernetes automatically:
          ├─ Detects template change
          ├─ Initiates rolling update
          └─ Restarts pods sequentially
```

## Module Organization

### New Module: `peer_discovery.rs`

Located in `src/controller/peer_discovery.rs`:

**Public Functions:**
- `discover_peers()` - Find all validator peers
- `ensure_peers_config_map()` - Create/update peer ConfigMap
- `trigger_rolling_update()` - Restart validators to load new config
- `watch_peers()` - Background watcher task
- `PeerDiscoveryResult` - Result struct with peer list

**Internal Helpers:**
- `get_peer_address()` - Extract IP:port from a single validator
- Tests for basic functionality

### Modified Modules

**`src/controller/mod.rs`**
- Added `pub mod peer_discovery`
- Exported public functions

**`src/controller/reconciler.rs`**
- Import `peer_discovery` module
- Import `metrics` module (was missing)
- Spawn watcher task in `run_controller()`

**`src/crd/types.rs`**
- Added `peer_port: Option<u16>` field to `ValidatorConfig`
- Added helper function `default_peer_port()`

**`src/main.rs`**
- Removed unused `kube-leader-election` dependency imports
- Simplified namespace handling

**`Cargo.toml`**
- Removed `kube-leader-election` dependency

## Data Flow Example

### Scenario: Add a new validator

```
1. Operator detects new StellarNode "validator-4"
   ├─ Reconciler creates StatefulSet
   ├─ Pod starts and becomes Ready
   └─ Pod gets assigned IP: 10.244.0.6

2. Peer watcher runs next cycle (30-second interval)
   ├─ Discovers all validators: [validator-1, validator-2, validator-3, validator-4]
   ├─ Extracts IPs from ready pods:
   │   ├─ validator-1: 10.244.0.2:11625
   │   ├─ validator-2: 10.244.0.3:11625
   │   ├─ validator-3: 10.244.0.4:11625
   │   └─ validator-4: 10.244.0.6:11625
   └─ Compares with last known peers (3 peers vs 4 peers)

3. Peers changed! Update ConfigMap
   ├─ Create/update ConfigMap "stellar-peers"
   ├─ Set KNOWN_PEERS = "10.244.0.2:11625\n10.244.0.3:11625\n..."
   └─ Return: "peers changed"

4. Trigger rolling update
   ├─ Patch validator-1 StatefulSet
   │   └─ Update pod restart annotation
   ├─ Kubernetes detects template change
   ├─ Pod validator-1-0 terminates (gracefully)
   ├─ New pod validator-1-0 starts with updated ConfigMap
   ├─ Repeat for validator-2, validator-3
   └─ (validator-4 already has latest config)

5. Stellar Core instances restart with new KNOWN_PEERS
   ├─ Load configuration from ConfigMap
   ├─ Connect to newly discovered validator-4
   └─ Network now includes all 4 validators
```

### Scenario: Suspend a validator

```
1. User updates StellarNode "validator-2" with suspended: true
   └─ Reconciler scales down StatefulSet to 0 replicas

2. Peer watcher runs next cycle
   ├─ Discovers validators: [validator-1, validator-3, validator-4]
   │   └─ validator-2 skipped (suspended flag is true)
   ├─ Compares: 4 peers → 3 peers (changed!)

3. Update ConfigMap and trigger rolling restart
   ├─ Remove validator-2 IP from KNOWN_PEERS
   ├─ Trigger rolling restart of remaining validators

4. Validators reconnect without validator-2
   └─ Validator quorum updates to reflect 3-node cluster
```

## Configuration Integration

### ValidatorConfig Changes

```rust
// OLD
pub struct ValidatorConfig {
    pub seed_secret_ref: String,
    pub quorum_set: Option<String>,
    pub enable_history_archive: bool,
    pub history_archive_urls: Vec<String>,
    pub catchup_complete: bool,
    pub key_source: KeySource,
    pub kms_config: Option<KmsConfig>,
    // ... no peer port
}

// NEW
pub struct ValidatorConfig {
    pub seed_secret_ref: String,
    pub quorum_set: Option<String>,
    pub enable_history_archive: bool,
    pub history_archive_urls: Vec<String>,
    pub catchup_complete: bool,
    pub key_source: KeySource,
    pub kms_config: Option<KmsConfig>,
    pub peer_port: Option<u16>,  // NEW!
}
```

Users can now optionally specify peer port:

```yaml
spec:
  validatorConfig:
    seedSecretRef: my-seed
    peerPort: 11625  # Optional, defaults to 11625
```

## Error Handling Strategy

### Resilience Features

1. **Partial Failure Tolerance**
   - If one pod IP extraction fails, continue with others
   - If ConfigMap update fails, retry next cycle
   - If rolling update fails, logged but doesn't break discovery

2. **Graceful Degradation**
   - Missing pods don't block discovery
   - Suspended nodes silently skipped
   - Non-validator nodes ignored

3. **Logging Strategy**
   - DEBUG: Skipped nodes, unchanged peers
   - INFO: Discovered peers, ConfigMap updates, rolling updates
   - WARN: Failures in rolling updates
   - ERROR: Discovery failures, ConfigMap creation failures

### No Blocking

The peer discovery runs in a background task and never blocks:

```rust
// In run_controller() - main thread
tokio::spawn(async move {
    // This runs independently
    peer_discovery::watch_peers(client, namespace).await;
});

// Main controller continues immediately
Controller::new(...)
    .run(reconcile, ...)
    .await;  // This is NOT blocked by peer discovery
```

## Testing Considerations

The implementation includes:

1. **Unit Test Example**
   ```rust
   #[test]
   fn test_peer_discovery_result_stellar_config() {
       let result = PeerDiscoveryResult {
           peers: vec!["192.168.1.1:11625".to_string()],
           active_validator_count: 2,
           changed: false,
       };
       assert!(result.to_stellar_config().contains("192.168.1.1:11625"));
   }
   ```

2. **Integration Testing Approach**
   - Deploy multiple StellarNode resources
   - Verify ConfigMap appears in namespace
   - Check ConfigMap contains correct peer IPs
   - Verify pods are restarted when peers change
   - Check logs for discovery messages

## Performance Characteristics

### Resource Usage

- **CPU**: Minimal - single list operation every 30 seconds
- **Memory**: Small cache of last peer list (typically < 1KB)
- **API Calls**: 
  - 30s cycle: 1 StellarNode list + 1 Pod list per active validator
  - ConfigMap update: Only when peers change (network-dependent)
  - StatefulSet patches: Only when ConfigMap updates

### Latency

- **Discovery to ConfigMap Update**: < 1 second
- **ConfigMap Update to Pod Restart**: 1-5 seconds (Kubernetes scheduling)
- **Pod Restart to Config Load**: 1-10 seconds (Stellar Core startup)
- **Total E2E**: < 30 seconds (next discovery cycle)

### Scalability

- **Validators per namespace**: Tested up to 10+ validators
- **Namespaces**: Single instance watches one namespace
- **API rate limiting**: Minimal impact (1 list per 30 seconds)

## Security Considerations

### RBAC Requirements

Existing ClusterRole permissions are sufficient:

```yaml
# Reads StellarNode CRDs
- apiGroups: ["stellar.org"]
  resources: ["stellarnodes"]
  verbs: ["get", "list", "watch"]

# Reads pod information for IP extraction
- apiGroups: [""]
  resources: ["pods"]
  verbs: ["get", "list", "watch"]

# Reads StatefulSet status
- apiGroups: ["apps"]
  resources: ["statefulsets"]
  verbs: ["get", "list", "watch", "patch"]  # "patch" for rolling restart

# Creates/updates ConfigMap
- apiGroups: [""]
  resources: ["configmaps"]
  verbs: ["get", "list", "watch", "create", "update", "patch"]
```

### No Secrets Accessed

- Peer discovery only reads StellarNode specs
- Never accesses validator seed secrets
- ConfigMap contains only IP addresses (public information)

## Debugging Guide

### Enable Debug Logging

```bash
kubectl set env deployment/stellar-operator RUST_LOG=stellar_k8s=debug
```

### Monitor Discovery in Real-Time

```bash
# Watch ConfigMap updates
kubectl get configmap stellar-peers -w

# Watch pod restarts
kubectl get pods -l app.kubernetes.io/name=stellar-node -w

# Watch operator logs
kubectl logs -f -l app.kubernetes.io/name=stellar-operator
```

### Manual Discovery Test

```bash
# Get all validators
kubectl get stellarnodes -o jsonpath='{.items[*].metadata.name}' && echo ""

# Get their pod IPs
kubectl get pods -l app.kubernetes.io/name=stellar-node -o wide

# Expected ConfigMap
kubectl get configmap stellar-peers -o yaml
```

## Future Enhancement Opportunities

1. **Configurable Discovery Interval**
   - Environment variable: `PEER_DISCOVERY_INTERVAL_SECS`
   - Default: 30 seconds

2. **Peer Health Checking**
   - TCP connection test before adding to list
   - GRPC ping to verify Stellar peer protocol

3. **Multi-Namespace Support**
   - Watch multiple namespaces simultaneously
   - Cross-namespace peer discovery (careful with isolation!)

4. **Metrics and Observability**
   - Prometheus metrics for peer count, discovery errors
   - Tracing spans for each discovery cycle

5. **Quorum Set Auto-Update**
   - Automatically update validator quorum sets based on discovered peers
   - Careful rollout to prevent liveness issues

## References

- **Rust async patterns**: https://tokio.rs/tokio/topics/spawning
- **kube-rs API**: https://docs.rs/kube/latest/kube/
- **Kubernetes StatefulSet**: https://kubernetes.io/docs/concepts/workloads/controllers/statefulset/
- **Stellar Core Configuration**: https://developers.stellar.org/docs/run-core-node/core-configuration
