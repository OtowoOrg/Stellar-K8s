# Governance and Policy Management for Stellar-K8s

This document describes how to enforce operational and security best practices for Stellar-K8s deployments using Kyverno policies.

## Table of Contents

- [Overview](#overview)
- [Prerequisites](#prerequisites)
- [Installation](#installation)
- [Available Policies](#available-policies)
- [Policy Enforcement Modes](#policy-enforcement-modes)
- [Customization](#customization)
- [Troubleshooting](#troubleshooting)

## Overview

Kyverno is a Kubernetes-native policy engine that allows you to:

- **Validate** resource configurations before they're created
- **Mutate** resources to apply defaults or enforce standards
- **Generate** resources automatically based on policies
- **Verify** image signatures and compliance

For Stellar-K8s, we provide a suite of Kyverno policies that enforce:

- **Security Best Practices**: Non-root containers, security contexts, resource limits
- **Operational Standards**: Required labels, explicit versions, storage configuration
- **High Availability**: Pod disruption budgets, replica counts, resource requests
- **Data Protection**: Persistent storage for validators, retention policies

## Prerequisites

- Kubernetes 1.24+
- Kyverno 1.9+ installed in your cluster
- `kubectl` configured with cluster admin access

## Installation

### Step 1: Install Kyverno

If you don't have Kyverno installed, install it using Helm:

```bash
# Add the Kyverno Helm repository
helm repo add kyverno https://kyverno.github.io/kyverno/
helm repo update

# Install Kyverno in the kyverno namespace
helm install kyverno kyverno/kyverno --namespace kyverno --create-namespace

# Verify installation
kubectl get pods -n kyverno
```

### Step 2: Install Stellar-K8s Policies

Apply the Kyverno policies for Stellar-K8s:

```bash
# Apply all policies
kubectl apply -f policy/kyverno-policies.yaml

# Verify policies are installed
kubectl get clusterpolicies | grep stellar
```

### Step 3: Verify Policy Installation

```bash
# List all installed policies
kubectl get clusterpolicies

# Check policy details
kubectl describe clusterpolicy disallow-latest-tag

# View policy violations (if in audit mode)
kubectl get policyreport -A
```

## Available Policies

### 1. Disallow Latest Tag (`disallow-latest-tag`)

**Purpose**: Prevent using the `latest` tag for container images, ensuring deterministic deployments.

**Validation Rule**:
- StellarNode `spec.version` must not be `latest`

**Example - Valid**:
```yaml
spec:
  version: "v21.0.0"
```

**Example - Invalid**:
```yaml
spec:
  version: "latest"  # ❌ Rejected
```

### 2. Require Stellar Labels (`require-stellar-labels`)

**Purpose**: Enforce organizational labels for cost tracking, ownership, and environment management.

**Required Labels**:
- `cost-center`: Cost allocation identifier
- `owner`: Team or person responsible
- `environment`: Deployment environment (dev, staging, prod)

**Example - Valid**:
```yaml
metadata:
  labels:
    cost-center: "engineering"
    owner: "stellar-team"
    environment: "production"
```

**Example - Invalid**:
```yaml
metadata:
  labels:
    cost-center: "engineering"
    # ❌ Missing 'owner' and 'environment'
```

### 3. Require Storage for Validators (`require-storage-for-validators`)

**Purpose**: Ensure Validator nodes use persistent storage, not ephemeral emptyDir.

**Validation Rule**:
- Validator nodes must have `spec.storage.storageClass` defined

**Example - Valid**:
```yaml
spec:
  nodeType: Validator
  storage:
    storageClass: "fast-ssd"
    size: "100Gi"
```

**Example - Invalid**:
```yaml
spec:
  nodeType: Validator
  storage:
    # ❌ No storageClass specified
```

### 4. Require Resource Limits (`require-resource-limits`)

**Purpose**: Ensure all nodes specify CPU and memory limits for predictable resource management.

**Validation Rule**:
- `spec.resources.limits.cpu` must be defined
- `spec.resources.limits.memory` must be defined

**Example - Valid**:
```yaml
spec:
  resources:
    limits:
      cpu: "4"
      memory: "8Gi"
```

### 5. Require Resource Requests (`require-resource-requests`)

**Purpose**: Ensure all nodes specify CPU and memory requests for proper Kubernetes scheduling.

**Validation Rule**:
- `spec.resources.requests.cpu` must be defined
- `spec.resources.requests.memory` must be defined

**Example - Valid**:
```yaml
spec:
  resources:
    requests:
      cpu: "2"
      memory: "4Gi"
```

### 6. Require Network Specification (`require-network-specification`)

**Purpose**: Ensure the target Stellar network is explicitly specified.

**Validation Rule**:
- `spec.network` must be one of: `Mainnet`, `Testnet`, `Futurenet`, or `Custom`

**Example - Valid**:
```yaml
spec:
  network: Testnet
```

### 7. Require Node Type (`require-node-type`)

**Purpose**: Ensure a valid node type is specified.

**Validation Rule**:
- `spec.nodeType` must be one of: `Validator`, `Horizon`, `SorobanRpc`

**Example - Valid**:
```yaml
spec:
  nodeType: Validator
```

### 8. Require Version Specification (`require-version-specification`)

**Purpose**: Ensure an explicit version is specified (not `latest`).

**Validation Rule**:
- `spec.version` must be defined and not empty

**Example - Valid**:
```yaml
spec:
  version: "v21.0.0"
```

### 9. Require Replica Count (`require-replica-count`)

**Purpose**: Encourage high availability by requiring explicit replica count specification.

**Validation Rule**:
- `spec.replicas` should be defined

**Example - Valid**:
```yaml
spec:
  replicas: 3
```

### 10. Disallow Privileged Containers (`disallow-privileged-containers`)

**Purpose**: Prevent privileged containers for security reasons.

**Validation Rule**:
- Pod `spec.containers[].securityContext.privileged` must be `false`

### 11. Require Security Context (`require-security-context`)

**Purpose**: Ensure pods run as non-root for security.

**Validation Rule**:
- Pod `spec.securityContext.runAsNonRoot` must be `true`

### 12. Require Pod Disruption Budget (`require-pod-disruption-budget`)

**Purpose**: Ensure high availability by requiring PDB configuration.

**Validation Rule**:
- `spec.minAvailable` or `spec.maxUnavailable` must be defined

**Example - Valid**:
```yaml
spec:
  minAvailable: 1
```

### 13. Require Storage Retention Policy (`require-storage-retention-policy`)

**Purpose**: Ensure explicit data retention policy for persistent volumes.

**Validation Rule**:
- `spec.storage.retentionPolicy` must be one of: `Retain`, `Delete`, `Recycle`

**Example - Valid**:
```yaml
spec:
  storage:
    retentionPolicy: Retain
```

## Policy Enforcement Modes

Policies can operate in two modes:

### Audit Mode (Default)

Violations are logged but resources are still created. Use this for initial rollout:

```yaml
spec:
  validationFailureAction: audit
```

**View violations**:
```bash
kubectl get policyreport -A
kubectl describe policyreport <name> -n <namespace>
```

### Enforce Mode

Violations are rejected and resources are not created. Use this after validation:

```yaml
spec:
  validationFailureAction: enforce
```

## Customization

### Modify Enforcement Mode

To switch a policy from audit to enforce:

```bash
# Edit the policy
kubectl edit clusterpolicy disallow-latest-tag

# Change validationFailureAction from 'audit' to 'enforce'
```

### Create Custom Policies

Create a new policy file `policy/custom-policies.yaml`:

```yaml
apiVersion: kyverno.io/v1
kind: ClusterPolicy
metadata:
  name: my-custom-policy
spec:
  validationFailureAction: audit
  rules:
    - name: my-rule
      match:
        resources:
          kinds:
            - StellarNode
      validate:
        message: "Custom validation message"
        pattern:
          spec:
            # Your validation pattern here
```

Apply it:
```bash
kubectl apply -f policy/custom-policies.yaml
```

### Exclude Namespaces

To exclude certain namespaces from a policy:

```yaml
spec:
  validationFailureAction: audit
  rules:
    - name: check-image-tag
      match:
        resources:
          kinds:
            - StellarNode
        excludeResources:
          namespaces:
            - kube-system
            - kyverno
      validate:
        # ...
```

## Troubleshooting

### Policy Not Triggering

1. **Verify policy is installed**:
   ```bash
   kubectl get clusterpolicies <policy-name>
   ```

2. **Check policy status**:
   ```bash
   kubectl describe clusterpolicy <policy-name>
   ```

3. **View policy violations**:
   ```bash
   kubectl get policyreport -A
   ```

### Resource Rejected by Policy

If a resource is rejected:

1. **Check the error message**:
   ```bash
   kubectl apply -f my-node.yaml
   # Error: policy <name> validation error: <message>
   ```

2. **Review the policy**:
   ```bash
   kubectl describe clusterpolicy <policy-name>
   ```

3. **Fix the resource** to comply with the policy

4. **Temporarily disable policy** (for testing):
   ```bash
   kubectl patch clusterpolicy <policy-name> -p '{"spec":{"validationFailureAction":"audit"}}'
   ```

### Performance Issues

If policies are causing performance issues:

1. **Check Kyverno logs**:
   ```bash
   kubectl logs -n kyverno -l app=kyverno -f
   ```

2. **Disable unnecessary policies**:
   ```bash
   kubectl delete clusterpolicy <policy-name>
   ```

3. **Optimize policy rules** to be more specific

## Best Practices

1. **Start in Audit Mode**: Deploy policies in audit mode first to understand impact
2. **Review Violations**: Check `policyreport` resources regularly
3. **Gradual Enforcement**: Move to enforce mode after addressing violations
4. **Document Exceptions**: If you need to exclude resources, document why
5. **Version Control**: Keep policies in version control alongside your infrastructure code
6. **Test Changes**: Test policy changes in a staging environment first

## Integration with CI/CD

### Pre-deployment Validation

Use Kyverno CLI to validate resources before deployment:

```bash
# Install Kyverno CLI
curl -L https://github.com/kyverno/kyverno/releases/latest/download/kyverno-cli_linux_x86_64.tar.gz | tar xz

# Validate a resource
./kyverno apply policy/kyverno-policies.yaml -r my-node.yaml
```

### GitHub Actions Example

```yaml
name: Validate StellarNode Policies

on: [pull_request]

jobs:
  validate:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      
      - name: Install Kyverno CLI
        run: |
          curl -L https://github.com/kyverno/kyverno/releases/latest/download/kyverno-cli_linux_x86_64.tar.gz | tar xz
      
      - name: Validate policies
        run: |
          ./kyverno apply policy/kyverno-policies.yaml -r config/samples/*.yaml
```

## References

- [Kyverno Documentation](https://kyverno.io/docs/)
- [Kyverno Policy Library](https://kyverno.io/policies/)
- [Kubernetes Security Best Practices](https://kubernetes.io/docs/concepts/security/)
