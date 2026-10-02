# GitOps Deployment Architecture via ArgoCD

This document defines the canonical GitOps deployment architecture for Stellar-K8s.
It covers the complete ArgoCD integration, multi-environment Kustomize overlay
structure, automated state verification gates, and the pull-request workflow that
enforces GitHub as the single, immutable source of truth for all node
infrastructure.

> **Non-negotiable constraint:** Manual editing of live cluster objects with
> `kubectl edit`, `kubectl patch`, or any imperative command is **forbidden** in
> any environment managed by this pipeline. Every change to node infrastructure
> — version bumps, resource adjustments, scaling policy updates — must be
> expressed as a Git commit and flow through the pull-request process described
> below. ArgoCD will detect and revert any out-of-band mutation within its
> reconciliation interval.

## Table of Contents

1. [Architecture Overview](#architecture-overview)
2. [Repository Layout](#repository-layout)
3. [Prerequisites](#prerequisites)
4. [ArgoCD Bootstrap](#argocd-bootstrap)
5. [Kustomize Overlay Structure](#kustomize-overlay-structure)
6. [Multi-Environment Applications](#multi-environment-applications)
7. [Captive Core Upgrade Verification Gates](#captive-core-upgrade-verification-gates)
8. [Pull-Request Workflow](#pull-request-workflow)
9. [Sync Wave Ordering](#sync-wave-ordering)
10. [Secret Management](#secret-management)
11. [Drift Detection and Automatic Remediation](#drift-detection-and-automatic-remediation)
12. [Validation: End-to-End Sync in Under 3 Minutes](#validation-end-to-end-sync-in-under-3-minutes)
13. [Troubleshooting](#troubleshooting)

---

## Architecture Overview

```
GitHub (source of truth)
        │
        │  PR merged to main
        ▼
  ArgoCD repo-server
  (polls every 3 min or via webhook)
        │
        │  renders Kustomize overlays
        ▼
  ArgoCD Application Controller
        │
        │  applies diffs via server-side apply
        ▼
  Kubernetes API Server
        │
        │  StellarNode CR created/updated
        ▼
  Stellar-K8s Operator
  (kube-rs reconciliation loop)
        │
        │  manages StatefulSets, Deployments,
        │  Services, ConfigMaps, PVCs
        ▼
  Stellar Core / Horizon / Soroban RPC Pods
```

The pipeline has three logical layers:

| Layer | Tool | Responsibility |
|---|---|---|
| Source of truth | GitHub | Version control, PR review, branch protection |
| Delivery | ArgoCD | Declarative sync, drift remediation, sync ordering |
| Execution | Stellar-K8s Operator | CRD reconciliation into workloads |

---

## Repository Layout

The GitOps configuration lives entirely under `examples/gitops/`:

```
examples/gitops/
├── base/                         # Shared StellarNode base resources
│   ├── kustomization.yaml
│   ├── namespace.yaml
│   ├── validator.yaml
│   ├── soroban-rpc.yaml
│   └── horizon.yaml
├── overlays/
│   ├── testnet/                  # Testnet-specific patches
│   │   ├── kustomization.yaml
│   │   ├── validator-patch.yaml
│   │   ├── soroban-rpc-patch.yaml
│   │   └── horizon-patch.yaml
│   ├── futurenet/                # Futurenet-specific patches
│   │   ├── kustomization.yaml
│   │   ├── validator-patch.yaml
│   │   └── soroban-rpc-patch.yaml
│   └── mainnet/                  # Mainnet-specific patches
│       ├── kustomization.yaml
│       ├── validator-patch.yaml
│       ├── soroban-rpc-patch.yaml
│       └── horizon-patch.yaml
└── argocd/
    ├── app-of-apps.yaml          # Root Application (bootstrap entry point)
    ├── testnet-app.yaml
    ├── futurenet-app.yaml
    └── mainnet-app.yaml
```

ArgoCD Application manifests are stored alongside the Kustomize overlays so the
entire deployment configuration is version-controlled as a unit.

---

## Prerequisites

- Kubernetes 1.28+ cluster with ArgoCD installed in the `argocd` namespace.
- The Stellar-K8s operator deployed (wave `-20` handles this automatically via
  the app-of-apps bootstrap if you have not installed it separately).
- Git access from the ArgoCD repo-server to
  `https://github.com/agnesnaomiolim-cloud/Stellar-K8s`.
- Namespace secrets created before sync (see [Secret Management](#secret-management)).

Install ArgoCD if not present:

```bash
kubectl create namespace argocd
kubectl apply -n argocd \
  -f https://raw.githubusercontent.com/argoproj/argo-cd/stable/manifests/install.yaml
kubectl rollout status deploy/argocd-server -n argocd
```

---

## ArgoCD Bootstrap

The entire deployment is initialised with a single command. ArgoCD then manages
everything else declaratively:

```bash
# Register the repository (once per cluster)
argocd repo add https://github.com/agnesnaomiolim-cloud/Stellar-K8s \
  --name stellar-k8s

# Bootstrap the app-of-apps (once per cluster)
kubectl apply -f examples/gitops/argocd/app-of-apps.yaml
```

This creates the root Application. ArgoCD renders
`examples/gitops/argocd/` and discovers the per-environment child Applications.
Each child Application points at its Kustomize overlay and has automated
prune + self-heal enabled.

After bootstrap, monitor convergence:

```bash
argocd app list
argocd app wait stellar-k8s-gitops --health --sync --timeout 180
```

---

## Kustomize Overlay Structure

### Base

The base contains network-neutral resource skeletons. No environment-specific
values (network passphrase, resource sizes, version tags) appear here.

```yaml
# examples/gitops/base/kustomization.yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
resources:
  - namespace.yaml
  - validator.yaml
  - soroban-rpc.yaml
  - horizon.yaml
commonLabels:
  app.kubernetes.io/managed-by: argocd
  app.kubernetes.io/part-of: stellar-k8s
```

### Overlay pattern

Each overlay uses `patches` to override only the fields that differ between
environments. This prevents copy-paste divergence and surfaces environment
differences as a small, reviewable diff.

```yaml
# examples/gitops/overlays/testnet/kustomization.yaml (example structure)
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
namespace: stellar-testnet
resources:
  - ../../base
namePrefix: testnet-
commonLabels:
  stellar.org/network: testnet
  app.kubernetes.io/managed-by: argocd
patches:
  - path: validator-patch.yaml
    target:
      kind: StellarNode
      name: validator
  - path: soroban-rpc-patch.yaml
    target:
      kind: StellarNode
      name: soroban-rpc
  - path: horizon-patch.yaml
    target:
      kind: StellarNode
      name: horizon
```

The actual overlay files are under `examples/gitops/overlays/{testnet,futurenet,mainnet}/`.

### Rendering overlays locally

Verify the rendered output before committing:

```bash
# Testnet
kubectl kustomize examples/gitops/overlays/testnet

# Futurenet
kubectl kustomize examples/gitops/overlays/futurenet

# Mainnet
kubectl kustomize examples/gitops/overlays/mainnet
```

Pipe through `kubeconform` for schema validation:

```bash
kubectl kustomize examples/gitops/overlays/testnet \
  | kubeconform -strict -summary
```

---

## Multi-Environment Applications

Each environment maps to one ArgoCD `Application`. The Applications are defined
in `examples/gitops/argocd/` and managed by the app-of-apps root.

| Application | Overlay path | Destination namespace | Auto-sync |
|---|---|---|---|
| `stellar-testnet` | `overlays/testnet` | `stellar-testnet` | Yes |
| `stellar-futurenet` | `overlays/futurenet` | `stellar-futurenet` | Yes |
| `stellar-mainnet` | `overlays/mainnet` | `stellar-mainnet` | **Manual sync only** |

Mainnet is intentionally set to manual sync. Promoting a change to mainnet
requires a second explicit approval step in the PR (see
[Pull-Request Workflow](#pull-request-workflow)).

---

## Captive Core Upgrade Verification Gates

Before ArgoCD syncs a new Stellar Core (Captive Core) version to any
environment, the following automated checks must pass. These run in the CI
pipeline on every PR that bumps `spec.version` in any StellarNode manifest.

### Gate 1 — Schema validation

```bash
# Validate CRD schema conformance
kubectl kustomize examples/gitops/overlays/testnet \
  | kubeconform \
      -strict \
      -schema-location default \
      -schema-location 'https://raw.githubusercontent.com/datreeio/CRDs-catalog/main/{{.Group}}/{{.ResourceKind}}_{{.ResourceAPIVersion}}.json' \
      -summary
```

A non-zero exit code blocks the PR merge.

### Gate 2 — Version compatibility check

```bash
# Confirm the requested version exists in the operator compatibility matrix
stellar-operator compat-check \
  --version "$(yq '.spec.version' examples/gitops/overlays/testnet/validator-patch.yaml)" \
  --network testnet
```

See [docs/compat-matrix.md](../compat-matrix.md) for the full version matrix.

### Gate 3 — OPA / Gatekeeper dry-run

```bash
# Dry-run against Gatekeeper policies before applying to cluster
kubectl kustomize examples/gitops/overlays/testnet \
  | conftest test --policy policies/gitops/ -
```

Policies enforced:
- `require-resource-limits` — all StellarNode resources must have CPU/memory limits.
- `deny-latest-tag` — `spec.version` must be a pinned semver tag, not `latest`.
- `require-seed-secret` — Validators must reference a named Kubernetes Secret, not an inline value.
- `mainnet-retain-policy` — Mainnet StellarNodes must set `retentionPolicy: Retain`.

### Gate 4 — Testnet smoke test (staging promotion gate)

After the PR merges to `main` and ArgoCD syncs testnet, the following job runs
automatically via GitHub Actions:

```bash
# Wait for the StellarNode to reach Ready on testnet
kubectl wait stellarnode/testnet-validator \
  --namespace stellar-testnet \
  --for=condition=Ready \
  --timeout=300s

# Verify ledger is advancing
stellar-operator status --name testnet-validator --namespace stellar-testnet
```

If the testnet smoke test fails, the workflow opens an automated revert PR and
blocks the futurenet and mainnet promotion labels from being applied.

---

## Pull-Request Workflow

All infrastructure changes follow this pull-request lifecycle:

```
1. Fork / branch from main
   git checkout -b feat/bump-core-v21.1.0

2. Edit the relevant overlay patch
   # e.g., bump spec.version in overlays/testnet/validator-patch.yaml

3. Render and validate locally
   kubectl kustomize examples/gitops/overlays/testnet | kubeconform -strict -summary

4. Open PR against main
   - Required reviewers: ≥1 approved review
   - Required CI checks:
       ✓ schema-validation
       ✓ compat-check
       ✓ conftest-policies
       ✓ kubeconform-strict

5. Merge PR to main
   ArgoCD detects the change (webhook or 3-minute poll) and syncs testnet.

6. Automated testnet smoke test (≤3 minutes after merge)
   GitHub Actions verifies StellarNode is Ready and ledger is advancing.

7. Apply label promote/futurenet to PR (optional)
   GitHub Actions syncs futurenet Application.

8. Apply label promote/mainnet to PR (requires second approver)
   GitHub Actions triggers manual ArgoCD sync for the mainnet Application.
```

Branch protection rules that must be enabled on `main`:
- Require pull request reviews before merging (minimum 1).
- Require status checks to pass before merging.
- Do not allow bypassing the above settings.
- Require linear history (no merge commits; squash or rebase only).

---

## Sync Wave Ordering

ArgoCD sync waves ensure resources are applied in the correct dependency order.

| Wave | Resources |
|---|---|
| `-20` | Stellar-K8s Operator (CRDs, RBAC, Deployment) |
| `0` | Namespace, Secrets (ExternalSecret/Vault reference), ConfigMaps |
| `10` | StellarNode Custom Resources |

Waves are set via the `argocd.argoproj.io/sync-wave` annotation on each manifest
or in the Kustomize overlay:

```yaml
metadata:
  annotations:
    argocd.argoproj.io/sync-wave: "10"
```

The operator must be Ready before any StellarNode CR is applied. The `-20` wave
for the operator Application combined with `PruneLast=true` ensures that on
deletion the CRs are removed before the operator is torn down, giving finalizers
time to clean up PVCs.

---

## Secret Management

Never commit real validator seed keys or database credentials to Git.

Approved patterns:

### External Secrets Operator (recommended)

```yaml
# In the overlay — references AWS Secrets Manager or Vault
apiVersion: external-secrets.io/v1beta1
kind: ExternalSecret
metadata:
  name: validator-seed-testnet
  namespace: stellar-testnet
  annotations:
    argocd.argoproj.io/sync-wave: "0"
spec:
  refreshInterval: 1h
  secretStoreRef:
    name: aws-secrets-manager
    kind: ClusterSecretStore
  target:
    name: validator-seed-testnet
  data:
    - secretKey: STELLAR_CORE_SEED
      remoteRef:
        key: stellar/testnet/validator-seed
```

### Sealed Secrets (air-gapped / no external KMS)

```bash
# Encrypt and commit the sealed secret
kubectl create secret generic validator-seed-testnet \
  --from-literal=STELLAR_CORE_SEED=S... \
  --dry-run=client -o yaml \
  | kubeseal --format yaml > overlays/testnet/validator-seed-sealed.yaml
git add overlays/testnet/validator-seed-sealed.yaml
```

The plaintext secret never enters Git. Only the encrypted `SealedSecret` does.

### StellarNode reference

Once the Secret exists in the namespace (created by ExternalSecret or
SealedSecret), reference it in the StellarNode patch:

```yaml
spec:
  validatorConfig:
    seedSecretRef: validator-seed-testnet
```

See [docs/security/credentials-and-secrets.md](../security/credentials-and-secrets.md)
for the full approved patterns.

---

## Drift Detection and Automatic Remediation

ArgoCD is configured with `selfHeal: true` on all Applications. If any live
cluster object drifts from the Git-committed state — whether from a manual
`kubectl edit`, a cluster-level admission mutation, or operator side-effects —
ArgoCD detects the diff and re-applies the Git state within its sync interval
(default 3 minutes, configurable via webhook for sub-minute response).

To verify drift detection is working:

```bash
# Intentionally mutate a live object
kubectl patch stellarnode testnet-validator \
  -n stellar-testnet \
  --type merge \
  -p '{"spec":{"resources":{"limits":{"cpu":"99"}}}}'

# Within 3 minutes ArgoCD reverts the change.
# Watch the sync:
argocd app get stellar-testnet --watch
```

If you need to temporarily suspend self-heal (e.g., during an emergency
maintenance window), use:

```bash
argocd app set stellar-testnet --self-heal=false
# ... perform maintenance ...
argocd app set stellar-testnet --self-heal=true
argocd app sync stellar-testnet
```

Document the reason for the suspension in your incident timeline.

---

## Validation: End-to-End Sync in Under 3 Minutes

This procedure validates the full GitOps loop: a PR mutation merges, and ArgoCD
synchronises the live cluster state within 3 minutes.

### Step 1 — Configure ArgoCD webhook (eliminates poll lag)

```bash
# In your GitHub repository settings → Webhooks → Add webhook
# Payload URL: https://<argocd-server>/api/webhook
# Content type: application/json
# Events: push
```

With a webhook, ArgoCD triggers a sync within seconds of a push to `main`.

### Step 2 — Open and merge a test PR

```bash
git checkout -b test/gitops-validation-$(date +%s)

# Bump testnet validator CPU request by 1m as a trivial, safe change
sed -i 's/cpu: "500m"/cpu: "501m"/' \
  examples/gitops/overlays/testnet/validator-patch.yaml

git add examples/gitops/overlays/testnet/validator-patch.yaml
git commit -m "test: bump testnet validator CPU request for GitOps validation"
git push -u origin HEAD

# Open PR via GitHub CLI and merge immediately (skip review for test branches)
gh pr create --title "test: GitOps validation" --body "Automated validation PR" --base main
gh pr merge --squash --auto
```

### Step 3 — Measure time to convergence

```bash
START=$(date +%s)

# Wait until ArgoCD shows Synced + Healthy
argocd app wait stellar-testnet \
  --sync \
  --health \
  --timeout 180

END=$(date +%s)
echo "Convergence time: $((END - START)) seconds"
```

The convergence time should be under 180 seconds with a webhook configured. With
polling only, allow up to 3 minutes (180 s) for the poll cycle plus apply time.

### Step 4 — Verify the live StellarNode reflects the committed state

```bash
kubectl get stellarnode testnet-validator \
  -n stellar-testnet \
  -o jsonpath='{.spec.resources.requests.cpu}'
# Expected output: 501m
```

### Step 5 — Revert the test change

```bash
git checkout -b revert/gitops-validation
sed -i 's/cpu: "501m"/cpu: "500m"/' \
  examples/gitops/overlays/testnet/validator-patch.yaml
git add examples/gitops/overlays/testnet/validator-patch.yaml
git commit -m "revert: restore testnet validator CPU request"
git push -u origin HEAD
gh pr create --title "revert: GitOps validation cleanup" --base main
gh pr merge --squash --auto
```

---

## Troubleshooting

### Application stuck in `OutOfSync`

```bash
argocd app diff stellar-testnet
argocd app sync stellar-testnet --force
```

If the diff shows operator-managed fields (e.g., `status`, controller
annotations), add them to the `ignoreDifferences` block in the Application spec:

```yaml
spec:
  ignoreDifferences:
    - group: stellar.org
      kind: StellarNode
      jsonPointers:
        - /status
        - /metadata/annotations/kubectl.kubernetes.io~1last-applied-configuration
```

### StellarNode stuck due to finalizer

The kube-rs finalizer (`stellar.org/cleanup`) can block deletion if the operator
is unavailable. Never remove it manually during normal operations. If the
operator is being replaced, ensure the operator Application (wave `-20`) is
synced first, then re-sync the CR Applications.

Emergency finalizer removal (requires documented approval):

```bash
kubectl patch stellarnode <name> -n <namespace> \
  --type json \
  -p '[{"op":"remove","path":"/metadata/finalizers"}]'
```

Document this action in your incident timeline.

### ArgoCD cannot render Kustomize overlay

```bash
# Test kustomize rendering from the ArgoCD repo-server pod
kubectl exec -n argocd deploy/argocd-repo-server -- \
  argocd-cmp-server kustomize build examples/gitops/overlays/testnet
```

Common causes: missing `kustomization.yaml`, broken patch target selector, or
Kustomize version mismatch. See [docs/yaml-schema-validation.md](../yaml-schema-validation.md).

### Sync completes but StellarNode not Ready

```bash
# Check operator logs for reconciliation errors
kubectl logs -n stellar-system deploy/stellar-operator --tail=100

# Check StellarNode events
kubectl describe stellarnode testnet-validator -n stellar-testnet

# Use the diff utility
stellar-operator diff --name testnet-validator --namespace stellar-testnet
```

---

## Related Documentation

- [ArgoCD GitOps golden path (v1)](../../examples/argocd/v1/README.md)
- [docs/gitops/argocd.mdx](../gitops/argocd.mdx) — interactive manifest generator
- [docs/upgrade-workflow.md](../upgrade-workflow.md)
- [docs/security/credentials-and-secrets.md](../security/credentials-and-secrets.md)
- [docs/yaml-schema-validation.md](../yaml-schema-validation.md)
- [docs/gatekeeper-policies.md](../gatekeeper-policies.md)
- [docs/compat-matrix.md](../compat-matrix.md)
- [docs/diff-utility.md](../diff-utility.md)
