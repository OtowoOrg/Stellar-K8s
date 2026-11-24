# Unified Observability Contract

Issue #1481 defines a **versioned resource-attribute vocabulary** shared by
logs, metrics, and traces. Signals correlate from pod identity; operators do
not translate field names by hand.

## Schema

Published catalog: [`schemas/observability/resource-attributes.v1.json`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/schemas/observability/resource-attributes.v1.json)

| Field | Required | Source |
|-------|----------|--------|
| `service.name` | yes | `OTEL_SERVICE_NAME` |
| `service.instance.id` | yes | `POD_UID` |
| `k8s.pod.name` | yes | `POD_NAME` |
| `k8s.namespace.name` | yes | `POD_NAMESPACE` |
| `k8s.node.name` | yes | `NODE_NAME` |
| `stellar.observability.contract.version` | yes | `1.0.0` |

`contractVersion`, `compatibleWith`, and `minCompatibleVersion` live on the
schema document. Compatible emitters keep working across a minor bump.

## Ingest

The bundled OpenTelemetry Collector (`charts/stellar-operator/templates/otel-collector.yaml`):

1. Stamps the contract version when it is absent.
2. Forwards **valid** signals to Tempo / Loki / backends.
3. Routes **violations** to `file/deadletter` (`traces/deadletter`,
   `metrics/deadletter`, `logs/deadletter`). Nothing is silently dropped.

The same rules run in-process via `stellar_k8s::observability_contract`.

## Generator

```bash
python3 scripts/generate-observability-instrumentation.py \
  --service stellar-golden-path \
  --out generated/observability_instrumentation.rs
```

## CI

`python3 scripts/ci/lint-observability-contract.py` fails the build when a
`KeyValue::new("…")` resource key is not in the schema.

## Grafana

Import `monitoring/grafana-log-trace-correlation.json`. Derived fields pivot
from Loki `trace_id` to Tempo using `k8s.pod.name` and `service.instance.id`.

## Versioning and compatibility rules (#1960)

`contractVersion` is semantic. `python3 scripts/ci/check-observability-schema-compat.py`
(CI, against `origin/main`) enforces:

| Change | Required bump |
|---|---|
| remove an attribute, add a *required* attribute, change type, narrow an enum, lower `maxLength`, raise `minLength`, change `pattern` | **MAJOR** (and raise `minCompatibleVersion`) |
| add an optional attribute, widen an enum, relax limits | at least **MINOR**, keep the previous version in `compatibleWith` |
| any schema change | `contractVersion` must increase |

**Deprecation/migration:** mark the attribute `"deprecated": true` with a replacement in
its description for one MINOR release, keep accepting it (legacy names are normalized via
`x-legacyAliases`), then remove it in the next MAJOR.

## CI checks

`python3 scripts/ci/lint-observability-contract.py` now also:

* validates every `tests/fixtures/observability/*.json` (`expect: valid|invalid`, errors name the attribute);
  the same fixtures are replayed against the Rust runtime validator (`tests/observability_contract.rs`);
* fails if the collector's quarantine expression requires a different attribute set than the schema,
  or stamps a different contract version.

CI is strict about **unknown attributes**; ingest deliberately tolerates them (and normalizes legacy
aliases) so SDK migrations do not drop telemetry. A missing contract version is stamped by the collector
at ingest, not quarantined.

## Dead-letter metrics (#1960)

The collector counts quarantined records with the `count` connector and serves them on
`:8889` (`stellar_otel_deadletter_{spans,metrics,logs}`); accepted volume comes from the collector's own
`otelcol_receiver_accepted_*`. `monitoring/otel-contract-alerts.yaml` records
`stellar:otel_deadletter_ratio:rate5m` and alerts above **0.01 %** for 30 min.

> Validation status: the rendered collector config is checked structurally (all pipeline
> references resolve; `helm template`). It has **not** been run in a live collector 0.99.0, so the
> exact exported metric suffixes (`_total`) are matched with a tolerant regex.

## Grafana log ↔ trace pivot (#1960)

`config/monitoring/grafana/provisioning/datasources/prometheus.yaml` provisions explicit UIDs
(`prometheus`, `loki`, `tempo`). Loki has a derived field turning `trace_id` (JSON, logfmt or camelCase)
into a Tempo link; Tempo's `tracesToLogsV2` links back using canonical attributes
(`k8s.pod.name`, `k8s.namespace.name`, `service.name`). Tests verify cross-references and that the regex
extracts the id from real log shapes; the click-through itself needs a running Grafana.

## Pilot

The golden-path service (`emit_golden_path_signals`) emits one log, one
metric, and one span with the **same** resource identity.

## Mean-time-to-correlate: what is and is not measured

`observability_contract::mean_time_to_correlate_ms` is a **model** of the manual-translation steps removed
by the contract, not a stopwatch measurement. No seeded production incident was run for this PR, so
there is **no measured pre/post MTTC**. Remaining gaps: services outside the golden path still emit legacy
attribute names (normalized at ingest but not in dashboards), and trace ids must be present in log lines
for the derived field to match.

## Validation

```bash
python3 scripts/ci/lint-observability-contract.py
python3 -m unittest scripts.tests.test_lint_observability_contract
cargo test --lib observability_contract -- --nocapture
cargo test --test observability_contract -- --nocapture
```
