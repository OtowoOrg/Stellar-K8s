# Deploy Stellar-K8s with Flux

This guide is the **golden path** for deploying a Testnet Validator and a
Soroban RPC node with [Flux CD](https://fluxcd.io). It uses only declarative
Kubernetes resources: Flux reconciles the operator Helm chart and the versioned
workload chart; the Stellar-K8s operator then reconciles each `StellarNode`
into its workloads, Services, ConfigMaps, and PVC.

The reference files live in [`examples/flux/`](../../examples/flux/).

> **Security note:** The checked-in Validator values contain an obvious
> `REPLACE_WITH_A_TESTNET_VALIDATOR_SECRET_KEY` placeholder. It is safe for
> manifest validation, but it is not a validator signing key. For production,
> use a [SOPS-encrypted secret](https://fluxcd.io/flux/guides/mozilla-sops/) or
> an External Secrets Operator `ExternalSecret`. Never commit a real seed to Git.

## Prerequisites

- A Kubernetes cluster (1.28+) with Flux installed (`flux bootstrap` or equivalent).
- `kubectl` and `flux` CLI configured against the cluster.
- At least 4 vCPU and 8 GiB of memory for both node profiles.
- Git access from the Flux source-controller to this repository.

No `kubectl apply`, `helm install`, hand-created namespace, or manually created
PVC is required by this guide. Flux's `HelmRelease` `install.createNamespace: true`
creates destination namespaces, and the operator chart installs its CRDs.

## How it works

```
GitRepository (stellar-k8s)
  └── Kustomization (examples/flux/)
        ├── HelmRepository (stellar-k8s Helm registry)
        ├── HelmRelease: stellar-operator        ← installs CRDs + controller
        ├── HelmRelease: stellar-testnet-validator  (dependsOn: stellar-operator)
        └── HelmRelease: stellar-soroban-rpc        (dependsOn: stellar-operator)
```

`dependsOn` ensures the operator `HelmRelease` is `Ready` before the workload
releases are reconciled, so the `StellarNode` CRD is always present when the
workload chart applies it.

## 1. Register the GitRepository source

Apply the `GitRepository` that points Flux at this repository:

```bash
kubectl apply -f examples/flux/git-repository.yaml
```

Or commit it to your Flux bootstrap repository and let Flux reconcile it.

For a private fork, create a deploy key or token secret first:

```bash
flux create secret git stellar-k8s-auth \
  --url=https://github.com/OtowoOrg/Stellar-K8s \
  --username=git \
  --password=<token>
```

Then add `secretRef: { name: stellar-k8s-auth }` to the `GitRepository` spec.

## 2. Apply the root Kustomization

```bash
kubectl apply -f examples/flux/kustomization.yaml
```

Flux will reconcile the `HelmRepository`, then the operator `HelmRelease`, and
finally the workload `HelmRelease` resources in dependency order.

Watch progress:

```bash
flux get helmreleases --all-namespaces
```

Expected steady state:

```
NAMESPACE        NAME                        READY  STATUS
stellar-system   stellar-operator            True   Release reconciliation succeeded
stellar-testnet  stellar-testnet-validator   True   Release reconciliation succeeded
stellar-testnet  stellar-soroban-rpc         True   Release reconciliation succeeded
```

## 3. Follow the Validator sync

The Validator `HelmRelease` renders a `Secret` and a `StellarNode`:

```yaml
spec:
  nodeType: Validator
  network: testnet
  version: "v21.0.0"
  replicas: 1
  storage:
    storageClass: standard
    size: 100Gi
    retentionPolicy: Delete
  validatorConfig:
    seedSecretRef: validator-seed-testnet
    enableHistoryArchive: true
```

Flux considers the release reconciled when the Helm install succeeds. The
operator will report the node as `Progressing` while Stellar Core catches up
with the network. That is expected.

For production, replace the `seedSecret.value` placeholder with a reference to
your approved secret backend. The SOPS pattern keeps the encrypted value in Git:

```yaml
# validator-seed.sops.yaml (encrypted with age/GPG)
apiVersion: v1
kind: Secret
metadata:
  name: validator-seed-testnet
  namespace: stellar-testnet
stringData:
  STELLAR_CORE_SEED: ENC[AES256_GCM,data:...,type:str]
```

Commit the encrypted file and reference it from the `HelmRelease` via
`valuesFrom` or a `SopsSecret`.

## 4. Follow the Soroban RPC sync

The Soroban RPC `HelmRelease` renders a `StellarNode` with two replicas and an
HPA range:

```yaml
spec:
  nodeType: SorobanRpc
  network: testnet
  version: "v21.0.0"
  replicas: 2
  sorobanConfig:
    stellarCoreUrl: http://validator-testnet:11626
    enablePreflight: true
    maxEventsPerRequest: 10000
  autoscaling:
    minReplicas: 2
    maxReplicas: 3
    targetCpuUtilizationPercentage: 70
```

The `stellarCoreUrl` is an in-cluster service endpoint. In a multi-cluster HA
layout, make this endpoint a declaratively managed cross-cluster DNS name and
give each cluster its own `HelmRelease`.

## 5. Automate and verify the sync

The examples in `examples/flux/` are rendered and validated in CI by the
`Flux Sync` workflow, so a broken manifest fails the pull request instead of
the next production sync. The same check runs locally:

```bash
make flux-validate
```

The target renders the kustomization with `kubectl kustomize` and, when the
`flux` CLI is installed, with `flux build kustomization examples/flux`.
Neither command needs a cluster.

To request a reconciliation from the terminal against a bootstrapped cluster,
run:

```bash
make flux-sync KUSTOMIZATION=stellar-k8s
```

The target requires an explicit Kustomization name and uses the currently
configured Flux context. Pass `NAMESPACE=<namespace>` when the
`Kustomization` is not in `flux-system`:

```bash
make flux-sync KUSTOMIZATION=stellar-k8s NAMESPACE=flux-system
```

The equivalent manual commands are:

```bash
flux reconcile kustomization stellar-k8s --namespace flux-system
flux get kustomizations --all-namespaces
```

Prefer `flux reconcile` over `kubectl apply` for anything that Flux already
owns: a manual `apply` is overwritten by the next reconciliation and leaves the
cluster drifting from Git.

### Reconciliation interval and drift

The `GitRepository` polls every minute and the `HelmRelease` resources every
ten minutes. A shorter interval reacts faster to a bad merge but increases
the load on the Git provider. When a release is reconciled out of band, Flux
re-applies the Git state and removes the manual change on the next interval.

## Finalizers, deletion, and PVC lifecycle

Stellar-K8s uses a kube-rs finalizer on each `StellarNode`:

```
stellarnode.stellar.org/finalizer
```

When a `HelmRelease` is deleted or a `StellarNode` is removed from the chart
values, Flux triggers a Helm uninstall. Kubernetes sets
`metadata.deletionTimestamp` on the `StellarNode` but keeps the resource until
the operator completes cleanup:

1. The operator deletes operator-managed external resources in dependency order.
2. The PVC is deleted or retained according to `spec.storage.retentionPolicy`.
3. Only after cleanup succeeds does the operator remove its finalizer.
4. Kubernetes completes deletion; Flux reports the release healthy again.

Do **not** remove the finalizer by hand. That bypasses the operator's cleanup
contract and can leave PVCs, Services, or credentials orphaned.

For data that must survive node removal, set `retentionPolicy: Retain` before
the deletion is merged and record the PVC ownership decision in Git.

## Pinning to a release tag

The `GitRepository` uses `branch: main` so a fresh cluster can follow the
repository head. Before production use, pin to a reviewed tag or commit SHA:

```yaml
spec:
  ref:
    tag: v4.0.0   # or commit: <sha>
```

Keep the `examples/flux/` path stable. Introduce breaking changes under a new
directory such as `examples/flux/v2/`.

## Multi-cluster pattern

Repeat the `HelmRelease` resources per cluster by changing only declarative
inputs:

1. Give each cluster a unique `HelmRelease` name, e.g.
   `stellar-testnet-validator-us-east`.
2. Register each cluster as a Flux target using `spec.kubeConfig` or a separate
   Flux instance bootstrapped on that cluster.
3. Use a cluster-specific values overlay for storage class, node selectors, and
   network endpoints.
4. Keep the same `targetRevision` and chart path in every cluster.
5. Use separate Validator identities; never copy one signing seed into multiple
   active Validators.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| `HelmRelease` is `False` | `flux logs --kind=HelmRelease --name=stellar-operator -n stellar-system` |
| `StellarNode` is rejected | Compare rendered YAML with `config/crd/stellarnode-crd.yaml`; ensure node type-specific config is present. |
| PVC is `Pending` | Set the cluster's real StorageClass in the `HelmRelease` values; do not patch the PVC. |
| Validator pod is not ready | Confirm the secret provider has materialized the seed and wait for Testnet catch-up. |
| Soroban RPC cannot reach Core | Change `soroban.stellarCoreUrl` in Git to the intended Service/DNS endpoint. |
| Resource is stuck terminating | Fix the operator cleanup dependency declaratively and allow the finalizer retry. |

For more details on the kube-rs finalizer design, see
[`docs/adr/0003-kube-rs-finalizers.md`](../adr/0003-kube-rs-finalizers.md).
For the ArgoCD equivalent of this guide, see
[`docs/gitops/argocd.md`](argocd.md) (and its rendered variant
[`docs/gitops/argocd.mdx`](argocd.mdx)).
