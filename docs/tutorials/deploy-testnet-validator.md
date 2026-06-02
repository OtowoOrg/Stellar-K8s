# Tutorial: Deploy a Testnet Validator

Learn how to deploy a Stellar testnet validator node step-by-step.

## Prerequisites

- Kubernetes cluster with 4+ CPU cores and 8+ GB RAM available
- kubectl configured and connected to your cluster
- Stellar-K8s operator installed ([Installation Guide](../getting-started/installation.md))

## Step 1: Create Namespace

```bash
kubectl create namespace stellar-testnet
```

## Step 2: Generate Validator Keypair

```bash
docker run --rm stellar/stellar-core:latest stellar-core gen-seed
```

Save the output:
```
Secret seed: SBXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX
Public key: GDXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX
```

## Step 3: Create Secret

```bash
kubectl create secret generic validator-seed \
  --from-literal=seed='SBXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX' \
  -n stellar-testnet
```

## Step 4: Create Validator Configuration

```yaml title="testnet-validator.yaml"
apiVersion: stellar.k8s.io/v1alpha1
kind: StellarValidator
metadata:
  name: testnet-validator
  namespace: stellar-testnet
spec:
  network: testnet
  replicas: 1
  
  config:
    nodeIsValidator: true
    publicNetwork: true
    nodeSeed: "secret://validator-seed"
    catchupRecent: 1024
    
    # Testnet quorum configuration
    quorumSet:
      threshold: 2
      validators:
        - "GDKXE2OZMJIPOSLNA6N6F2BVCI3O777I2OOC4BV7VOYUEHYX7RTRYA7Y"  # SDF testnet 1
        - "GCUCJTIYXSOXKBSNFGNFWW5MUQ54HKRPGJUTQFJ5RQXZXNOLNXYDHRAP"  # SDF testnet 2
        - "GC2V2EFSXN6SQTWVYA5EPJPBWWIMSD2XQNKUOHGEKB535AQE2I6IXV2Z"  # SDF testnet 3
        - "$self"
  
  storage:
    size: 100Gi
    storageClassName: standard
    
  resources:
    requests:
      cpu: "2"
      memory: "4Gi"
    limits:
      cpu: "4"
      memory: "8Gi"
      
  monitoring:
    enabled: true
```

## Step 5: Deploy

```bash
kubectl apply -f testnet-validator.yaml
```

## Step 6: Verify Deployment

```bash
# Check validator status
kubectl get stellarvalidators -n stellar-testnet

# Check pod
kubectl get pods -n stellar-testnet

# View logs
kubectl logs -n stellar-testnet testnet-validator-0 -f
```

Look for:
```
INFO [Herder] Joining network: testnet
INFO [Herder] Connected to peers
```

## Step 7: Check Sync Status

```bash
kubectl exec -n stellar-testnet testnet-validator-0 -- \
  stellar-core --c info
```

## Expected Outcomes

✅ Validator pod is Running
✅ Connected to 5+ peers
✅ Syncing with testnet (or caught up)
✅ Metrics endpoint accessible

## Troubleshooting

### Pod not starting
Check events:
```bash
kubectl describe pod -n stellar-testnet testnet-validator-0
```

### No peer connections
Verify network connectivity:
```bash
kubectl exec -n stellar-testnet testnet-validator-0 -- nc -zv history.stellar.org 443
```

### Slow sync
This is normal - initial catchup can take 30+ minutes.

## Next Steps

- [Configure high availability](configure-ha-setup.md)
- [Set up monitoring dashboards](../deployment-guides/validator.md#monitoring)
- [Deploy Horizon API](../deployment-guides/horizon.md)

!!! success "Congratulations!"
    Your testnet validator is now running and participating in consensus! 🎉
