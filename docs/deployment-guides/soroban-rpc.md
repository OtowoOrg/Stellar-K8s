# Soroban RPC Deployment

Deploy Soroban RPC nodes for smart contract interaction.

## Overview

Soroban RPC provides JSON-RPC endpoints for interacting with Soroban smart contracts on the Stellar network.

## Basic Deployment

```yaml title="soroban-rpc-basic.yaml"
apiVersion: stellar.k8s.io/v1alpha1
kind: SorobanRPC
metadata:
  name: soroban-rpc
  namespace: stellar
spec:
  network: mainnet
  replicas: 2
  
  # Stellar Core connection
  stellarCoreUrl: "http://validator-node:11626"
  
  # Storage
  storage:
    size: 200Gi
    storageClassName: standard
    
  # Resources
  resources:
    requests:
      cpu: "2"
      memory: "4Gi"
    limits:
      cpu: "4"
      memory: "8Gi"
      
  # Service
  service:
    type: LoadBalancer
    port: 8000
```

Apply and verify:

```bash
kubectl apply -f soroban-rpc-basic.yaml
kubectl get sorobanrpcs -n stellar
```

## Test RPC Endpoint

```bash
# Port forward
kubectl port-forward -n stellar svc/soroban-rpc 8000:8000

# Test getHealth
curl -X POST http://localhost:8000 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}'
```
