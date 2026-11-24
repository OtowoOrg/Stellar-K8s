# Stellar-K8s ArgoCD workload chart v1

This chart is intentionally small: ArgoCD renders it into one `StellarNode` and,
for the development Validator profile, one Kubernetes Secret. The operator owns
the StatefulSet/Deployment, Service, ConfigMap, and PVC generated from the CR.

The chart is not a replacement for the operator chart. Install the operator first
through `examples/argocd/v1/apps/stellar-operator.yaml`.

## Profiles

- `values-validator.yaml`: one Testnet Validator with a 100Gi PVC.
- `values-soroban-rpc.yaml`: two Soroban RPC replicas with an HPA range of 2–3.

The validator seed in the example is a deliberate placeholder. For production,
replace it with an encrypted secret workflow (for example, the typed
`seedSecretSource.externalRef`, `csiRef`, or `vaultRef` fields supported by the
StellarNode CRD) rather than committing secret material to Git.
