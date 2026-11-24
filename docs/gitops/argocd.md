# Deploy Stellar-K8s with ArgoCD

This guide is the **v1 golden path** for deploying a Testnet Validator and a
Soroban RPC node with ArgoCD. It uses only declarative Kubernetes resources:
ArgoCD renders the operator Helm chart and the versioned workload chart; the
Stellar-K8s operator then reconciles each `StellarNode` into its workloads,
Services, ConfigMaps, and PVC.

The reference files live in [`examples/argocd/v1`](../../examples/argocd/v1/).
The v1 directory is a versioned contract. Keep it stable for existing users and
introduce breaking changes under a new directory such as `v2`.

> **Security note:** The checked-in Validator values contain an obvious
> `REPLACE_WITH_A_TESTNET_VALIDATOR_SECRET_KEY` placeholder. It is safe for
> manifest validation, but it is not a validator signing key. For production,
> use `validatorConfig.seedSecretSource` with your approved External Secrets,
> Secrets Store CSI, or Vault integration. Never commit a real seed to Git.

## Prerequisites

- A fresh Minikube or Kind cluster. The golden path includes a declarative local
  `stellar-local` StorageClass and static PVs for the two profiles; cloud
  clusters should override these with their approved dynamic StorageClass.
- ArgoCD installed in the `argocd` namespace.
- The `argocd` CLI installed and authenticated if syncing from the terminal.
- Git access from the ArgoCD repo-server to this repository.
- At least 4 vCPU and 8 GiB of memory for both node profiles.
- A reviewed Git revision. The examples use `main` to remain runnable from a
  fresh checkout; production Applications should use a release tag or commit SHA.

No `kubectl apply`, `helm install`, hand-created namespace, or manually created
PVC is required by this guide. ArgoCD's `CreateNamespace=true` option creates
its destination namespaces, and the operator chart installs its CRDs.

## 1. Install ArgoCD declaratively

Install ArgoCD using the installation method approved by your platform team. If
ArgoCD is already present, skip this step. This guide starts at the ArgoCD
Application API and does not depend on how ArgoCD itself was bootstrapped.

The ArgoCD instance must be able to read this repository. For a public repository
no repository Secret is needed. For a private fork, register the repository in
ArgoCD using your organization's declarative ArgoCD repository configuration.

## 2. Bootstrap the versioned golden path

Submit [`examples/argocd/v1/bootstrap.yaml`](../../examples/argocd/v1/bootstrap.yaml)
to ArgoCD using the ArgoCD UI's **+ New App > YAML** editor or the equivalent
ArgoCD API/GitOps bootstrap layer. The manifest creates an app-of-apps:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: stellar-k8s-gitops-v1
  namespace: argocd
spec:
  source:
    path: examples/argocd/v1/apps
    targetRevision: main
  destination:
    server: https://kubernetes.default.svc
    namespace: argocd
```

The child Applications are:

| Application | Source | Destination | Purpose |
| --- | --- | --- | --- |
| `stellar-operator` | `charts/stellar-operator` | `stellar-system` | CRDs, controller, RBAC |
| `stellar-testnet-validator` | `examples/argocd/v1/node-chart` + `values-validator.yaml` | `stellar-testnet` | One Testnet Validator |
| `stellar-soroban-rpc` | `examples/argocd/v1/node-chart` + `values-soroban-rpc.yaml` | `stellar-testnet` | Two Soroban RPC replicas |

The operator Application uses sync wave `-20`. The workload Applications use
wave `0`, and the generated `StellarNode` uses wave `10`. This ensures the CRD
and controller are available before the custom resources are reconciled.

Each Application enables:

- `automated.prune` and `automated.selfHeal` for Git as the desired state
- `CreateNamespace=true` for fresh-cluster destinations
- `ServerSideApply=true` for large CRDs and managed fields
- `PruneLast=true` and `PrunePropagationPolicy=foreground` for dependency-aware deletion
- the ArgoCD resources finalizer so deleting an Application deletes its managed
  resources instead of orphaning them

## 3. Follow the Validator sync

The Validator profile renders a `Secret` and a `StellarNode`:

```yaml
spec:
  nodeType: Validator
  network: testnet
  version: "v21.0.0"
  replicas: 1
  storage:
    storageClass: stellar-local
    size: 100Gi
    retentionPolicy: Delete
  validatorConfig:
    seedSecretRef: validator-seed-testnet
    enableHistoryArchive: true
    historyArchiveUrls:
      - https://history.stellar.org/prd/core-testnet/core_testnet_001
```

ArgoCD considers the resource synced when the desired object is accepted by the
API server. The operator will report the node as progressing while Stellar Core
catches up. That is expected. Do not scale a Validator above one replica and do
not place a production signing seed in the checked-in placeholder Secret.

For production, replace the legacy `seedSecretRef` profile with the CRD's typed
secret source:

```yaml
validatorConfig:
  seedSecretSource:
    externalRef:
      name: validator-prod-seed
      secretStoreRef:
        name: production-secrets
        kind: ClusterSecretStore
      remoteKey: prod/stellar/testnet-validator
      remoteProperty: seed
