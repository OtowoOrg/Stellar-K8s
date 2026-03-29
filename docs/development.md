# Local Development with k3d

This guide walks you through setting up a fully functional Stellar development environment on your local machine using [k3d](https://k3d.io) — K3s running inside Docker. No cloud account required.

---

## Prerequisites

Install the following tools before proceeding:

| Tool | Version | Install |
|------|---------|---------|
| Docker | 24.0+ | [docs.docker.com](https://docs.docker.com/get-docker/) |
| k3d | 5.6+ | `curl -s https://raw.githubusercontent.com/k3d-io/k3d/main/install.sh \| bash` |
| kubectl | 1.28+ | [kubernetes.io/docs](https://kubernetes.io/docs/tasks/tools/) |
| Helm | 3.x | `curl https://raw.githubusercontent.com/helm/helm/main/scripts/get-helm-3 \| bash` |

Verify everything is in place:

```bash
docker version
k3d version
kubectl version --client
helm version
```

> **Docker resource requirements**: Stellar nodes are memory and I/O intensive. Before creating the cluster, ensure Docker has sufficient resources allocated. On Docker Desktop (Mac/Windows), go to **Settings → Resources** and set at minimum:
> - **CPUs**: 4
> - **Memory**: 8 GB
> - **Disk**: 40 GB

---

## Step 1: Create the k3d Cluster

The following command creates a single-node k3d cluster with port mappings for all Stellar services and a local image registry:

```bash
k3d cluster create stellar-dev \
  --agents 1 \
  --k3s-arg "--disable=traefik@server:0" \
  -p "8000:30000@loadbalancer" \
  -p "8001:30001@loadbalancer" \
  -p "8080:30080@loadbalancer" \
  --registry-create stellar-registry:0.0.0.0:5050 \
  --k3s-arg "--kubelet-arg=eviction-hard=memory.available<512Mi@agent:0"
```

Port mapping reference:

| Local Port | NodePort | Service |
|------------|----------|---------|
| `8000` | `30000` | Horizon RPC / REST API |
| `8001` | `30001` | Stellar Core peer port |
| `8080` | `30080` | Soroban RPC |

Confirm the cluster is running:

```bash
k3d cluster list
kubectl get nodes
```

Expected output:
```
NAME                        STATUS   ROLES                  AGE
k3d-stellar-dev-server-0    Ready    control-plane,master   30s
k3d-stellar-dev-agent-0     Ready    <none>                 25s
```

---

## Step 2: Install the Stellar Operator

### 2a. Apply the CRD

```bash
kubectl apply -f config/crd/stellarnode-crd.yaml
```

### 2b. Install via Helm

```bash
# Create the operator namespace
kubectl create namespace stellar-system

# Install the operator chart from the local charts directory
helm install stellar-operator ./charts/stellar-operator \
  --namespace stellar-system \
  --set image.repository=ghcr.io/stellar/stellar-k8s \
  --set image.tag=latest \
  --wait
```

Verify the operator pod is running:

```bash
kubectl get pods -n stellar-system
```

```
NAME                                READY   STATUS    RESTARTS   AGE
stellar-operator-7d9f8b6c4-xk2pq   1/1     Running   0          45s
```

---

## Step 3: Deploy a Testnet Node

### 3a. Create the validator seed secret

```bash
kubectl create namespace stellar

# Replace <your-seed> with a valid Stellar secret key (starts with S...)
kubectl create secret generic validator-seed \
  --from-literal=seed=<your-seed> \
  -n stellar
```

For local testing you can generate a throwaway keypair using the [Stellar Laboratory](https://laboratory.stellar.org/#account-creator?network=test).

### 3b. Apply a StellarNode manifest

Create `testnet-validator.yaml`:

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: testnet-validator
  namespace: stellar
spec:
  nodeType: Validator
  network: Testnet
  version: "v21.0.0"
  replicas: 1
  serviceConfig:
    peerNodePort: 30001   # maps to localhost:8001
    httpNodePort: 30000   # maps to localhost:8000
  resources:
    requests:
      cpu: "500m"
      memory: "1Gi"
    limits:
      cpu: "2"
      memory: "4Gi"
  storage:
    storageClass: "local-path"   # k3d's built-in provisioner
    size: "20Gi"
    retentionPolicy: Delete
  validatorConfig:
    seedSecretRef: "validator-seed"
    enableHistoryArchive: false   # disable for faster local startup
```

Apply it:

```bash
kubectl apply -f testnet-validator.yaml
```

### 3c. Verify pods are running

```bash
# Watch pods come up
kubectl get pods -n stellar -w

# Check the StellarNode status
kubectl get stellarnodes -n stellar

# Detailed status
kubectl describe stellarnode testnet-validator -n stellar
```

Once syncing, the `READY` column will show `True` and `STATUS` will show `Synced`.

---

## Step 4: Access Services Locally

Once the node is running, all services are reachable on `localhost` via the port mappings configured in Step 1.

### Horizon API

```bash
# Check Horizon health
curl http://localhost:8000/

# Query the latest ledger
curl http://localhost:8000/ledgers?order=desc&limit=1
```

### Soroban RPC (if deployed)

```bash
curl -X POST http://localhost:8080 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}'
```

### Point Freighter Wallet to the local cluster

1. Open Freighter → **Settings → Network**
2. Click **Add Custom Network**
3. Fill in:
   - **Network Name**: `k3d-local`
   - **Horizon URL**: `http://localhost:8000`
   - **Network Passphrase**: `Test SDF Network ; September 2015`
4. Select `k3d-local` as the active network

### Point a Stellar SDK to the local cluster

```javascript
// JavaScript SDK
import { Horizon } from "@stellar/stellar-sdk";
const server = new Horizon.Server("http://localhost:8000", { allowHttp: true });
```

```python
# Python SDK
from stellar_sdk import Server
server = Server("http://localhost:8000")
```

---

## Troubleshooting

### Docker is out of memory

**Symptom**: Pods stuck in `Pending` or `OOMKilled`.

```bash
# Check node resource pressure
kubectl describe node k3d-stellar-dev-agent-0 | grep -A5 "Conditions:"
```

**Fix**: Increase Docker memory to at least 8 GB in Docker Desktop settings, then recreate the cluster.

---

### Port already in use

**Symptom**: `k3d cluster create` fails with `address already in use`.

```bash
# Find what's using the port
lsof -i :8000
```

**Fix**: Stop the conflicting process, or change the local port in the `k3d cluster create` command (e.g., `-p "18000:30000@loadbalancer"`) and update your SDK/wallet config accordingly.

---

### Pods stuck in `Pending` — no storage

**Symptom**: `kubectl describe pod` shows `no persistent volumes available`.

**Fix**: k3d ships with the `local-path` storage provisioner. Confirm it's running:

```bash
kubectl get pods -n kube-system | grep local-path
```

If missing, install it manually:

```bash
kubectl apply -f https://raw.githubusercontent.com/rancher/local-path-provisioner/master/deploy/local-path-storage.yaml
```

---

### Operator pod in `CrashLoopBackOff`

```bash
kubectl logs -n stellar-system deploy/stellar-operator --previous
```

Common causes:
- CRD not applied before the operator started → re-apply `config/crd/stellarnode-crd.yaml`
- Missing RBAC permissions → reinstall the Helm chart with `--wait`

---

### Slow ledger sync

Testnet sync can take 10–30 minutes on first boot. Monitor progress:

```bash
kubectl logs -n stellar stellarnode-testnet-validator-0 -f | grep "ledger"
```

To speed things up, set `enableHistoryArchive: false` in the spec (as shown in Step 3b) to skip archive verification during initial sync.

---

## Cleanup

Delete the cluster and free all local resources:

```bash
k3d cluster delete stellar-dev
```

This removes all containers, volumes, and port mappings associated with the cluster. Your local Docker images are preserved.

To also remove the local registry created in Step 1:

```bash
k3d registry delete stellar-registry
```
