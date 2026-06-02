# Quick Start Guide

Get your first Stellar validator node running on Kubernetes in minutes.

## Overview

This guide will help you deploy a basic Stellar testnet validator node using Stellar-K8s. You'll learn how to:

1. Create a StellarValidator custom resource
2. Verify the deployment
3. Check node status and logs
4. Connect to the Stellar testnet

!!! info "Prerequisites"
    Before starting, ensure you've completed the [Prerequisites](prerequisites.md) and [Installation](installation.md) guides.

## Step 1: Create Namespace

Create a dedicated namespace for your Stellar nodes:

```bash
kubectl create namespace stellar-testnet
```

## Step 2: Deploy a Validator Node

Create a validator configuration file:

```yaml title="testnet-validator.yaml"
apiVersion: stellar.k8s.io/v1alpha1
kind: StellarValidator
metadata:
  name: my-testnet-validator
  namespace: stellar-testnet
spec:
  network: testnet
  replicas: 1
  
  # Node configuration
  config:
    nodeIsValidator: true
    publicNetwork: true
    catchupRecent: 1024
    
  # Storage configuration
  storage:
    size: 500Gi
    storageClassName: standard
    
  # Resource requests and limits
  resources:
    requests:
      cpu: "4"
      memory: "8Gi"
    limits:
      cpu: "8"
      memory: "16Gi"
      
  # Monitoring
  monitoring:
    enabled: true
    serviceMonitor: true
```

Apply the configuration:

```bash
kubectl apply -f testnet-validator.yaml
```

## Step 3: Verify Deployment

### Check Validator Resource

```bash
kubectl get stellarvalidators -n stellar-testnet
```

Expected output:

```
NAME                    NETWORK   REPLICAS   READY   AGE
my-testnet-validator    testnet   1          1/1     2m30s
```

### Check Pod Status

```bash
kubectl get pods -n stellar-testnet -l app=my-testnet-validator
```

Wait for the pod to be in `Running` state:

```
NAME                                     READY   STATUS    RESTARTS   AGE
my-testnet-validator-0                   1/1     Running   0          3m
```

### Check PersistentVolumeClaim

```bash
kubectl get pvc -n stellar-testnet
```

Verify the volume is bound:

```
NAME                           STATUS   VOLUME      CAPACITY   ACCESS MODES   AGE
data-my-testnet-validator-0    Bound    pvc-xyz...  500Gi      RWO            3m
```

## Step 4: Monitor Node Startup

### View Pod Logs

```bash
kubectl logs -n stellar-testnet my-testnet-validator-0 -f
```

Look for successful startup messages:

```
2024-06-02T10:30:15.123 INFO [default] Node starting...
2024-06-02T10:30:16.456 INFO [Herder] Joining network: testnet
2024-06-02T10:30:20.789 INFO [Herder] Connected to peers
2024-06-02T10:30:25.012 INFO [History] Catching up to network
```

### Check Node Info

Get node information using kubectl exec:

```bash
kubectl exec -n stellar-testnet my-testnet-validator-0 -- \
  stellar-core --c info
```

## Step 5: Verify Network Connectivity

### Check Peer Connections

```bash
kubectl exec -n stellar-testnet my-testnet-validator-0 -- \
  stellar-core --c peers
```

You should see active peer connections:

```json
{
  "authenticated_peers": {
    "inbound": 5,
    "outbound": 8
  }
}
```

### Check Sync Status

```bash
kubectl exec -n stellar-testnet my-testnet-validator-0 -- \
  stellar-core --c 'll?level=info'
```

## Step 6: Access Node Metrics (Optional)

If monitoring is enabled, access Prometheus metrics:

```bash
# Port-forward to metrics endpoint
kubectl port-forward -n stellar-testnet my-testnet-validator-0 11626:11626

# In another terminal, query metrics
curl http://localhost:11626/metrics
```

## Step 7: Expose Node Externally (Optional)

To expose your validator for external peer connections, create a Service:

```yaml title="validator-service.yaml"
apiVersion: v1
kind: Service
metadata:
  name: my-testnet-validator-external
  namespace: stellar-testnet
spec:
  type: LoadBalancer
  selector:
    app: my-testnet-validator
  ports:
    - name: peer
      port: 11625
      targetPort: 11625
      protocol: TCP
```

Apply and get external IP:

```bash
kubectl apply -f validator-service.yaml
kubectl get svc -n stellar-testnet my-testnet-validator-external
```

## Common Operations

### Scale Validator Replicas

```bash
kubectl patch stellarvalidator my-testnet-validator \
  -n stellar-testnet \
  --type='merge' \
  -p '{"spec":{"replicas":3}}'
```

### Update Configuration

Edit the validator resource:

```bash
kubectl edit stellarvalidator my-testnet-validator -n stellar-testnet
```

### Restart Node

Delete the pod to trigger a restart:

```bash
kubectl delete pod -n stellar-testnet my-testnet-validator-0
```

The StatefulSet will automatically recreate it.

### View Events

```bash
kubectl get events -n stellar-testnet --sort-by='.lastTimestamp'
```

## Cleanup

To remove the validator:

```bash
# Delete the validator resource
kubectl delete stellarvalidator my-testnet-validator -n stellar-testnet

# Delete the namespace (removes all resources)
kubectl delete namespace stellar-testnet
```

!!! warning "Data Persistence"
    Deleting the namespace will also delete PersistentVolumeClaims. Ensure you've backed up any important data.

## Next Steps

Now that you have a basic validator running:

- [Configure High Availability](../tutorials/configure-ha-setup.md) for production deployments
- [Set up Monitoring](../deployment-guides/validator.md#monitoring) with Grafana dashboards
- [Deploy Horizon API](../deployment-guides/horizon.md) to query the network
- [Troubleshoot common issues](../troubleshooting/common-issues.md)

## Verification Checklist

Use this checklist to verify your deployment:

- [x] Validator resource created and shows READY 1/1
- [x] Pod is in Running state
- [x] PersistentVolumeClaim is Bound
- [x] Node logs show successful network connection
- [x] Peer connections are established (min 5 peers)
- [x] Node is syncing or caught up with the network
- [x] Metrics endpoint is accessible (if monitoring enabled)

!!! success "Congratulations!"
    You've successfully deployed your first Stellar validator node on Kubernetes! 🎉
