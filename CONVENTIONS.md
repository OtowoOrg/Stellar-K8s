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
├── docs/                All project documentation (see mkdocs.yml)
├── examples/            Ready-to-use StellarNode manifests
├── monitoring/          Grafana dashboards and Prometheus alert rules
├── policy/              CEL and OPA policies
├── schemas/             JSON schemas
├── scripts/             Operational scripts
│   ├── ci/              CI helper scripts
│   └── lib/             Shared script library functions
│   ├── lib/             Shared script library functions
│   ├── ci/              CI validation helpers
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
| File names | `kebab-case.sh` | `cleanup.sh`, `preflight.sh` |
| Operational scripts | live in `scripts/` | `scripts/cleanup.sh`, `scripts/repo-health.sh` |
| One-off / historical | delete or fold into a supported tool | Prefer `scripts/cleanup.sh` over new ad-hoc helpers |

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
   added to `mkdocs.yml` under the appropriate section.

3. **Config files**: Go under `config/` with a clear subdirectory. Use `config/crd/` for CRDs,
   `config/samples/` for test resources, and `config/manifests/` for OLM bases.

4. **Scripts**: Operational scripts go in `scripts/`. Do not add one-off archive
   or batch helpers — fold cleanup into `scripts/cleanup.sh` (or remove the script).
   Scripts must not live at the repository root.
4. **Scripts**: Operational scripts go in `scripts/`. One-off or historical scripts should not
   be committed to the repository. Scripts must not live at the repository root.

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

## Structured Logging Conventions

All structured log field names must use the field name constants defined in `src/logging/fields.rs` (`stellar_k8s::logging::fields as F`).

- **Consistent keys**: Always reference `F::NODE`, `F::NAMESPACE`, `F::RECONCILE_ID`, `F::ERROR`, `F::DURATION_MS`, etc.
- **No string literals for field names**: Eliminates key drift across CI log aggregators and dashboards.
- **Redaction**: All sensitive fields (seeds, tokens, secrets) are scrubbed automatically via `RedactingFields`.

---

## Integration Test Teardown Conventions

All integration and E2E tests that allocate Kubernetes resources or temporary state MUST use the RAII guards defined in `tests/common/mod.rs`:

- `NamespaceGuard`: Automatically deletes temporary test namespaces on `Drop`.
- `StellarNodeGuard`: Automatically deletes temporary `StellarNode` CRs on `Drop`.
- `ManifestGuard`: Automatically deletes applied YAML manifests on `Drop`.
- `E2eTestGuard`: Composite teardown guard managing nodes, operator manifests, and namespaces in proper dependency order.

Using `Drop` guards ensures resource cleanup happens deterministically even if a test panics or returns early.

---

## Enforcement

These conventions are enforced by:

- **Pre-commit hooks** (`shellcheck`, `make fmt`, `yamllint`) — run `make pre-commit-install`
- **CI lint step** (`make lint`, `make fmt-check`) — runs on every PR
- **PR checklist** in [CONTRIBUTING.md](CONTRIBUTING.md#9-repo-health-checklist)

If you find a file that violates these conventions and is not covered by the checklist, open
a PR to fix it or add it to the checklist.

