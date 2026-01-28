# Quick Start: Dynamic Peer Discovery

Get your Stellar validators discovering each other automatically in 5 minutes.

## Prerequisites

- Stellar-K8s operator installed and running
- At least one StellarNode resource (Validator type)
- kubectl access to your cluster

## Step 1: Deploy Multiple Validators

Create validators in your cluster. The operator will automatically discover them:

```bash
kubectl apply -f examples/peer-discovery-example.yaml
```

This creates:
- 3 validator nodes (validator-1, validator-2, validator-3)
- Shared peers ConfigMap in stellar-system namespace
- Required secrets for validator seeds

## Step 2: Verify Peer Discovery

Check if peers are being discovered:

```bash
# View the shared peers ConfigMap
kubectl get configmap stellar-peers -n stellar-system -o yaml

# Check peer count
kubectl get configmap stellar-peers -n stellar-system -o jsonpath='{.data.peer_count}'

# View peers as JSON
kubectl get configmap stellar-peers -n stellar-system -o jsonpath='{.data.peers\.json}' | jq
```

Expected output:
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
  ...
]
```

## Step 3: Monitor Peer Discovery

Watch the operator logs to see peer discovery in action:

```bash
# Watch peer discovery logs
kubectl logs -f deployment/stellar-operator -n stellar-system | grep "peer discovery"

# View all peer-related events
kubectl logs -f deployment/stellar-operator -n stellar-system | grep -i peer
```

Expected log output:
```
INFO stellar_k8s::controller::peer_discovery: Starting peer discovery watcher
INFO stellar_k8s::controller::peer_discovery: New peer discovered: stellar-nodes/validator-1 at 10.0.1.5:11625
INFO stellar_k8s::controller::peer_discovery: New peer discovered: stellar-nodes/validator-2 at 10.0.1.6:11625
INFO stellar_k8s::controller::peer_discovery: Updated peers ConfigMap with 2 peers
```

## Step 4: Verify Config Reload

Check if validators are reloading configuration with new peers:

```bash
# Check config-reload logs
kubectl logs -f deployment/stellar-operator -n stellar-system | grep "config-reload"

# Check validator pod logs
kubectl logs -f <validator-pod> -n stellar-nodes | grep "config-reload"
```

## Step 5: Add a New Validator

Create a new validator and watch it get discovered automatically:

```bash
kubectl apply -f - <<EOF
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: validator-4
  namespace: stellar-nodes
spec:
  nodeType: Validator
  network: Testnet
  version: "v21.0.0"
  replicas: 1
  resources:
    requests:
      cpu: "2"
      memory: "8Gi"
    limits:
      cpu: "4"
      memory: "16Gi"
  storage:
    storageClass: "standard"
    size: "500Gi"
  validatorConfig:
    seedSecretRef: "validator-4-seed"
EOF
```

Watch the peer count increase:

```bash
watch kubectl get configmap stellar-peers -n stellar-system -o jsonpath='{.data.peer_count}'
```

## Step 6: Test Peer Connectivity

Verify validators can reach each other:

```bash
# Get a validator pod
POD=$(kubectl get pods -n stellar-nodes -l app=validator-1 -o jsonpath='{.items[0].metadata.name}')

# Test connectivity to another validator
kubectl exec -it $POD -n stellar-nodes -- \
  curl http://validator-2-service.stellar-nodes:11625/info

# Check Stellar Core info
kubectl exec -it $POD -n stellar-nodes -- \
  curl http://localhost:11626/http-command?admin=true&command=info | jq
```

## Troubleshooting

### Peers Not Discovered

```bash
# Check if validators are running
kubectl get stellarnodes -A

# Check service IPs
kubectl get svc -A -l app.kubernetes.io/component=stellar-node

# Check operator logs
kubectl logs deployment/stellar-operator -n stellar-system | grep "peer discovery"
```

### Config Reload Not Triggering

```bash
# Check validator health
kubectl get stellarnodes -A -o wide

# Check pod status
kubectl get pods -A -l app.kubernetes.io/component=stellar-node

# Test HTTP endpoint
kubectl exec <validator-pod> -- curl http://localhost:11626/http-command?admin=true&command=info
```

### ConfigMap Not Updating

```bash
# Check if ConfigMap exists
kubectl get configmap stellar-peers -n stellar-system

# Check operator RBAC permissions
kubectl get clusterrole stellar-operator -o yaml | grep -A 20 "configmaps"

# Check watcher status
kubectl logs deployment/stellar-operator -n stellar-system | grep "watcher"
```

## Next Steps

- Read the [full peer discovery documentation](peer-discovery.md)
- Configure custom peer discovery settings
- Set up monitoring and alerting for peer count
- Integrate with your CI/CD pipeline

## Common Tasks

### Query Peers Programmatically

```bash
# Get peers as JSON
kubectl get configmap stellar-peers -n stellar-system -o jsonpath='{.data.peers\.json}' | jq '.[] | .peerString'

# Get peer count
kubectl get configmap stellar-peers -n stellar-system -o jsonpath='{.data.peer_count}'

# Get specific peer info
kubectl get configmap stellar-peers -n stellar-system -o jsonpath='{.data.peers\.json}' | jq '.[] | select(.name=="validator-1")'
```

### Monitor Peer Changes

```bash
# Watch for ConfigMap updates
kubectl get configmap stellar-peers -n stellar-system -w

# Watch peer count changes
watch -n 5 'kubectl get configmap stellar-peers -n stellar-system -o jsonpath="{.data.peer_count}"'
```

### Export Peers for External Use

```bash
# Export peers as environment variable
export STELLAR_PEERS=$(kubectl get configmap stellar-peers -n stellar-system -o jsonpath='{.data.peers\.txt}')
echo $STELLAR_PEERS

# Export as JSON
kubectl get configmap stellar-peers -n stellar-system -o jsonpath='{.data.peers\.json}' > peers.json
```

## Performance Tips

1. **Peer Port**: Default is 11625. Ensure this port is open between validators.
2. **ConfigMap Size**: With many validators, the ConfigMap can grow. Monitor its size.
3. **Config Reload Frequency**: Happens once per reconciliation cycle (60s when ready).
4. **Network Latency**: Peer discovery uses Kubernetes watch API, which is efficient.

## Security Notes

- Peer IPs are internal cluster IPs (not exposed externally)
- ConfigMap is not encrypted by default (consider encryption at rest)
- Peer discovery requires RBAC permissions to list nodes and services
- Config reload uses internal pod IP (no external network access)

## Support

For issues or questions:
1. Check the [full documentation](peer-discovery.md)
2. Review operator logs: `kubectl logs deployment/stellar-operator -n stellar-system`
3. Check StellarNode status: `kubectl describe stellarnode <name> -n stellar-nodes`
