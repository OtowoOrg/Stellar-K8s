# examples/gitops/argocd

ArgoCD Application manifests for the Stellar-K8s GitOps pipeline.

## Files

| File | Purpose |
|---|---|
| `app-of-apps.yaml` | Root Application — bootstrap entry point. Apply this once to kick off the entire pipeline. |
| `testnet-app.yaml` | Testnet environment Application. Automated sync enabled. |
| `futurenet-app.yaml` | Futurenet environment Application. Automated sync enabled. |
| `mainnet-app.yaml` | Mainnet (production) Application. **Manual sync only.** |

## Bootstrap

```bash
# Register the repository with ArgoCD (once per cluster)
argocd repo add https://github.com/agnesnaomiolim-cloud/Stellar-K8s \
  --name stellar-k8s

# Apply the root Application
kubectl apply -f examples/gitops/argocd/app-of-apps.yaml

# Monitor convergence
argocd app wait stellar-k8s-gitops --health --sync --timeout 180
```

## Promotion to mainnet

```bash
# After PR is merged and testnet smoke test passes:
argocd app sync stellar-mainnet --prune
argocd app wait stellar-mainnet --health --sync --timeout 300
```

## Full documentation

See [docs/operations/gitops-argocd.md](../../../docs/operations/gitops-argocd.md).
