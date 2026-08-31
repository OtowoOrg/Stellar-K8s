# Helm Chart Testing

The operator chart is tested with **helm lint**, **helm unittest**, JSON schema
validation (`values.schema.json`), kubeconform on **rendered** manifests, and a
values-preservation upgrade check.

## Commands

```bash
make helm-lint
make helm-unittest
make helm-upgrade-test
make yaml-schema-validate
```

CI runs the same targets from `.github/workflows/ci.yml` (`helm-lint`,
`helm-upgrade-test`, `yaml-schema-validate`). Pin: Helm **3.14.0**,
helm-unittest **v0.5.1**.

## Edge cases (issue #1289)

`charts/stellar-operator/tests/edge_cases_test.yaml` covers:

- Resource requests/limits: defaults, minimal, production-like, empty
- `nodeSelector` absent, single label, multiple labels, production-like
- Affinity absent (default), node affinity, pod affinity, pod anti-affinity
- REST API / log level / watch namespace
- OpenTelemetry env injection when `otel.enabled=true`
- `logShipper` renders nothing when disabled and a full DaemonSet + RBAC +
  ConfigMap set when enabled (issue #1381)

Invalid values that `values.schema.json` must reject (logLevel, service type,
image pullPolicy) are asserted in the `helm-lint` CI job via `helm lint -f`.

## Upgrade testing

`Chart.yaml` version has been `0.1.0` since the chart was introduced
(`git log -- charts/stellar-operator/Chart.yaml`). There is **no** historical
chart tarball or git tag to install. We do **not** fabricate a previous
version.

Instead:

1. `charts/stellar-operator/tests/upgrade_preservation_test.yaml` renders
   `examples/values-production.yaml` (the last supported values schema) on the
   current templates and checks replicas, nodeSelector, affinity, tolerations,
   Service ports, and PDB.
2. `scripts/ci/helm-upgrade-test.sh` re-renders those values plus additive
   tracing overrides and asserts the original scheduling fields remain.

When a real previous chart version is published, replace the fixture with:

```bash
helm pull stellar-operator --version <previous>
helm template previous ./stellar-operator-<previous>.tgz -f values.yaml
helm template current charts/stellar-operator -f values.yaml
```

## Production-like values

Use `charts/stellar-operator/examples/values-production.yaml` as the canonical
production overlay (HA replicas, zone anti-affinity, dedicated node pool,
resource envelopes).
