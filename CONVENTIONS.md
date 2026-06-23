# Repository Conventions

This document defines the naming and structural conventions for the Stellar-K8s repository.
Following these conventions keeps the directory tree easy to skim and reduces surprises for
contributors navigating the codebase for the first time.

---

## Directory Layout

```
Stellar-K8s/
├── assets/              Logo and static images
├── benchmarks/          k6 performance tests and baseline results
├── bundle/              OLM bundle (generated — do not hand-edit)
├── charts/              Helm charts
│   └── stellar-operator/
├── config/              Kubernetes manifests and CRDs (see config/README.md)
│   ├── crd/             Generated CRD YAML files
│   ├── samples/         Example resources for testing
│   ├── manifests/       OLM CSV bases and Gatekeeper policies
│   └── dev/             Local dev kubeconfigs (not for production)
├── docs/                All project documentation (see docs/README.md)
├── examples/            Ready-to-use StellarNode manifests
├── monitoring/          Grafana dashboards and Prometheus alert rules
├── policy/              CEL and OPA policies
├── schemas/             JSON schemas
├── scripts/             Operational scripts
│   ├── dev-utils/       Development helper utilities
│   ├── lib/             Shared script library functions
│   └── archive/         Historical one-off scripts (not part of normal workflow)
├── security/            Security policies and SBOM
├── src/                 Rust source code
├── tests/               Integration and E2E tests
└── tools/               CLI and utility tools
```

Each top-level directory has a single, obvious purpose. If a new directory is needed, add it
here and keep its name lowercase with hyphens (`kebab-case`).

---

## Naming Rules

### Rust source files and modules

| Element | Convention | Example |
|---|---|---|
| File names | `snake_case.rs` | `disk_scaler.rs` |
| Module directories | `snake_case/` | `rest_api/` |
| Public types and traits | `UpperCamelCase` | `StellarNode` |
| Public functions | `snake_case` | `reconcile_node` |
| Constants | `SCREAMING_SNAKE_CASE` | `STELLAR_NODE_FINALIZER` |
| Feature flags (`#[cfg]`) | `kebab-case` | `rest-api`, `metrics` |

### Documentation files

| Element | Convention | Example |
|---|---|---|
| File names | `kebab-case.md` | `disk-scaling.md` |
| Directory names | `kebab-case/` | `deployment-guides/` |
| Root-level docs | ALL-CAPS.md for repo meta | `README.md`, `CONTRIBUTING.md` |

Documentation files that belong to a topic area go in the matching `docs/<topic>/` subdirectory.
Root-level files (`README.md`, `DEVELOPMENT.md`, `CONTRIBUTING.md`, `CONVENTIONS.md`) are
entry points only — detailed content belongs in `docs/`.

### Shell scripts

| Element | Convention | Example |
|---|---|---|
| File names | `kebab-case.sh` | `setup-mac.sh` |
| Operational scripts | live in `scripts/` | `scripts/validate.sh` |
| Historical / one-off | move to `scripts/archive/` | `scripts/archive/create_batch_2_issues.sh` |

Every script must pass `shellcheck -S error` before merging.

### Kubernetes manifests

| Element | Convention | Example |
|---|---|---|
| CRD files | `stellar{feature}-crd.yaml` | `stellarnode-crd.yaml` |
| Sample files | descriptive, lowercase, hyphens | `test-stellarnode.yaml` |
| Example files | feature-based, no issue numbers | `validator-mainnet.yaml` |
| Helm chart values | `values.yaml` (defaults), `values-ha.yaml` (variants) | — |

**CRD naming**: All CRD YAML files under `config/crd/` follow the `stellar{feature}-crd.yaml`
pattern. The resource kind in the file itself uses `UpperCamelCase` (e.g. `StellarNode`).

**Example manifests**: Files in `examples/` use descriptive, feature-based names. Issue numbers
must not appear in filenames — use the feature name instead
(e.g. `advanced-features-compliance-upgrade-scaling.yaml`, not `advanced-features-500-503.yaml`).

---

## File Placement Rules

1. **Source files**: Go in the most specific module directory under `src/`. Do not place new
   `.rs` files directly in `src/` unless they are top-level entry points (`main.rs`, `lib.rs`,
   `error.rs`, `cli.rs`).

2. **Documentation files**: Go in the matching `docs/<topic>/` subdirectory. New files must be
   linked from `docs/README.md` under the appropriate section.

3. **Config files**: Go under `config/` with a clear subdirectory. Use `config/crd/` for CRDs,
   `config/samples/` for test resources, and `config/manifests/` for OLM bases.

4. **Scripts**: Operational scripts go in `scripts/`. One-off or historical scripts go in
   `scripts/archive/`. Scripts must not live at the repository root.

5. **Generated files**: Never hand-edit generated files. Always regenerate from source.
   See the [Regenerating Manifests](DEVELOPMENT.md#regenerating-manifests) table.

---

## Generated vs Hand-Written Files

| File or directory | Hand-written? | Source of truth |
|---|---|---|
| `config/crd/*.yaml` | No | `src/crd/` Rust types |
| `bundle/manifests/*.yaml` | No | `config/manifests/bases/` + operator-sdk |
| `docs/api-reference.md` | No | `src/crd/` + `make generate-api-docs` |
| Shell completions | No | `src/cli.rs` + `make completions` |
| `charts/stellar-operator/values.yaml` | Yes | — |
| `config/operator-config.yaml` | Yes | — |
| `docs/**/*.md` (other than api-reference) | Yes | — |

---

## Enforcement

These conventions are enforced by:

- **Pre-commit hooks** (`shellcheck`, `cargo fmt`, `yamllint`) — run `make pre-commit-install`
- **CI lint step** (`cargo clippy`, `make fmt-check`) — runs on every PR
- **PR checklist** in [CONTRIBUTING.md](CONTRIBUTING.md#9-repo-health-checklist)

If you find a file that violates these conventions and is not covered by the checklist, open
a PR to fix it or add it to the checklist.
