# Quick Start: Peer Discovery Feature

## What's New

The Stellar-K8s operator now includes **dynamic peer discovery** - validators automatically find each other!

## Quick Facts

- ✅ Zero configuration needed (works out of the box)
- ✅ Automatic peer detection every 30 seconds
- ✅ Validators restart automatically when peers change
- ✅ No manual peer configuration required
- ✅ Supports custom peer ports

## 1-Minute Setup

### Deploy Multiple Validators

```bash
# Create seed secrets
kubectl create secret generic validator1-seed --from-literal=STELLAR_CORE_SEED=SBXYZ...
kubectl create secret generic validator2-seed --from-literal=STELLAR_CORE_SEED=SBABCD...

# Deploy validators
kubectl apply -f examples/multi-validator-with-peer-discovery.yaml
```

### Watch It Work

```bash
# Terminal 1: Monitor peer discovery
kubectl logs -f deployment/stellar-operator | grep "peer discovery"

# Terminal 2: Watch peer ConfigMap updates
kubectl get configmap stellar-peers -o jsonpath='{.data.KNOWN_PEERS}' && sleep 3 && clear && kubectl get configmap stellar-peers -o jsonpath='{.data.KNOWN_PEERS}' && echo ""

# Terminal 3: Watch pod restarts
kubectl get pods -w -l app.kubernetes.io/name=stellar-node
```

## How It Works

1. **Discovery** (every 30 seconds)
   - Operator finds all running validators
   - Extracts their IP:port addresses
   - Detects if peer list changed

2. **Update** (when peers change)
   - ConfigMap `stellar-peers` is updated
   - Contains all validator peer addresses

3. **Restart** (automatic)
   - Pods are restarted with rolling update
   - New peers are loaded on startup

4. **Connection**
   - Validators read peer list from ConfigMap
   - Connect to discovered peers
   - Build quorum

## Verify It's Working

```bash
# Check if ConfigMap exists
kubectl get configmap stellar-peers

# View discovered peers
kubectl get configmap stellar-peers -o jsonpath='{.data.KNOWN_PEERS}' | column

# Check peer count
kubectl get configmap stellar-peers -o jsonpath='{.data.discovery_metadata}' | grep peer_count

# Monitor discovery in logs
kubectl logs -f deployment/stellar-operator | grep "Peer discovery"
```

## Configuration Options

### Custom Peer Port

If your validators don't use port 11625:

```yaml
spec:
  validatorConfig:
    seedSecretRef: my-seed
    peerPort: 12625  # Custom port
```

## Common Tasks

### Add a New Validator

```bash
kubectl apply -f validator-4.yaml
# The operator will:
# 1. Create the validator StatefulSet (within 2 seconds)
# 2. Pod starts and becomes ready (within 10 seconds)
# 3. Peer discovery finds it (within 30 seconds)
# 4. ConfigMap is updated (within 1 second)
# 5. Other validators restart (within 5 seconds)
# 6. All validators connect to the new validator
```

### Remove a Validator

```bash
kubectl delete stellarnode validator-3
# The operator will:
# 1. Scale down the StatefulSet
# 2. Peer discovery detects the missing validator (within 30 seconds)
# 3. ConfigMap is updated (within 1 second)
# 4. Other validators restart (within 5 seconds)
# 5. Validator is removed from peer list
```

### Suspend a Validator (without deleting)

```bash
kubectl patch stellarnode validator-3 --type merge -p '{"spec":{"suspended":true}}'
# Same result as above, but validator resources are kept for quick restart
```

### Resume a Validator

```bash
kubectl patch stellarnode validator-3 --type merge -p '{"spec":{"suspended":false}}'
# Validator is discovered again and peers restart
```

## Troubleshooting

### ConfigMap not appearing

```bash
# Check operator logs
kubectl logs -f deployment/stellar-operator | grep -i error

# Check if validators are running
kubectl get stellarnodes -o wide

# Check if pods are ready
kubectl get pods -l app.kubernetes.io/name=stellar-node
```

### Peers not updating

```bash
# Check discovery is running
kubectl logs deployment/stellar-operator | grep "Starting peer discovery"

# Check ConfigMap content
kubectl get configmap stellar-peers -o yaml

# Check pod logs for ConfigMap mount
kubectl exec validator-1-0 -- ls -la /config/
```

### Pods not restarting

```bash
# Check StatefulSet annotations
kubectl get statefulset validator-1 -o jsonpath='{.spec.template.metadata.annotations}' | jq

# Verify operator has patch permissions
kubectl auth can-i patch statefulsets --as=system:serviceaccount:default:stellar-operator
```

## What Gets Created

When you deploy the example:

```
stellar-peers ConfigMap
  ├─ KNOWN_PEERS = "10.0.0.1:11625\n10.0.0.2:11625\n..."
  └─ discovery_metadata = "timestamp, counts..."

validator-1 StatefulSet → Pod → Stellar Core running
validator-2 StatefulSet → Pod → Stellar Core running
validator-3 StatefulSet → Pod → Stellar Core running

Each pod reads KNOWN_PEERS from ConfigMap on startup
```

## Performance

- **Discovery frequency**: Every 30 seconds
- **API calls**: Minimal (1 list per cycle)
- **Latency**: < 30 seconds for new validators to appear
- **Resource impact**: Negligible

## Next Steps

1. **Deploy the example**: `kubectl apply -f examples/multi-validator-with-peer-discovery.yaml`
2. **Monitor it**: `kubectl logs -f deployment/stellar-operator | grep peer`
3. **Experiment**: Add/remove validators and watch them discover each other
4. **Scale up**: Deploy dozens of validators - all automatic discovery!

## Advanced

For detailed information:
- User guide: [PEER_DISCOVERY.md](docs/PEER_DISCOVERY.md)
- Technical docs: [PEER_DISCOVERY_INTEGRATION.md](docs/PEER_DISCOVERY_INTEGRATION.md)
- Source code: [src/controller/peer_discovery.rs](src/controller/peer_discovery.rs)

## Support

- Check logs: `kubectl logs deployment/stellar-operator`
- Check ConfigMap: `kubectl describe configmap stellar-peers`
- Check pods: `kubectl get pods -w`
- Check events: `kubectl get events`
