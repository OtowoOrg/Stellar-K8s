# Regenerating Charts and Bundle Manifests

Several files in this repository are **generated** and must never be hand-edited. This guide documents how to regenerate Helm charts and OLM bundle manifests from their sources of truth.

## Overview

| Generated artifact | Source of truth | Command |
|---|---|---|
| Helm chart templates (`charts/stellar-operator/templates/`) | `src/` Rust types + `config/` manifests | `helm template` or `make crd-gen` |
| CRD YAML (`config/crd/*.yaml`) | `src/crd/` Rust structs | `make crd-gen` |
| OLM bundle (`bundle/manifests/`, `bundle/metadata/`) | `config/manifests/bases/` | `make bundle` |
| API reference docs (`docs/api-reference.md`) | `src/crd/` + `scripts/generate-api-docs.py` | `make generate-api-docs` |

## Prerequisites

| Tool | Version | Required for |
|---|---|---|
| [operator-sdk](https://sdk.operatorframework.io/docs/installation/) | >= 1.42.0 | Bundle generation |
| [kustomize](https://kustomize.io/) | >= 5.x | Bundle generation |
| [helm](https://helm.sh/) | >= 3.x | Chart rendering |
| Python 3 | >= 3.12 | API docs generation |

```bash
# Verify installed tools
operator-sdk version
kustomize version
helm version
python3 --version
```

---

## Regenerating CRD Manifests

The StellarNode CRD and all other CRDs are generated from Rust types in `src/crd/`.

```bash
make crd-gen
```

This runs the `crdgen` binary which reads the `#[derive(JsonSchema)]` annotated structs and outputs updated CRD YAML files to `config/crd/`.

> **Note:** After modifying any `src/crd/*.rs` file, always run `make crd-gen` and commit the updated CRD alongside your Rust changes. The CI pipeline will fail if CRDs are stale.

---

## Regenerating the OLM Bundle

The Operator Lifecycle Manager (OLM) bundle is generated from the Kustomize bases in `config/manifests/`.

### Step-by-step

```bash
# 1. Generate OLM manifests from Helm chart + bases
make bundle

# 2. Validate the generated bundle
operator-sdk bundle validate ./bundle

# 3. (Optional) Build the bundle container image
make bundle-build
```

The `make bundle` target performs these steps internally:

1. Renders the Helm chart to raw manifests: `helm template stellar-operator charts/stellar-operator`
2. Generates Kustomize manifests: `operator-sdk generate kustomize manifests -q`
3. Produces the OLM bundle: `kustomize build config/manifests | operator-sdk generate bundle -q --overwrite --version $(VERSION)`
4. Validates the bundle structure with `operator-sdk bundle validate`

### Bundle structure

```
bundle/
├── manifests/
│   └── stellar-operator.clusterserviceversion.yaml   # ClusterServiceVersion
└── metadata/
    └── annotations.yaml                               # Bundle annotations
```

### Customizing bundle metadata

Edit these files directly (they are **not** auto-generated):

| File | Purpose |
|---|---|
| `bundle/metadata/annotations.yaml` | Channel, package name, mediatype |
| `config/manifests/bases/stellar-operator.clusterserviceversion.yaml` | CSV base (descriptor, icon, maintainer) |

After editing, run `make bundle` to regenerate the full CSV.

---

## Regenerating the Helm Chart

The Helm chart in `charts/stellar-operator/` is **hand-written** for the most part, but some components are derived from code:

### Chart templates that are hand-written

| Template | Description |
|---|---|
| `deployment.yaml` | Operator Deployment spec |
| `rbac.yaml` | ClusterRole, ClusterRoleBinding, Role, RoleBinding |
| `service.yaml` | Service definition |
| `serviceaccount.yaml` | ServiceAccount |
| `secret.yaml` | Kubernetes Secret |
| `externalsecret.yaml` | External Secrets Operator integration |
| `configmap.yaml` | Operator ConfigMap |
| `webhook.yaml` | Admission webhook configuration |
| `pdb.yaml` | PodDisruptionBudget |
| `hpa-*.yaml` | Horizontal Pod Autoscalers |
| `otel-collector.yaml` | OpenTelemetry sidecar |
| `scp-kafka-sidecar.yaml` | SCP Kafka sidecar |
| `byzantine-watcher.yaml` | Byzantine monitoring |
| `fork-detector.yaml` | Fork detection |
| `network-isolation.yaml` | Network policies |

### Testing chart changes

After modifying any template or `values.yaml`:

```bash
# Lint the chart
make helm-lint

# Validate against JSON schema
helm lint charts/stellar-operator --strict

# Render and inspect output
helm template stellar-operator charts/stellar-operator > /tmp/rendered.yaml

# Run Helm unit tests
helm unittest charts/stellar-operator --strict --color
```

### Updating Chart.yaml

| Field | Source | When to update |
|---|---|---|
| `version` | Manual | On each release / breaking chart change |
| `appVersion` | Manual | When the operator image version changes |
| `dependencies` | Manual | When adding new chart dependencies |

---

## Regenerating API Reference Docs

```bash
make generate-api-docs
```

This regenerates `docs/api-reference.md` from the CRD schema. The CI job `api-docs` in `.github/workflows/ci.yml` will fail if this file is stale.

---

## Testing Generated Assets in CI

The CI pipeline automatically validates generated files:

1. **CRD freshness** — the `api-docs` job regenerates `docs/api-reference.md` and fails if `git diff` shows changes
2. **Helm chart correctness** — `helm-lint` and `helm-test` jobs validate chart syntax and run unit tests
3. **Bundle validity** — `make bundle` validates with `operator-sdk bundle validate`

When making PR changes, always run:

```bash
make crd-gen          # Regenerate CRDs if Rust types changed
make bundle           # Regenerate OLM bundle if bases changed
make generate-api-docs  # Regenerate API docs if CRD schema changed
make helm-lint        # Validate Helm chart
```

Commit all regenerated files in the same PR as the source change.

---

## Troubleshooting

### `operator-sdk` not found

```bash
# Install operator-sdk (Linux / macOS / WSL2)
export ARCH=$(case $(uname -m) in x86_64) echo -n amd64 ;; aarch64) echo -n arm64 ;; esac)
export OS=$(uname | awk '{print tolower($0)}')
export OPERATOR_SDK_DL_URL=https://github.com/operator-framework/operator-sdk/releases/download/v1.42.0
curl -LO ${OPERATOR_SDK_DL_URL}/operator-sdk_${OS}_${ARCH}
chmod +x operator-sdk_${OS}_${ARCH} && sudo mv operator-sdk_${OS}_${ARCH} /usr/local/bin/operator-sdk
```

### `kustomize` not found

```bash
# Install kustomize
curl -s "https://raw.githubusercontent.com/kubernetes-sigs/kustomize/master/hack/install_kustomize.sh" | bash
sudo mv kustomize /usr/local/bin/
```

### Bundle validation fails

```bash
# Check the specific validation errors
operator-sdk bundle validate ./bundle --verbose

# Common fixes:
# - Ensure CRD YAML files exist in bundle/manifests/
# - Verify CSV metadata (displayName, description, installModes)
# - Check that all referenced images are valid
```

### Helm unit tests fail

```bash
# Install helm-unittest plugin
helm plugin install https://github.com/helm-unittest/helm-unittest.git

# Run with verbose output
helm unittest charts/stellar-operator --strict --color -v
```
