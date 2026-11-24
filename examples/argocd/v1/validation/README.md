# v1 validation record

This is the review checklist for a clean ArgoCD sync. It deliberately uses the
ArgoCD UI or ArgoCD CLI as the deployment interface; the guide does not require
imperative Kubernetes resource creation.

## Static validation

The repository includes a dependency-free consistency check:

```text
python3 examples/argocd/v1/validation/check.py
```

From the repository root, also render the charts:

```text
helm lint charts/stellar-operator
helm template stellar-testnet-validator examples/argocd/v1/node-chart \
  --values examples/argocd/v1/node-chart/values-validator.yaml
helm template stellar-soroban-rpc examples/argocd/v1/node-chart \
  --values examples/argocd/v1/node-chart/values-soroban-rpc.yaml
```

Review both rendered outputs against `config/crd/stellarnode-crd.yaml` and confirm
that the Application manifests contain the expected repo URL, versioned paths,
sync waves, `CreateNamespace=true`, and foreground pruning options.

## Recorded-session template

Record the following ArgoCD UI/CLI results in the PR description or attach a
terminal transcript. Do not paste secret values.

```text
Application: stellar-k8s-gitops-v1
  sync: Synced
  health: Healthy

Application: stellar-operator
  sync: Synced
  health: Healthy
  resources: CRDs, Deployment, ServiceAccount, RBAC

Application: stellar-testnet-validator
  sync: Synced
  health: Progressing (until the Testnet node catches up)
  resources: Secret, StellarNode, operator-managed workload, PVC

Application: stellar-soroban-rpc
  sync: Synced
  health: Progressing (until captive core catches up)
  resources: StellarNode, operator-managed workload, PVC, HPA
```

A fresh cluster can reach `Synced` without hand-created namespaces, CRDs, PVCs,
Deployments, or Services. The development validator still requires a real seed
before it can become operationally `Ready`; replace the placeholder through an
approved encrypted-secret workflow before running it as a validator.
