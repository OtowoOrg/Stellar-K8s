# Registry Pull-Path Vulnerability Gate

Epic: #1521 · Policy: `StellarRegistry.spec.admission.pullGate` · Module: `src/controller/registry_gate.rs`

The gate runs on the in-cluster registry's pull path rather than in an
admission webhook, so blocking still works when webhooks are degraded.

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarRegistry
spec:
  endpoint: registry.stellar.svc:5000
  scanning:
    enabled: true
    endpoint: http://trivy.security:4954
    maxCriticalCves: 0
  admission:
    pullGate: enforce   # off | audit | enforce
```

## Push

`PullGate::handle_push` scans the pushed digest (`<registry>/<repo>@<digest>`)
before the push is acknowledged, and stores a per-digest report. If the
scanner fails, no report is stored, so the digest stays unscanned. Reports can
be queried with `PullGate::report(digest)` and are served at
`<reportBaseUrl>/<digest>`.

## Pull

`PullGate::authorize_pull(digest)` decides whether a pull may proceed. Tag
pulls must be resolved to a digest first.

| Situation | `enforce` | `audit` | `off` |
|-----------|-----------|---------|-------|
| digest never scanned | deny | allow and record | allow |
| critical CVEs above `maxCriticalCves` | deny, with link to the report | allow and record | allow |
| clean | allow | allow | allow |

A denial is returned as an OCI distribution error, `403`:

```json
{"errors":[{"code":"DENIED","message":"sha256:… has 1 critical CVE(s) (max 0): CVE-2024-0001",
            "detail":{"digest":"sha256:…","reportUrl":"https://…/reports/sha256:…"}}]}
```

## Serving the gate (#2000)

The registry reconciler builds the gate from the `StellarRegistry` CR on every
reconcile and keeps it in a process-wide registry (`controller::registry_gate_registry`).
A policy change (`pullGate`, `maxCriticalCves`) takes effect on the next
reconcile and keeps the scan reports already stored. The operator's REST API
(behind the usual `api_reader` auth) serves the gate:

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/registry/:namespace/:name/pull-gate/:digest` | Authorize a manifest pull: `200` with the decision, or `403` with the OCI distribution error above |
| GET | `/api/v1/registry/scan-reports/:digest` | The scan report for a digest. This is the `reportUrl` in a denial |
| POST | `/api/v1/registry/:namespace/:name/push` | Admin only. Scan a pushed digest before the push is acknowledged and store its report. Body `{"repository":"app","digest":"sha256:…"}` |

```bash
curl -i https://operator:9090/api/v1/registry/stellar/main/pull-gate/sha256:<64 hex>
```

`reportUrl` is built from `REGISTRY_REPORT_BASE_URL` (default
`http://stellar-operator.stellar-system.svc:9090/api/v1/registry/scan-reports`),
so set it to an address clients can reach. Digests must look like
`sha256:<64 hex>`; resolve tags first. An unknown registry is `404`, a malformed
digest `400`.

A digest only has a report once it has been scanned, and `enforce` denies digests
without one. So point the registry's push notification (or a sidecar) at the
`push` endpoint: it scans through the scanner in `spec.scanning.endpoint` and
returns the report. If the scanner fails it returns `502` and stores nothing, so
the digest stays unscanned and is denied rather than waved through. With
`spec.scanning.enabled: false` it returns `503` for the same reason.

A registry front end or sidecar enforces the gate by asking the pull-gate
endpoint before serving a manifest and relaying a `403` unchanged. The endpoint
decides; it does not sit in the data path itself.

## Local serving

- `rewrite_to_local` rewrites every image reference to the pull-through
  layout of the local registry (`<local>/<upstream-host>/<path>`), so all
  in-cluster pulls are served locally.
- `CacheStats::hit_rate` tracks the steady-state cache hit rate.
