# Flux GitOps examples

This folder contains declarative Flux CD manifests for deploying Stellar-K8s.

- `operator/` — `HelmRepository` and `HelmRelease` that install the `stellar-operator` Helm chart.
- `validator/` — `HelmRelease` for a Testnet Validator node.
- `soroban-rpc/` — `HelmRelease` for Soroban RPC nodes.
- `kustomization.yaml` — root `Kustomization` that applies all resources in dependency order.

## Usage

1. Install Flux on your cluster (`flux bootstrap` or equivalent).
2. Add this repository as a Flux `GitRepository` source.
3. Apply `kustomization.yaml` (or commit it to your GitOps repository and let Flux reconcile it).

## Sync and validation

- `make flux-validate` renders these manifests with `kubectl kustomize` and
  `flux build kustomization examples/flux`. It needs no cluster and runs in
  the `Flux Sync` CI workflow on every pull request.
- `make flux-sync KUSTOMIZATION=<name> [NAMESPACE=<namespace>]` requests a
  reconciliation from the terminal against a bootstrapped cluster.
- `make flux-sync` without `KUSTOMIZATION` fails fast and prints the usage.

See [docs/gitops/flux.md](../../docs/gitops/flux.md) for the full guide including finalizer handling and multi-cluster patterns.

## Notes

- The `HelmRelease` resources use `dependsOn` to ensure the operator is installed before workload releases are reconciled.
- The Validator values contain an obvious `REPLACE_WITH_A_TESTNET_VALIDATOR_SECRET_KEY` placeholder. Use a `SopsSecret` or an External Secrets Operator `ExternalSecret` for production.
- See [docs/gitops/flux.md](../../docs/gitops/flux.md) for the full guide including finalizer handling and multi-cluster patterns.
