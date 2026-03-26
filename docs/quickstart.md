# Quickstart: Running the Stellar-K8s Operator on Kind

This guide walks you from `git clone` to a running operator with a sample `StellarNode` on a local [Kind](https://kind.sigs.k8s.io/) cluster.

## Prerequisites

| Tool | Version | Install |
|------|---------|---------|
| Rust | stable (≥ 1.75) | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| Docker | ≥ 24 | [docs.docker.com](https://docs.docker.com/get-docker/) |
| Kind | ≥ 0.22 | `go install sigs.k8s.io/kind@latest` or [kind.sigs.k8s.io](https://kind.sigs.k8s.io/docs/user/quick-start/#installation) |
| kubectl | ≥ 1.28 | [kubernetes.io/docs](https://kubernetes.io/docs/tasks/tools/) |
| Helm | ≥ 3.14 | `curl https://raw.githubusercontent.com/helm/helm/main/scripts/get-helm-3 \| bash` |

Verify everything is installed:

```bash
rustc --version
docker version
kind version
kubectl version --client
helm version
```

## Step 1 — Clone the Repository

```bash
git clone https://github.com/OtowoOrg/Stellar-K8s.git
cd Stellar-K8s
```

## Step 2 — Build the Operator

```bash
cargo build --release --locked
```

The binary is placed at `target/release/stellar-operator`. You can verify it works:

```bash
./target/release/stellar-operator version
```

## Step 3 — Create a Kind Cluster

```bash
kind create cluster --name stellar-dev --wait 120s
```

Confirm the cluster is running:

```bash
kubectl cluster-info --context kind-stellar-dev
kubectl get nodes
```

## Step 4 — Install the CRD

```bash
kubectl apply -f config/crd/stellarnode-crd.yaml
```

Verify the CRD is registered:

```bash
kubectl get crd stellarnodes.stellar.org
```

## Step 5 — Build and Load the Operator Image

```bash
docker build -t stellar-operator:dev .
kind load docker-image stellar-operator:dev --name stellar-dev
```

## Step 6 — Deploy the Operator

Create the namespace and apply the Helm chart with dev values:

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

Apply the example `StellarNode` resource:

```bash
kubectl apply -f config/samples/test-stellarnode.yaml
```

Watch the operator reconcile it:

```bash
kubectl get stellarnode -n stellar-system -w
```

Check the resources the operator created:

```bash
kubectl get deploy,sts,svc,pvc,cm -n stellar-system -l app.kubernetes.io/managed-by=stellar-operator
```

## Step 8 — Verify Health

```bash
# Port-forward the operator REST API
kubectl port-forward -n stellar-system svc/stellar-operator 8080:8080 &

# Check health endpoint
curl http://localhost:8080/health

# Check leader status
curl http://localhost:8080/leader

# View Prometheus metrics (if metrics feature is enabled)
curl http://localhost:8080/metrics
```

## Cleanup

```bash
kind delete cluster --name stellar-dev
```

## Automated Quickstart

You can run all the steps above with a single command:

```bash
make quickstart
```

This target automates cluster creation, CRD installation, image build/load, operator deployment, and sample `StellarNode` creation.

## Troubleshooting

**Operator pod is in `CrashLoopBackOff`**
```bash
kubectl logs -n stellar-system -l app.kubernetes.io/name=stellar-operator --previous
```

**CRD not found error**
```bash
kubectl apply -f config/crd/stellarnode-crd.yaml
```

**Image pull errors with Kind**

Make sure you loaded the image into the cluster:
```bash
kind load docker-image stellar-operator:dev --name stellar-dev
```

**`kind` not found**

Install Kind following the [official guide](https://kind.sigs.k8s.io/docs/user/quick-start/#installation).

## Next Steps

- Read [DEVELOPMENT.md](../DEVELOPMENT.md) for contributor workflows
- Explore [examples/](../examples/) for advanced `StellarNode` configurations
- See [docs/health-checks.md](health-checks.md) for health check configuration
- See [docs/peer-discovery.md](peer-discovery.md) for peer discovery setup
