# Horizon API Deployment

Deploy and manage Horizon API servers for querying Stellar network data.

## Overview

Horizon provides a RESTful API interface to the Stellar network. It ingests data from Stellar Core and makes it available through HTTP endpoints.

## Basic Deployment

```yaml title="horizon-basic.yaml"
apiVersion: stellar.k8s.io/v1alpha1
kind: HorizonServer
metadata:
  name: horizon-api
  namespace: stellar
spec:
  network: mainnet
  replicas: 2
  
  # Stellar Core connection
  stellarCoreUrl: "http://validator-node:11626"
  
  # Database configuration
  database:
    url: "postgresql://horizon:password@postgres:5432/horizon"
    maxOpenConnections: 20
    maxIdleConnections: 10
    
  # Storage for ingestion
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
      
  # Service configuration
  service:
    type: LoadBalancer
    port: 8000
```

Apply the configuration:

```bash
kubectl apply -f horizon-basic.yaml
```

## Verify Deployment

```bash
# Check Horizon status
kubectl get horizonservers -n stellar

# Check pods
kubectl get pods -n stellar -l app=horizon-api

# Test API endpoint
kubectl port-forward -n stellar svc/horizon-api 8000:8000

# Query ledgers
curl http://localhost:8000/ledgers?order=desc&limit=5
```

## Next Steps

- Configure [Ingress](../configuration/operators.md) for external access
- Set up [monitoring](validator.md#monitoring)
- Optimize [database performance](../configuration/storage.md)
