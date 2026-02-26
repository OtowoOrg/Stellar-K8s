# Automated Horizon to Soroban RPC Migration

This guide explains how to use the automated migration feature to convert a running Horizon node to a Soroban RPC node with zero downtime.

## Overview

The migration controller automatically handles the transition from Horizon to Soroban RPC by:

1. **Parallel Execution**: Running both Horizon and Soroban RPC deployments simultaneously during migration
2. **Data Preservation**: Reusing existing storage and database configurations
3. **Zero Downtime**: Ensuring continuous service availability throughout the migration
4. **Automatic Cleanup**: Removing old Horizon resources once migration is complete

## Migration Process

### Prerequisites

- A running Horizon node managed by Stellar-K8s
- Sufficient cluster resources to run both deployments temporarily
- Database credentials that work for both Horizon and Soroban RPC

### Step 1: Prepare for Migration

Ensure your Horizon node is healthy and fully synced:

```bash
kubectl stellar status api-node -n stellar
```

Expected output:
```
NAME       TYPE     NETWORK   READY   REPLICAS   SYNCED
api-node   Horizon  Testnet   True    3/3        Yes
```

### Step 2: Initiate Migration

Update your StellarNode manifest to change the `nodeType` and add `sorobanConfig`:

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: api-node
  namespace: stellar
spec:
  nodeType: SorobanRpc  # Changed from Horizon
  network: Testnet
  version: "v21.0.0"
  replicas: 3
  storage:
    storageClass: "fast-ssd"
    size: "200Gi"
    retentionPolicy: Retain
  sorobanConfig:  # New configuration
    stellarCoreUrl: "http://core-validator:11626"
    enablePreflight: true
    maxEventsPerRequest: 10000
    captiveCoreStructuredConfig:
      networkPassphrase: "Test SDF Network ; September 2015"
      historyArchiveUrls:
        - "https://history.stellar.org/prd/core-testnet/core_testnet_001"
      peerPort: 11625
      httpPort: 11626
  database:
    secretKeyRef:
      name: "horizon-db-secret"
      key: "DATABASE_URL"
```

Apply the changes:

```bash
kubectl apply -f api-node.yaml
```

### Step 3: Monitor Migration

The operator will automatically:

1. Detect the node type change
2. Mark the migration as in progress
3. Create a new Soroban RPC deployment alongside the existing Horizon deployment
4. Wait for Soroban RPC to sync
5. Mark migration as complete
6. Clean up old Horizon resources

Monitor the migration status:

```bash
# Watch the migration progress
kubectl get stellarnode api-node -n stellar -w

# Check detailed status
kubectl describe stellarnode api-node -n stellar

# View operator logs
kubectl logs -n stellar-system -l app=stellar-operator -f
```

### Step 4: Verify Migration

Once complete, verify the Soroban RPC node is operational:

```bash
# Check node status
kubectl stellar status api-node -n stellar

# Test Soroban RPC endpoint
kubectl port-forward -n stellar svc/api-node 8000:8000
curl http://localhost:8000/health
```

## Migration Timeline

Typical migration timeline for a Testnet node:

- **T+0s**: Migration initiated, Soroban RPC deployment created
- **T+30s**: Soroban RPC pods starting
- **T+2m**: Soroban RPC syncing with network
- **T+5m**: Soroban RPC fully synced
- **T+5m30s**: Migration marked complete, Horizon resources cleaned up

For Mainnet nodes with full history, sync time may be longer.

## Configuration Migration

The migration controller automatically converts Horizon configuration to Soroban RPC:

| Horizon Config | Soroban RPC Config | Notes |
|----------------|-------------------|-------|
| `stellarCoreUrl` | `stellarCoreUrl` | Direct mapping |
| `databaseSecretRef` | Reused via `spec.database` | Same database can be used |
| `enableIngest` | N/A | Not applicable to Soroban RPC |
| `ingestWorkers` | N/A | Not applicable to Soroban RPC |

## Rollback

If you need to rollback to Horizon during migration:

```bash
# Edit the StellarNode to change nodeType back to Horizon
kubectl edit stellarnode api-node -n stellar

# Change:
#   nodeType: SorobanRpc
# Back to:
#   nodeType: Horizon

# And restore horizonConfig section
```

The operator will automatically handle the reverse migration.

## Troubleshooting

### Migration Stuck in Progress

If migration doesn't complete after 15 minutes:

```bash
# Check Soroban RPC pod logs
kubectl logs -n stellar -l app.kubernetes.io/instance=api-node -c stellar-node

# Check for resource constraints
kubectl top pods -n stellar

# Check events
kubectl get events -n stellar --sort-by='.lastTimestamp'
```

### Database Connection Issues

Ensure the database secret works for both Horizon and Soroban RPC:

```bash
# Verify secret exists
kubectl get secret horizon-db-secret -n stellar

# Test database connectivity
kubectl run -it --rm debug --image=postgres:16 --restart=Never -- \
  psql $(kubectl get secret horizon-db-secret -n stellar -o jsonpath='{.data.DATABASE_URL}' | base64 -d)
```

### Insufficient Resources

If pods fail to schedule during migration:

```bash
# Check node resources
kubectl top nodes

# Check pod status
kubectl get pods -n stellar -l app.kubernetes.io/instance=api-node

# Scale down temporarily if needed
kubectl scale deployment api-node -n stellar --replicas=1
```

## Best Practices

1. **Test in Non-Production First**: Always test migration on Testnet before Mainnet
2. **Monitor Resources**: Ensure sufficient CPU/memory for parallel deployments
3. **Backup Database**: Take a database snapshot before migration
4. **Plan Maintenance Window**: Although zero-downtime, plan for unexpected issues
5. **Update Monitoring**: Update dashboards and alerts for Soroban RPC metrics

## Advanced: Manual Migration

For more control, you can perform a manual migration:

```bash
# 1. Create a new Soroban RPC node with a different name
kubectl apply -f soroban-node.yaml

# 2. Wait for it to sync
kubectl wait --for=condition=Ready stellarnode/soroban-node -n stellar --timeout=600s

# 3. Update your application to use the new endpoint
# 4. Delete the old Horizon node
kubectl delete stellarnode api-node -n stellar
```

## See Also

- [Soroban RPC Documentation](https://developers.stellar.org/docs/data/rpc)
- [Health Checks Guide](health-checks.md)
- [Monitoring Guide](../monitoring/SOROBAN_DASHBOARD_GUIDE.md)