```

## 4. Follow the Soroban RPC sync

The Soroban profile renders a `StellarNode` with two replicas and an HPA range:

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
    captiveCoreStructuredConfig:
      logLevel: info
      historyArchiveUrls:
        - https://history.stellar.org/prd/core-testnet/core_testnet_001
  autoscaling:
    minReplicas: 2
    maxReplicas: 3
    targetCpuUtilizationPercentage: 70
```

The `stellarCoreUrl` is an in-cluster service endpoint. In a multi-cluster HA
layout, make this endpoint a declaratively managed cross-cluster DNS name or
service-mesh address and give each cluster its own ArgoCD Application destination.

## 5. Verify a clean ArgoCD sync

To request a sync from the terminal, run `make argocd-sync
APP=stellar-k8s-gitops-v1`. The target requires an explicit Application name and
uses the currently configured ArgoCD CLI context.

Use the ArgoCD application tree or read-only ArgoCD status output and record the
result in the PR description. The expected high-level state is:

```text
stellar-k8s-gitops-v1       Synced     Healthy
stellar-operator            Synced     Healthy
stellar-testnet-validator   Synced     Progressing/Healthy while catching up
stellar-soroban-rpc          Synced     Progressing/Healthy while catching up
```

A clean sync on a fresh cluster must not require manually creating:

- `stellar-system` or `stellar-testnet` namespaces
- the `StellarNode` CRD
- the Validator Secret in the development profile
- StatefulSets, Deployments, Services, HPAs, ConfigMaps, or PVCs

The committed validation checklist and terminal-session template are in
[`examples/argocd/v1/validation/README.md`](../../examples/argocd/v1/validation/README.md).

## Multi-cluster HA pattern

Repeat the child Application per ArgoCD cluster, changing only declarative inputs:

1. Give each cluster a unique Application name, such as
   `stellar-testnet-validator-us-east` and `stellar-testnet-validator-eu-west`.
2. Set `destination.name` to the registered ArgoCD cluster name, or use the
   cluster API server URL.
3. Use a cluster-specific values file for storage class, node selectors, and
   network endpoints.
4. Keep the same `targetRevision` and v1 chart path in every cluster.
5. Use separate Validator identities and quorum configuration; never copy one
   signing seed into multiple active Validators.
6. For Soroban RPC, point each cluster's `stellarCoreUrl` at its local or
   intentionally selected HA Core endpoint.

ArgoCD's ApplicationSet controller can generate these Applications from a
committed cluster inventory.

## Finalizers, deletion, and PVC lifecycle

Stellar-K8s uses a kube-rs finalizer on each `StellarNode`:

```text
stellarnode.stellar.org/finalizer
```

The lifecycle is deliberately ordered:

1. ArgoCD marks the `StellarNode` for deletion when the resource disappears from
   the desired Git revision or when an Application is deleted with its resources
   finalizer.
2. Kubernetes sets `metadata.deletionTimestamp` but keeps the custom resource.
3. The operator's cleanup path deletes operator-managed external resources in
   dependency order: network and service objects, workloads, configuration, and
   finally the data PVC according to `spec.storage.retentionPolicy`.
4. Only after cleanup succeeds does the operator remove its finalizer.
5. Kubernetes completes deletion. ArgoCD then prunes the remaining desired-state
   object and reports the Application healthy again.

### The safe ArgoCD deletion procedure

- To remove a node, delete its `Application` manifest from Git and let ArgoCD
  prune it, or remove the node from the versioned chart values/profile.
- Keep the ArgoCD Application finalizer
  `resources-finalizer.argocd.argoproj.io` enabled.
- Keep `PruneLast=true` and foreground propagation enabled.
- Wait for the Application tree to show the `StellarNode` and its children gone.
- For data that must survive node removal, set `retentionPolicy: Retain` before
  the deletion is merged.
- For disposable Testnet data, `retentionPolicy: Delete` permits the operator to
  delete the PVC during finalizer cleanup.

Do **not** delete the `StellarNode` finalizer by hand. That bypasses the
operator's cleanup contract and can leave PVCs, Services, or credentials
orphaned.

For more details on the kube-rs finalizer design, see
[`docs/adr/0003-kube-rs-finalizers.md`](../adr/0003-kube-rs-finalizers.md).

## Versioning and promotion

- Pin production Applications to a tag or commit SHA, not `main`.
- Treat `examples/argocd/v1` and its chart as a public API.
- Make a new `v2` directory for breaking changes such as a new CRD version,
  changed resource names, or a changed secret contract.
- Promote the same Git revision through environments by changing only the
  destination cluster and approved values overlays.

## Troubleshooting

| Symptom | Declarative check |
| --- | --- |
| Application is `Unknown` | Confirm the ArgoCD repo-server can read the repository and the Application path exists. |
| `StellarNode` is rejected | Compare rendered YAML with `config/crd/stellarnode-crd.yaml`; ensure node type-specific config is present. |
| PVC is `Pending` | Set the cluster's real StorageClass in the profile values; do not patch the PVC. |
| Validator pod is not ready | Confirm the secret provider has materialized the seed and wait for Testnet catch-up. |
| Soroban RPC cannot reach Core | Change `soroban.stellarCoreUrl` in Git to the intended Service/DNS endpoint. |
| Resource is stuck terminating | Fix the operator cleanup dependency declaratively and allow the finalizer retry. |

For the Flux CD equivalent of this guide, see [`docs/gitops/flux.md`](flux.md).
