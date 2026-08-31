# Stellar-K8s Error Codes

This document provides details on all error variants encountered in the Stellar-K8s operator, their causes, structured fields, and resolution steps.

| Error Code | Name | Description | Resolution Steps |
| --- | --- | --- | --- |
| **SK8S-001** | `KubeError(kube::Error)` | Kubernetes API error returned from `kube-rs`. | Check the Kubernetes cluster status and accessibility of the API server. Review RBAC permissions for the operator. |
| **SK8S-002** | `SerializationError(serde_json::Error)` | JSON serialization/deserialization failed. | Ensure custom resource definitions (CRDs) match operator schema and specs contain valid JSON/YAML syntax. |
| **SK8S-003** | `FinalizerError(String)` | A finalizer failed to execute during resource cleanup. | Examine operator deployment logs to identify the failing cleanup task (e.g., non-deletable associated resources). |
| **SK8S-004** | `ConfigError(String)` | Operator or resource configuration is invalid. | Review configuration for typos and validate fields against supported schema constraints and environment settings. |
| **SK8S-005** | `ValidationError(String)` | Node specification validation failed. | Inspect `StellarNode` CR fields against validation rules. Verify parameter compatibility and resource bounds. |
| **SK8S-006** | `NotFound { kind, name, namespace }` | The requested Kubernetes resource (`kind/name` in `namespace`) was not found. | Ensure the target resource exists in the specified namespace and the resource name is spelled correctly. |
| **SK8S-007** | `InvalidNodeType(String)` | An invalid or unrecognized node type was requested. | Validate `nodeType` in the spec. Allowed types must be recognized by this operator version (e.g., Validator, Horizon, SorobanRpc). |
| **SK8S-008** | `MissingRequiredField { field, node_type }` | Mandatory `field` for the specified `node_type` is missing. | Complete the node spec by providing all required parameters for the specified `nodeType` (e.g., `seedSecretRef` for Validators). |
| **SK8S-009** | `ArchiveHealthCheckError(String)` | History archive health check failed. | Verify history archive URL reachability, network connectivity, and storage endpoint status. |
| **SK8S-010** | `HttpError(reqwest::Error)` | HTTP request error during external/internal API calls. | Check network connectivity, DNS resolution, and NetworkPolicies for outbound traffic. |
| **SK8S-011** | `RemediationError(String)` | Automated remediation task failed during execution. | Inspect operator logs for the failed remediation sequence. Check RBAC permissions and target pod/node stability. |
| **SK8S-012** | `PluginError(String)` | Error during WASM admission plugin execution. | Verify WASM plugin compilation integrity, runtime configuration, and dependency availability. |
| **SK8S-013** | `WebhookError(String)` | Admission webhook server operational error. | Verify webhook TLS certificates, service endpoint routing, and pod readiness. |
| **SK8S-014** | `NetworkError(String)` | General network connectivity failure encountered. | Check cluster CNI plugin health, pod routing, and inter-node network stability. |
| **SK8S-015** | `CertificateError(rcgen::Error)` | Generating or parsing TLS certificate failed. | Inspect certificate configuration and CA key pairs. Verify cert-manager integration if applicable. |
| **SK8S-016** | `IoError(std::io::Error)` | File system input/output failure. | Check filesystem permissions, mount availability, and disk capacity for local caching paths. |
| **SK8S-017** | `MaintenanceError(String)` | Database maintenance or pruning task failed. | Check PostgreSQL status, disk space, and process locks on node database tables. |
| **SK8S-018** | `SqlxError(sqlx::Error)` | SQL database interaction error from SQLx driver. | Verify database connectivity, active connections, credentials, and schema migration state. |
| **SK8S-019** | `KubeconfigError(kube::config::KubeconfigError)` | Failed to load or parse local Kubeconfig file. | Verify `KUBECONFIG` environment variable path, file existence, and file permissions. |
| **SK8S-020** | `ZipError(zip::result::ZipError)` | Failure during compression or extraction of snapshots. | Verify snapshot archive integrity and ensure adequate disk space is available for extraction. |
| **SK8S-021** | `NetworkSafetyViolation(NetworkSafetyViolation)` | Cross-network safety policy violation (e.g. Mainnet and Testnet in same namespace). | Deploy nodes from different network types into separate Kubernetes namespaces to prevent ledger contamination. |
| **SK8S-022** | `InternalError(String)` | Unexpected internal state error. | Check operator logs for `[SK8S-022]` details and report unrecoverable internal errors. |
| **SK8S-023** | `PhaseTransitionError(String)` | The reconciler attempted a reconcile phase transition that the state machine in `src/controller/phases.rs` does not permit. | Always an operator bug, never a bad input. The message names the source phase, the target phase, and the legal moves; see [Reconciler Phases](reconciler-phases.md). |

## Error Helper Functions & Behavior Semantics

The operator provides built-in helper functions and methods for structured diagnostic formatting, error construction, retry management, and status reporting:

### Diagnostic Formatting: `diagnostic(step, detail)`
Formats a user-facing diagnostic string by pairing an explicit pipeline execution step with error details:
`diagnostic("load kubeconfig", "file not found")` → `"[load kubeconfig] file not found"`

### Step-Aware Constructors
- `Error::config_step(step, detail)` — Constructs `Error::ConfigError` formatted via `diagnostic(step, detail)`.
- `Error::internal_step(step, detail)` — Constructs `Error::InternalError` formatted via `diagnostic(step, detail)`.
- `Error::validation_step(step, detail)` — Constructs `Error::ValidationError` formatted via `diagnostic(step, detail)`.

