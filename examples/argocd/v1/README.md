# ArgoCD golden path v1

This directory is the immutable-shape reference for a declarative Stellar-K8s
installation:

1. `bootstrap.yaml` is the single Application submitted to ArgoCD.
2. `apps/platform-storage.yaml` provisions a local `stellar-local` StorageClass and two static 100Gi PVs for a fresh Kind/Minikube cluster.
3. `apps/stellar-operator.yaml` installs the operator Helm chart and CRDs.
4. `apps/testnet-validator.yaml` renders the Testnet Validator workload chart.
5. `apps/soroban-rpc.yaml` renders the Soroban RPC workload chart.
6. `node-chart/` contains the v1 Helm templates and pinned profile values.

All Applications enable automated prune/self-heal, server-side apply, foreground
pruning, and `PruneLast`. The operator Application is sync wave `-20`; workload
Applications are sync wave `0`; the generated StellarNode is sync wave `10`.

## Repository pinning

The examples use `targetRevision: main` so a fresh local Kind/Minikube cluster can
follow the repository head. Before production use, change every Application to a
reviewed tag or commit SHA. Keep the `v1` path stable while introducing a new
breaking guide/chart contract under `v2`.

## Secret safety

`values-validator.yaml` contains an obvious development placeholder rather than
secret material. Use the CRD's `seedSecretSource` integration with your approved
secret backend for production. The placeholder makes the Application and CR
synchronizable on a fresh cluster without pretending that a signing key belongs in
source control.
