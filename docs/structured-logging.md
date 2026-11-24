# Structured Logging

The Stellar-K8s operator and every sidecar emit **structured JSON logs** to
`stdout`/`stderr` by default. Machine-readable, field-stable log lines are the
contract that aggregation, alerting, and redaction builds on (issue #1381).

## JSON log schema

Each line is a single JSON object. The exact keys depend on the emitter, but
the stable core is:

| Field | Meaning |
|---|---|
| `timestamp` | RFC 3339 timestamp of the event |
| `level` | `TRACE`/`DEBUG`/`INFO`/`WARN`/`ERROR` |
| `message` | Human-readable event message |
| `target` | The Rust module that emitted the event |
| `span_id` / `trace_id` | Active `tracing` span context (see OpenTelemetry) |
| `*` | Loose fields from `tracing` events (e.g. `node_name`, `namespace`, `reconcile_id`) |

Example:

```json
{"timestamp":"2026-08-28T10:00:00.000000Z","level":"INFO","message":"Reconciled StellarNode","target":"stellar_k8s::controller::reconciler","node_name":"testnet-validator","namespace":"stellar"}
```

## What emits JSON

| Binary | Logging setup | JSON by default |
|---|---|---|
| `stellar-operator` | `init_subscriber(SubscriberConfig)`; `RUST_LOG` + `--log-format` (`json`/`pretty`) | Yes |
| `stellar-webhook` | same as operator | Yes |
| `stellar-health-sidecar` | `init_binary_subscriber(Level::INFO, Json)` | Yes |
| `stellar-watcher` | `init_binary_subscriber(log_level, Json)` | Yes |
| `stellar-log-shipper` | `registry().with(fmt::layer().json())` | Yes |
| `stellar-fork-detector` | `registry().with(fmt::layer().json())` | Yes |
| `stellar-logs` | `registry().with(fmt::layer().json())` + `EnvFilter` | Yes |

`operator.logLevel` (Helm) and `RUST_LOG` (env) control verbosity; they do not
change the JSON envelope.

## Aggregation

1. **Chart log shipper (optional).** Set `logShipper.enabled: true` in
   `charts/stellar-operator/values.yaml` to deploy a Fluent Bit DaemonSet that
   tails container logs on every node and forwards them to Loki or
   Elasticsearch. It is disabled by default so the committed Helm drift goldens
   stay stable; the operator's JSON stdout works with any aggregator.
2. **External aggregators.** Loki + Promtail, Filebeat, or the OTel Collector's
   `logs` pipeline (see `values.yaml` → `otel`) consume the JSON without
   transformation.

See [Log Aggregation Guide](log-aggregation-guide.md) for reference
setups.

## Alerting on logs

`monitoring/log-alerts.yaml` ships LogQL alert rules (Loki Ruler) for
error-rate spikes, panic/fatal events, log-volume anomalies, and a downed log
shipper. Deploy them next to your Loki/Prometheus instance.

## Verifying locally

```bash
# Run the operator binary and inspect its log line as JSON
cargo run --bin stellar-logs -- list --help
kubectl logs -n stellar-system deploy/stellar-operator | head -1 | python3 -m json.tool
```

Redaction of sensitive fields (validator seeds etc.) is handled separately by
`src/logging/log_scrub.rs` — see [Log Redaction Policy](log-redaction-policy.md).

---

## JSON Field Reference

All call-sites **must** use the constants from `src/logging/fields.rs` instead
of bare string literals. This prevents field-name drift between CI log
aggregation pipelines and runtime logs.

Import pattern:

```rust
use stellar_k8s::logging::fields as F;
```

| Constant | Wire name | Type | Description |
|---|---|---|---|
| `F::NODE` | `node` | string | StellarNode resource name |
| `F::NAMESPACE` | `namespace` | string | Kubernetes namespace |
| `F::NODE_TYPE` | `node_type` | string | `Validator` / `Horizon` / `SorobanRpc` |
| `F::CLUSTER` | `cluster` | string | Kubernetes cluster name or ARN |
| `F::K8S_NODE` | `k8s_node` | string | Kubernetes host node name |
| `F::RECONCILE_ID` | `reconcile_id` | u64 | Monotonic reconcile counter |
| `F::PHASE` | `phase` | string | Lifecycle phase (`init`, `reconcile`, `cleanup`) |
| `F::ERROR` | `error` | string | Error description — use `%err` (Display), not `?err` (Debug) |
| `F::DURATION_MS` | `duration_ms` | u64 | Operation duration in milliseconds |
| `F::COMPONENT` | `component` | string | Sub-system emitting the log |
| `F::LEDGER` | `ledger` | u64 | Stellar ledger sequence number |
| `F::VERSION` | `version` | string | Image / software version |
| `F::REGION` | `region` | string | Cloud / geographic region |
| `F::JOB_ID` | `job_id` | string | Background job identifier |
| `F::AUDIT_ACTION` | `audit_action` | string | Audit trail action string |
| `F::SCRUB_PATTERN` | `scrub_pattern` | string | Regex pattern name that triggered redaction |
| `F::TRACE_ID` | `trace_id` | string | W3C trace ID (OTel) |
| `F::SPAN_ID` | `span_id` | string | W3C span ID (OTel) |
| `F::CORRELATION_ID` | `correlation_id` | string | Request correlation ID across service boundaries |
| `F::CI_STEP` | `ci_step` | string | CI pipeline step / job name |
| `F::GIT_SHA` | `git_sha` | string | Git commit SHA |
| `F::FEATURES` | `features` | string | Active Cargo feature flags |
| `F::PEER_ADDR` | `peer_addr` | string | Remote peer address (`IP:port`) |
| `F::REQUEST_ID` | `request_id` | string | Inbound HTTP / gRPC request ID |

### Key consistency rules

- **`error`** — always use `%err` (Display) so the JSON value is a plain
  human-readable string, not a Rust Debug representation.
- **`duration_ms`** — always pass a `u64` so the JSON value is a number.
  Aggregators (Loki LogQL, Prometheus) can compute averages directly.
- **`reconcile_id`** — always pass a `u64`. `StructuredLog::reconcile_id`
  handles both string and numeric span values for forward compatibility.
- **`namespace`** — `StructuredLog` serialises `k8s_namespace` with
  `#[serde(rename = "namespace")]`, so the wire name always matches `F::NAMESPACE`.

---

## Common Usage Patterns

### 1. Span with standard context fields

```rust
use stellar_k8s::logging::fields as F;

let span = tracing::info_span!(
    "reconcile",
    { F::NODE }         = %node_name,
    { F::NAMESPACE }    = %namespace,
    { F::RECONCILE_ID } = reconcile_id,
    { F::COMPONENT }    = "controller",
);
let _enter = span.enter();
tracing::info!("Reconciliation started");
```

### 2. Using `LogContext` builder

`LogContext` collects all standard fields in one place and lets you pass them
into a span without repeating field names:

```rust
use stellar_k8s::logging::{LogContext, fields as F};

let ctx = LogContext::new()
    .node("my-validator")
    .namespace("stellar")
    .reconcile_id(42)
    .component("disk-scaler");

let span = tracing::info_span!(
    "disk_scale",
    { F::NODE }         = ctx.node.as_deref().unwrap_or(""),
    { F::NAMESPACE }    = ctx.namespace.as_deref().unwrap_or(""),
    { F::RECONCILE_ID } = ctx.reconcile_id.unwrap_or(0),
    { F::COMPONENT }    = ctx.component.as_deref().unwrap_or(""),
);
```

### 3. Logging errors (Display form)

```rust
use stellar_k8s::logging::fields as F;

if let Err(err) = do_something() {
    tracing::error!({ F::ERROR } = %err, "operation failed");
}
```

### 4. Recording latency

```rust
use stellar_k8s::logging::fields as F;
use std::time::Instant;

let start = Instant::now();
do_work();
let duration_ms = start.elapsed().as_millis() as u64;
tracing::info!({ F::DURATION_MS } = duration_ms, "work complete");
```