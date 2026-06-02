# Installation Guide

This guide walks you through installing Stellar-K8s on your Kubernetes cluster.

## Quick Installation

### Step 1: Clone the Repository

```bash
git clone https://github.com/OtowoOrg/Stellar-K8s.git
cd Stellar-K8s
```

### Step 2: Install with Helm

```bash
# Add Stellar-K8s Helm repository (if published)
helm repo add stellar-k8s https://otowo.org/helm-charts
helm repo update

# Install the chart
helm install stellar-k8s stellar-k8s/stellar-k8s \
  --namespace stellar-system \
  --create-namespace
```

Alternatively, install from the local chart:

```bash
helm install stellar-k8s ./charts/stellar-k8s \
  --namespace stellar-system \
  --create-namespace
```

### Step 3: Verify Installation

Check that all components are running:

```bash
# Check operator deployment
kubectl get deployments -n stellar-system

# Check CRDs
kubectl get crds | grep stellar

# Check operator logs
kubectl logs -n stellar-system -l app=stellar-operator
```

Expected output:

```
NAME                  READY   UP-TO-DATE   AVAILABLE   AGE
stellar-operator      1/1     1            1           30s
```

## Installation Options

### Custom Values

Create a `values.yaml` file to customize the installation:

```yaml
# values.yaml
operator:
  replicas: 1
  resources:
    requests:
      cpu: 100m
      memory: 128Mi
    limits:
      cpu: 500m
      memory: 512Mi

monitoring:
  enabled: true
  prometheus:
    serviceMonitor: true

rbac:
  create: true

serviceAccount:
  create: true
  name: stellar-operator
```

Install with custom values:

```bash
helm install stellar-k8s stellar-k8s/stellar-k8s \
  --namespace stellar-system \
  --create-namespace \
  -f values.yaml
```

### Configuration Parameters

| Parameter | Description | Default |
|-----------|-------------|---------|
| `operator.replicas` | Number of operator replicas | `1` |
| `operator.image.repository` | Operator image repository | `stellar-k8s/operator` |
| `operator.image.tag` | Operator image tag | `latest` |
| `monitoring.enabled` | Enable Prometheus monitoring | `true` |
| `rbac.create` | Create RBAC resources | `true` |
| `serviceAccount.create` | Create ServiceAccount | `true` |

View all configuration options:

```bash
helm show values stellar-k8s/stellar-k8s
```

## Installation Methods

### Method 1: Helm (Recommended)

Helm provides the easiest installation and upgrade path:

```bash
helm install stellar-k8s stellar-k8s/stellar-k8s \
  --namespace stellar-system \
  --create-namespace \
  --set operator.replicas=2 \
  --set monitoring.enabled=true
```

### Method 2: kubectl with Manifests

Apply Kubernetes manifests directly:

```bash
# Install CRDs
kubectl apply -f deploy/crds/

# Create namespace
kubectl create namespace stellar-system

# Install operator
kubectl apply -f deploy/operator/ -n stellar-system

# Install RBAC
kubectl apply -f deploy/rbac/ -n stellar-system
```

### Method 3: Kustomize

Use Kustomize for declarative configuration:

```bash
# Create kustomization.yaml
cat <<EOF > kustomization.yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization

namespace: stellar-system

resources:
  - deploy/crds/
  - deploy/operator/
  - deploy/rbac/

patches:
  - patch: |-
      - op: replace
        path: /spec/replicas
        value: 2
    target:
      kind: Deployment
      name: stellar-operator
EOF

# Apply with kubectl
kubectl apply -k .
```

## Post-Installation Steps

### 1. Verify CRDs

Check that Custom Resource Definitions are installed:

```bash
kubectl get crds | grep stellar
```

Expected CRDs:

- `stellarvalidators.stellar.k8s.io`
- `horizonservers.stellar.k8s.io`
- `sorobanrpcs.stellar.k8s.io`

### 2. Check Operator Status

Ensure the operator is running and ready:

```bash
kubectl get pods -n stellar-system
kubectl logs -n stellar-system deployment/stellar-operator
```

### 3. Configure Monitoring (Optional)

If Prometheus is installed, verify ServiceMonitors are created:

```bash
kubectl get servicemonitors -n stellar-system
```

### 4. Test Operator Functionality

Create a test validator resource:

```yaml
# test-validator.yaml
apiVersion: stellar.k8s.io/v1alpha1
kind: StellarValidator
metadata:
  name: test-validator
  namespace: stellar-system
spec:
  network: testnet
  replicas: 1
  storage:
    size: 100Gi
    storageClassName: standard
```

Apply and verify:

```bash
kubectl apply -f test-validator.yaml
kubectl get stellarvalidators -n stellar-system
kubectl get pods -n stellar-system -l app=test-validator
```

Clean up the test:

```bash
kubectl delete -f test-validator.yaml
```

## Upgrading Stellar-K8s

### Helm Upgrade

To upgrade to a new version:

```bash
# Update Helm repository
helm repo update

# Upgrade the release
helm upgrade stellar-k8s stellar-k8s/stellar-k8s \
  --namespace stellar-system \
  -f values.yaml
```

### Check Upgrade Status

```bash
helm status stellar-k8s -n stellar-system
helm history stellar-k8s -n stellar-system
```

### Rollback if Needed

```bash
helm rollback stellar-k8s -n stellar-system
```

## Uninstallation

### Remove Helm Release

```bash
helm uninstall stellar-k8s -n stellar-system
```

### Remove CRDs

!!! danger "Data Loss Warning"
    Removing CRDs will delete all custom resources. Ensure you've backed up any important data.

```bash
kubectl delete crds -l app=stellar-k8s
```

### Clean Up Namespace

```bash
kubectl delete namespace stellar-system
```

## Troubleshooting Installation

### Operator Not Starting

Check operator logs:

```bash
kubectl logs -n stellar-system deployment/stellar-operator
```

Common issues:

- **Image pull errors**: Verify image repository and credentials
- **RBAC permission errors**: Ensure ServiceAccount has required permissions
- **CRD conflicts**: Check for existing CRDs with the same names

### CRDs Not Created

Verify CRD manifests are valid:

```bash
kubectl apply --dry-run=client -f deploy/crds/
```

### Helm Installation Failures

Check Helm release status:

```bash
helm status stellar-k8s -n stellar-system
helm get manifest stellar-k8s -n stellar-system
```

Enable Helm debug output:

```bash
helm install stellar-k8s stellar-k8s/stellar-k8s \
  --namespace stellar-system \
  --create-namespace \
  --debug
```

## Next Steps

Now that Stellar-K8s is installed, you can:

- Follow the [Quick Start Guide](quick-start.md) to deploy your first node
- Explore [Deployment Guides](../deployment-guides/index.md) for specific node types
- Review [Configuration Reference](../configuration/index.md) for advanced options

!!! success "Installation Complete!"
    Stellar-K8s is now installed and ready to manage your Stellar nodes on Kubernetes.