### Retry Semantics: `Error::is_retriable()`
Determines whether an error variant should trigger an automatic reconciliation retry. The following variants are classified as retriable:
- `Error::KubeError` — Transient cluster API server communication issues.
- `Error::FinalizerError` — Temporary resource cleanup impediments.
- `Error::RemediationError` — Transient auto-remediation failures.

Non-retriable variants (such as `ConfigError` or `ValidationError`) require manual user intervention or spec modifications.

### Status Reporting: `Error::status_message()`
Delegates directly to the `Display` implementation (`self.to_string()`), serving as a single source of truth for updating `StellarNode` custom resource status conditions.

## HTTP status mapping (REST API and API gateway, issue #1393)

Every `Error` variant maps to an HTTP status via `Error::status_code()` and to
a stable, machine-readable code via `Error::api_error_code()`
(`src/error.rs`). REST handlers (`src/rest_api/handlers.rs`) and the gateway
proxy (`src/api_gateway/server.rs`) both render failures through the same
`ErrorResponse` JSON envelope (originally introduced for `rest_api` in issue
#1282, now defined once in `crate::error` and re-exported from
`rest_api::dto` so neither module has its own copy):

```json
{
  "error": "err_not_found",
  "error_code": "ERR_NOT_FOUND",
  "message": "Node stellar/my-validator not found",
  "correlation_id": "6f3a9c2e-...-b1d4",
  "degraded": false,
  "timestamp": "2026-08-28T12:34:56Z"
}
```

| `Error` variant(s) | HTTP status | `ApiErrorCode` |
| --- | --- | --- |
| `NotFound` | 404 Not Found | `ERR_NOT_FOUND` |
| `ValidationError`, `InvalidNodeType`, `MissingRequiredField`, `SerializationError` | 400 Bad Request | `ERR_BAD_REQUEST` |
| `PhaseTransitionError` | 409 Conflict | `ERR_RECONCILE_STALLED` |
| `KubeError`, `HttpError`, `NetworkError`, `KubeconfigError` | 503 Service Unavailable | `ERR_SERVICE_UNAVAILABLE` |
| Everything else (`ConfigError`, `CertificateError`, `IoError`, `SqlxError`, `ZipError`, `InternalError`, ...) | 500 Internal Server Error | `ERR_INTERNAL_SERVER_ERROR` |

`api_gateway` reports a few failure modes that have no equivalent `Error`
variant, using additional `ApiErrorCode` members defined alongside the rest:

| Gateway condition | HTTP status | `ApiErrorCode` |
| --- | --- | --- |
| Missing/invalid API key | 401 Unauthorized | `ERR_UNAUTHORIZED` |
| Rate limit / quota exceeded | 429 Too Many Requests | `ERR_RATE_LIMITED` |
| No route matches the request | 404 Not Found | `ERR_NOT_FOUND` |
| Requested API version has been sunset | 410 Gone | `ERR_GONE` |
| Malformed request body / protocol transform failure | 400 Bad Request | `ERR_BAD_REQUEST` |
| Upstream request failed outright (connection/timeout) | 502 Bad Gateway | `ERR_SERVICE_UNAVAILABLE` |

### Graceful degradation

`ErrorResponse::degraded()` sets `"degraded": true` and attaches a `details`
payload for responses that carry a genuine (if incomplete) result rather than
a hard failure. The gateway uses it when an upstream call succeeds but the
response can't be reshaped into the client's expected protocol: instead of
discarding a response it actually received, it returns the raw upstream body
under `details.rawUpstreamBody` with `ERR_PARTIAL_DEGRADATION`. When there is
truly nothing to fall back on — e.g. the upstream connection itself fails, and
the gateway keeps no response cache — the failure is reported as a normal
(non-degraded) `ERR_SERVICE_UNAVAILABLE` / 502 instead of being dressed up as
a partial success.

## Correlation IDs

`telemetry::http_trace_middleware` (`src/telemetry.rs`), the same middleware
that starts the per-request tracing span, also resolves a per-request
correlation ID and threads it through the whole request/response lifecycle:

1. **Inbound**: reuses the caller's `X-Correlation-Id` request header when
   present and non-blank; otherwise mints a new UUID v4
   (`telemetry::resolve_correlation_id`).
2. **Logs**: records it as a `correlation_id` field on the `http.request`
   tracing span, which `logging::build_structured_log` (`src/logging/mod.rs`)
   already knew how to surface on every structured JSON log line for that
   request — no per-callsite logging changes were needed.
3. **Handlers**: stores it in the request's `axum` extensions as
   `telemetry::CorrelationId`, so REST handlers (via `Extension<CorrelationId>`)
   and the gateway's `handle_request` (via `req.extensions()`) can attach it
   to `ErrorResponse.correlation_id` without re-parsing headers.
4. **Outbound**: echoes it back on every response — success or error — as an
   `X-Correlation-Id` header, so a caller can always correlate its own
   request with operator logs and traces even without opting into the OTel
   `trace_id`/`span_id` fields that are logged alongside it.

This middleware is applied once, at the outermost `.layer()` of both the
`rest_api` protected router (`src/rest_api/server.rs`) and the `api_gateway`
router (`src/api_gateway/server.rs`), so both surfaces get identical
behavior.

## General Troubleshooting
When encountering these errors, the primary source of detailed insight will be the operator logs. You can fetch them with:
```bash
kubectl logs -n stellar-system deploy/stellar-operator
```
Look for the `[SK8S-XXX]` prefix in the logging output for rapid filtering.

---

*Last verified: 2026-07-29 (pipeline log redaction + rustfmt CI wave).*
