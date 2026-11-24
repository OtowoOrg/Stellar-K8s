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

## Contract Deployment

Install the contract deployment CRDs with the operator manifests, then create a
`ContractWASM` and its `ContractInstance` in the same namespace. The instance
waits for the WASM upload status before submitting its creation transaction.

The operator deliberately does not sign transactions or read Stellar secret
seeds. Prepare and sign the upload, contract creation, and storage-write XDR
envelopes outside the cluster. `wasmLedgerKeyXdr` and
`contractLedgerKeyXdr` are the ledger keys used to detect existing code or
instances; each storage entry's `keyXdr` is checked before its signed write is
submitted. The controller verifies the downloaded artifact against `sha256`
before it submits the upload envelope. Ensure each signed envelope targets that
artifact, contract ID, and storage value, and remains valid when the operator
submits it.

Use `config/samples/soroban-contract-deployment.yaml` as a template. Replace
all `REPLACE_WITH_...` values and the example artifact URL, then apply both
resources together:

```bash
kubectl apply -f config/samples/soroban-contract-deployment.yaml
kubectl get contractwasm,contractinstance -n stellar
kubectl get contractinstance hello-contract -n stellar \
  -o jsonpath='{.status.contractId}{"\n"}{.status.phase}{"\n"}'
```

Re-applying the same resources is idempotent: transaction hashes are tracked in
status, and existing code, contract-instance, and storage ledger entries are
checked before a transaction is submitted. Transaction failures are reported
in `status.lastError`; replace or recreate a failed resource after correcting
its signed envelope.
