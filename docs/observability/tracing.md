# Distributed Tracing (OpenTelemetry)

The operator, REST API, admission webhook, and API gateway emit **OpenTelemetry**
spans and propagate W3C `traceparent` across HTTP calls. JSON logs include
`trace_id` and `span_id` when a span is active. Tracing is **off** unless
`OTEL_EXPORTER_OTLP_ENDPOINT` is set — logs still work without a collector.

## Service inventory

| Service | Language | HTTP entry | Outbound | Logging | Tracing |
|---------|----------|------------|----------|---------|---------|
| `stellar-operator` (reconciler) | Rust / kube-rs | n/a (watch loop) | Kubernetes API | `tracing` JSON | spans via `#[instrument]` + OTel layer |
| REST API | Rust / Axum | `:8080` / `:9090` | — | same subscriber | `http_trace_middleware` |
| Admission webhook | Rust / Axum | `/validate`, `/mutate` | OPA/Gatekeeper (`reqwest`) | same | middleware + header injection |
| API gateway | Rust / Axum | configured bind | upstream `reqwest` | same | middleware + header injection |
| Diagnostic sidecar | Rust | eBPF scrape | `localhost:9435` | `tracing` | injects `traceparent` on scrape |

## Environment variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | unset (tracing disabled) | OTLP gRPC endpoint, e.g. `http://localhost:4317` |
| `OTEL_SERVICE_NAME` | `stellar-operator` | Resource `service.name` |
| `OTEL_TRACES_SAMPLER` | `parentbased_traceidratio` | `always_on`, `always_off`, `traceidratio`, or parent-based |
| `OTEL_TRACES_SAMPLER_ARG` | `1.0` | Ratio in `0.0`–`1.0` |

Helm injects these when `otel.enabled: true` (`charts/stellar-operator/values.yaml`).
Do not hard-code production collector URLs.

Sensitive attributes (`authorization`, cookies, tokens, passwords, client IPs)
are scrubbed before export.

## Local Jaeger

```bash
docker compose -f docker-compose.tracing.yml up -d
export OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317
export OTEL_SERVICE_NAME=stellar-operator
export OTEL_TRACES_SAMPLER=parentbased_traceidratio
export OTEL_TRACES_SAMPLER_ARG=1.0
make run-local
```

Open [http://localhost:16686](http://localhost:16686), search for service
`stellar-operator`, and open a trace. A REST call that triggers webhook
delegation or the API gateway should show **one trace id** with nested spans
(REST → gateway → upstream, or webhook → OPA).

In-cluster, set `otel.collector.config.jaeger.enabled` or `.tempo.enabled` in
Helm values. The bundled collector image is `otel/opentelemetry-collector-contrib`.

## Unified resource-attribute contract (issue #1481)

Logs, metrics, and traces share the catalog in
[`schemas/observability/resource-attributes.v1.json`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/schemas/observability/resource-attributes.v1.json).
Every accepted payload includes `stellar.observability.contract.version`.
The collector quarantines violations to `file/deadletter` instead of dropping
them. See [unified-contract.md](unified-contract.md).

## Trace / log correlation

Structured logs use the field names in `src/logging/fields.rs`:

- `trace_id` — 32-char W3C hex
- `span_id` — 16-char W3C hex

`OtelTraceIdLayer` copies those fields onto log lines when a span exists. If
tracing is disabled the fields are omitted; log level and message stay intact.

## Verification (CI, no Jaeger required)

```bash
cargo test --test otel_propagation -- --nocapture
```

The test sends `GET /a` through in-process services A → B → C, asserts a single
trace id, parent/child span links, and that captured logs contain that id.
