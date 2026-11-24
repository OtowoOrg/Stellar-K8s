# Maintenance & Validation Pipelines

This page documents the maintenance and validation tooling added for
issues #1064, #1065, #1066, and #1067.

## Periodic dead-code and unused-config report (#1064)

The [`dead-code-report.yml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/dead-code-report.yml)
workflow runs every Monday (and on manual dispatch). It executes
`scripts/dead-code-report.sh`, which collects:

- `rustc` dead-code diagnostics from `cargo check --all-targets`
- top-level keys in `config/operator-config.yaml` that are never
  referenced from `src/`

The result is uploaded as the `dead-code-report` artifact. The job is
informational and never fails the build.

**Verification**

```bash
SKIP_CARGO=1 ./scripts/dead-code-report.sh
make dead-code-report        # full report including the cargo pass
cat target/reports/dead-code-report.md
```

## CRD migration linter (#1065)

`scripts/crd_migration_lint.py` compares every manifest in `config/crd/`
against a baseline git ref (default `origin/main`) and fails when it finds
backward-incompatible evolution:

- a served API version was removed
- a schema property was removed or changed type
- an existing optional field became required

It runs in CI as the `crd-migration-lint` job of the quickstart validation
workflow, on every pull request.

**Verification**

```bash
python3 scripts/tests/test_crd_migration_lint.py
python3 scripts/crd_migration_lint.py --against origin/main
```

## Secret rotation integration check (#1066)

`scripts/secret-rotation-check.sh` rotates a secret (by annotating it with
a rotation timestamp) and then polls the consuming deployment, failing if
its available replicas ever drop below the pre-rotation baseline during
the observation window.

**Verification**

```bash
# No cluster required:
./scripts/secret-rotation-check.sh --dry-run

# Against a real cluster:
./scripts/secret-rotation-check.sh \
  --namespace stellar-system \
  --secret stellar-core-secret \
  --deployment stellar-k8s-operator \
  --window 60
```

CI smoke-tests the dry-run mode on every pull request.

## Golden-path quickstart validation pipeline (#1067)

The [`quickstart-validation.yml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/quickstart-validation.yml)
workflow guards the documented "golden path" for new users. Its
`golden-path` job runs `scripts/quickstart-golden-path.sh`, which checks
that:

- the quickstart entry points (`scripts/quickstart-verify.sh`, `README.md`,
  `Makefile`) exist
- the quickstart shell scripts parse cleanly
- every manifest under `config/crd/` and `config/samples/` is valid YAML
- the README still documents a quickstart section

**Verification**

```bash
./scripts/quickstart-golden-path.sh
```
