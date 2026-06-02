# Prerequisites

Before deploying Stellar-K8s, ensure your environment meets the following requirements.

## System Requirements

### Kubernetes Cluster

| Component | Minimum Version | Recommended Version |
|-----------|----------------|---------------------|
| Kubernetes | 1.24+ | 1.28+ |
| kubectl | 1.24+ | 1.28+ |
| Helm | 3.8+ | 3.12+ |

!!! warning "Version Compatibility"
    Stellar-K8s requires Kubernetes 1.24 or later for full CRD support. Earlier versions may have limited functionality.

### Resource Requirements

#### Validator Node (Per Instance)

- **CPU**: 4 cores minimum, 8 cores recommended
- **Memory**: 8 GB minimum, 16 GB recommended
- **Storage**: 500 GB minimum (SSD strongly recommended)
- **Network**: 1 Gbps network connection

#### Horizon API Server

- **CPU**: 2 cores minimum, 4 cores recommended
- **Memory**: 4 GB minimum, 8 GB recommended
- **Storage**: 100 GB minimum (database storage)
- **Network**: 1 Gbps network connection

#### Soroban RPC Node

- **CPU**: 2 cores minimum, 4 cores recommended
- **Memory**: 4 GB minimum, 8 GB recommended
- **Storage**: 200 GB minimum
- **Network**: 1 Gbps network connection

## Required Tools

### kubectl

The Kubernetes command-line tool is required for managing cluster resources.

=== "Linux"

    ```bash
    curl -LO "https://dl.k8s.io/release/$(curl -L -s https://dl.k8s.io/release/stable.txt)/bin/linux/amd64/kubectl"
    chmod +x kubectl
    sudo mv kubectl /usr/local/bin/
    ```

=== "macOS"

    ```bash
    brew install kubectl
    ```

=== "Windows"

    ```powershell
    choco install kubernetes-cli
    ```

### Helm

Helm is used to deploy Stellar-K8s charts.

=== "Linux"

    ```bash
    curl https://raw.githubusercontent.com/helm/helm/main/scripts/get-helm-3 | bash
    ```

=== "macOS"

    ```bash
    brew install helm
    ```

=== "Windows"

    ```powershell
    choco install kubernetes-helm
    ```

### Verify Installations

After installing the tools, verify they work correctly:

```bash
# Check kubectl version
kubectl version --client

# Check Helm version
helm version

# Verify cluster access
kubectl cluster-info
```

## Cluster Capabilities

### Storage Classes

Ensure your cluster has a StorageClass configured for dynamic volume provisioning:

```bash
kubectl get storageclasses
```

You should see at least one StorageClass available. If not, you'll need to configure one before deploying Stellar nodes.

!!! tip "Recommended Storage"
    SSD-backed storage (like AWS gp3, GCE pd-ssd, or Azure Premium SSD) is strongly recommended for validator nodes to ensure optimal performance.

### Ingress Controller

If you plan to expose Horizon API or RPC endpoints externally, ensure an Ingress controller is installed:

```bash
kubectl get pods -n ingress-nginx
```

Popular options include:

- [NGINX Ingress Controller](https://kubernetes.github.io/ingress-nginx/)
- [Traefik](https://doc.traefik.io/traefik/providers/kubernetes-ingress/)
- Cloud provider ingress (AWS ALB, GKE Ingress, etc.)

### Monitoring Stack (Optional)

For full observability, install Prometheus and Grafana:

```bash
# Add Prometheus Helm repository
helm repo add prometheus-community https://prometheus-community.github.io/helm-charts
helm repo update

# Install kube-prometheus-stack
helm install prometheus prometheus-community/kube-prometheus-stack \
  --namespace monitoring --create-namespace
```

## Network Requirements

### Firewall Rules

Stellar nodes require specific network ports to be accessible:

| Service | Port | Protocol | Purpose |
|---------|------|----------|---------|
| Stellar Core | 11625 | TCP | Peer-to-peer communication |
| Stellar Core | 11626 | TCP | Commands and metrics |
| Horizon API | 8000 | HTTP | REST API endpoints |
| Soroban RPC | 8000 | HTTP | RPC endpoints |

### External Connectivity

Validator nodes must be able to connect to:

- Public Stellar testnet or mainnet peers
- History archive servers (HTTP/HTTPS)
- NTP servers for time synchronization

## Permissions

### Kubernetes RBAC

You'll need cluster-admin or sufficient RBAC permissions to:

- Create CustomResourceDefinitions (CRDs)
- Create and manage Deployments, StatefulSets, Services
- Create PersistentVolumeClaims
- Create ServiceAccounts, Roles, and RoleBindings

Verify your permissions:

```bash
kubectl auth can-i create customresourcedefinitions
kubectl auth can-i create statefulsets
kubectl auth can-i create persistentvolumeclaims
```

All commands should return `yes`.

## Next Steps

Once all prerequisites are met, proceed to the [Installation Guide](installation.md) to deploy Stellar-K8s.

!!! success "Prerequisites Checklist"
    - [x] Kubernetes 1.24+ cluster running
    - [x] kubectl and Helm installed
    - [x] StorageClass configured
    - [x] Sufficient cluster resources available
    - [x] Network ports accessible
    - [x] RBAC permissions verified
