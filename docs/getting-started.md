# Getting Started: Running Stellar-K8s on Minikube

This guide walks you through running the Stellar-K8s operator on a local [Minikube](https://minikube.sigs.k8s.io/) cluster. Minikube is the most widely used local Kubernetes environment and works on Linux, macOS, and Windows.

> For Kind-based local development, see [quickstart.md](quickstart.md).

---

## Prerequisites

| Tool      | Version   | Install |
|-----------|-----------|---------|
| Rust      | stable (≥ 1.88) | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| Docker    | ≥ 24      | [docs.docker.com](https://docs.docker.com/get-docker/) |
| Minikube  | ≥ 1.32    | [minikube.sigs.k8s.io](https://minikube.sigs.k8s.io/docs/start/) |
| kubectl   | ≥ 1.28    | [kubernetes.io/docs](https://kubernetes.io/docs/tasks/tools/) |
| Helm      | ≥ 3.14    | `curl https://raw.githubusercontent.com/helm/helm/main/scripts/get-helm-3 \| bash` |

Verify everything is installed:

```bash
rustc --version
docker version
minikube version
kubectl version --client
helm version
```

---

## Driver Selection

Minikube supports several hypervisor drivers. Choose the one that matches your OS:

| Driver      | OS               | Notes |
|-------------|------------------|-------|
| `docker`    | Linux/macOS/Windows | Recommended — no separate VM, uses the Docker daemon |
| `hyperkit`  | macOS            | Native macOS hypervisor; lower overhead than Docker |
| `hyperv`    | Windows (Pro/Ent) | Native Windows hypervisor |
| `wsl2`      | Windows (WSL2)   | Runs Minikube inside WSL2; integrates with Windows Docker Desktop |

The examples in this guide use `--driver=docker`. Adjust accordingly for your environment.

---

## Resource Requirements

Running the operator alongside a Testnet node requires:

| Resource | Minimum | Recommended |
|----------|---------|-------------|
| CPU      | 2 cores | 4 cores     |
| RAM      | 4 GB    | 8 GB        |
| Disk     | 20 GB   | 40 GB       |

Allocate insufficient resources and you will see pods stuck in `Pending` or `OOMKilled`.

---

## Step 1 — Clone the Repository

```bash
git clone https://github.com/OtowoOrg/Stellar-K8s.git
cd Stellar-K8s
```

## Step 2 — Build the Operator

```bash
cargo build --release --locked
```

Verify the binary works:

```bash
./target/release/stellar-operator version
```

## Step 3 — Start Minikube

Start a cluster with the recommended resource allocation:

```bash
minikube start \
  --cpus=4 \
  --memory=8192 \
  --disk-size=40g \
  --driver=docker
```

Confirm the cluster is running:

```bash
kubectl cluster-info
kubectl get nodes
```

You should see a single node in `Ready` state.

## Step 4 — Install the CRD

```bash
kubectl apply -f config/crd/stellarnode-crd.yaml
```

Verify the CRD is registered:

```bash
kubectl get crd stellarnodes.stellar.org
```

## Step 5 — Build and Load the Operator Image

Minikube runs its own Docker daemon inside the VM. Point your local Docker client at it so the image is built directly into Minikube's registry:

```bash
# Point the local Docker CLI at Minikube's Docker daemon
eval $(minikube docker-env)

# Build the image (it lands directly in Minikube — no push needed)
docker build -t stellar-operator:dev .
```

If you prefer not to switch your Docker context, use the image load command instead:

```bash
# Build with the default Docker daemon, then copy into Minikube
docker build -t stellar-operator:dev .
minikube image load stellar-operator:dev
```

## Step 6 — Deploy the Operator

```bash
kubectl create namespace stellar-system

helm upgrade --install stellar-operator charts/stellar-operator \
  --namespace stellar-system \
  --set image.tag=dev \
  --set image.pullPolicy=Never \
  --wait
```

Confirm the operator pod is running:

```bash
kubectl get pods -n stellar-system
kubectl logs -n stellar-system -l app.kubernetes.io/name=stellar-operator --tail=20
```

## Step 7 — Create a Sample StellarNode

```bash
kubectl apply -f config/samples/test-stellarnode.yaml
```

Watch the operator reconcile it:

```bash
kubectl get stellarnode -n stellar-system -w
```

Check the resources the operator created:

```bash
kubectl get deploy,sts,svc,pvc,cm -n stellar-system \
  -l app.kubernetes.io/managed-by=stellar-operator
```

## Step 8 — Verify Health

```bash
# Port-forward the operator REST API
kubectl port-forward -n stellar-system svc/stellar-operator 8080:8080 &

# Health check
curl http://localhost:8080/health

# Leader status
curl http://localhost:8080/leader

# Prometheus metrics (requires metrics feature, enabled by default)
curl http://localhost:8080/metrics
```

---

## LoadBalancer Services and `minikube tunnel`

Minikube does not assign external IPs to `LoadBalancer` services by default — they remain in `<pending>`. To give LoadBalancer services a routable IP on your local machine, run `minikube tunnel` in a **separate terminal**:

```bash
# Keep this running in a dedicated terminal while you work
minikube tunnel
```

On Linux and macOS this requires `sudo` (it creates a local route). On Windows, run the terminal as Administrator.

Once the tunnel is active:

```bash
kubectl get svc -n stellar-system
# EXTERNAL-IP now shows 127.0.0.1 (or similar) instead of <pending>
```

If you only need to reach the operator's REST API for local testing, `kubectl port-forward` (Step 8) is simpler and does not require the tunnel.

---

## Persistent Volumes on Minikube

Minikube ships a built-in `standard` StorageClass backed by `hostPath` volumes on the Minikube VM. This is sufficient for development and testing.

Reference it in your `StellarNode` manifest:

```yaml
spec:
  storage:
    storageClass: "standard"
    size: "10Gi"
    retentionPolicy: Delete
```

List available storage classes:

```bash
kubectl get storageclass
```

**Data persistence note:** `hostPath` volumes live inside the Minikube VM. Running `minikube delete` permanently destroys all data. Use `minikube stop` (not `delete`) to pause the cluster and preserve your volumes between sessions.

For production-like storage testing with local NVMe paths, see the [LocalStorage section in the README](../README.md#high-performance-local-storage-nvme).

---

## Cleanup

Stop the cluster (preserves data and cluster state):

```bash
minikube stop
```

Delete the cluster entirely (removes all data):

```bash
minikube delete
```

---

## Troubleshooting

**Minikube fails to start: insufficient resources**

```
Error: insufficient memory (requested 8192MB, available NNNNmb)
```

Lower the memory flag or free RAM on your host:

```bash
minikube start --cpus=2 --memory=4096 --disk-size=20g --driver=docker
```

**Driver conflict after changing drivers**

Delete the existing profile and start fresh:

```bash
minikube delete
minikube start --driver=docker
```

**LoadBalancer service stuck in `<pending>`**

Run `minikube tunnel` in a separate terminal (see above). If you are on Windows, ensure the terminal has Administrator privileges.

**PVC stuck in `Pending`**

Check the StorageClass name matches what Minikube provides:

```bash
kubectl get storageclass
# Look for "standard" with provisioner "k8s.io/minikube-hostpath"
```

Update the `storageClass` field in your `StellarNode` manifest to match.

**Operator pod in `CrashLoopBackOff`**

```bash
kubectl logs -n stellar-system \
  -l app.kubernetes.io/name=stellar-operator --previous
```

**Image pull errors (`ErrImageNeverPull`)**

Ensure you either built inside Minikube's Docker daemon or loaded the image explicitly:

```bash
minikube image load stellar-operator:dev
```

---

## Next Steps

- Read [DEVELOPMENT.md](../DEVELOPMENT.md) for contributor workflows
- See [quickstart.md](quickstart.md) for the Kind-based equivalent
- Explore [examples/](../examples/) for advanced `StellarNode` configurations
- See [docs/health-checks.md](health-checks.md) for health check configuration
- See [docs/peer-discovery.md](peer-discovery.md) for peer discovery setup
