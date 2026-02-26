# Deploying Stellar-K8s Operator using OLM

The Operator Lifecycle Manager (OLM) is a component of the Operator Framework that manages the installation, update, and lifecycle of operators running on a Kubernetes cluster.

## Prerequisites
- A Kubernetes cluster with [OLM installed](https://olm.operatorframework.io/docs/getting-started/).
- Operator bundle generated and pushed to a registry.

## Installation Steps

### 1. Create a CatalogSource
If you are testing locally or publishing a custom catalog, create a `CatalogSource` to tell OLM where to find the Operator.

```yaml
apiVersion: operators.coreos.com/v1alpha1
kind: CatalogSource
metadata:
  name: stellar-catalog
  namespace: olm
spec:
  sourceType: grpc
  image: <your-registry>/stellar-operator-catalog:latest
  displayName: Stellar K8s Operators
  publisher: 0xOlivanode
  updateStrategy:
    registryPoll:
      interval: 10m
```

### 2. Create a Namespace and OperatorGroup
Create a namespace for the operator, and an `OperatorGroup` to specify its target scope.

```yaml
apiVersion: v1
kind: Namespace
metadata:
  name: stellar-operator-system
---
apiVersion: operators.coreos.com/v1
kind: OperatorGroup
metadata:
  name: stellar-operator-group
  namespace: stellar-operator-system
spec:
  targetNamespaces:
    - stellar-operator-system
```

### 3. Create a Subscription
Create a `Subscription` to install the operator from the catalog.

```yaml
apiVersion: operators.coreos.com/v1alpha1
kind: Subscription
metadata:
  name: stellar-operator-sub
  namespace: stellar-operator-system
spec:
  channel: alpha
  name: stellar-operator
  source: stellar-catalog
  sourceNamespace: olm
  installPlanApproval: Automatic
```

Apply these files:
```bash
kubectl apply -f catalog-source.yaml
kubectl apply -f operator-group.yaml
kubectl apply -f subscription.yaml
```

Once installed, verify the CSV is `Succeeded`:
```bash
kubectl get csv -n stellar-operator-system
```
