# Persistent Volume Storage Explorer

Frontend for issue #95 — visualizes PVC storage utilization and I/O
benchmark data for Stellar validator nodes managed by this operator.

## What it shows

- **Disk Usage %** over time, with a projected trend line and a 100%
  capacity threshold marker. A warning banner and a per-chart badge appear
  when the projected saturation date falls within 14 days.
- **Read/Write Throughput** (MB/s) and **I/O Wait Latency** (ms) time series.
- A **"Run Storage I/O Benchmark"** button that triggers a job and polls it
  to completion, displaying IOPS/throughput/latency results.

## Running locally

```bash
npm install
npm run dev      # http://localhost:5175
npm test         # vitest — includes the #95 saturation-warning validation tests
npm run build    # type-checks (tsc) then produces a production build
```

## Data source: mocked until the backend ships

This issue's scope (per its own "Impacted Files") is frontend-only:
`frontend/storage/explorer/` and `frontend/components/metrics_chart.tsx`.
No backend route currently serves historical PVC metrics or accepts a
benchmark-trigger request — `src/rest_api` only exposes point-in-time node
metrics (`dashboard_handlers::get_node_metrics`) and a generic node-action
POST endpoint (`dashboard_handlers::execute_node_action`), not per-PVC
history or a benchmark job.

By default (`VITE_USE_MOCKS` unset, or `true`) this app runs entirely
against deterministic, injected fixture data (`src/mocks/fixtures.ts`) —
including a "critical" volume whose growth rate is steep enough to trip the
saturation warning, which is what `src/StorageExplorer.test.tsx` exercises
for the issue's validation requirement ("Supply metric data indicating
impending volume exhaustion and verify the interface displays accurate
warning indicators").

`src/api/storageMetrics.ts` documents the REST contract (`GET
/api/v1/storage/pvcs`, `GET
/api/v1/storage/pvcs/:namespace/:name/metrics?range=`, `POST
/api/v1/storage/pvcs/:namespace/:name/benchmark`, `GET
/api/v1/storage/benchmarks/:jobId`) this app is built against, following
the same `/api/v1/...` conventions and response shapes as the existing
`src/rest_api/dashboard_handlers.rs` and `job_handlers.rs`. Once those
backend handlers exist, set `VITE_USE_MOCKS=false` to switch this app over
to them — the dev server already proxies `/api` to `localhost:9090` (see
`vite.config.ts`), matching `frontend/analytics`'s existing convention for
talking to the operator's REST API.

## Architecture notes

- `src/lib/saturation.ts` — pure ordinary-least-squares projection over
  historical `diskUsagePercent` samples; unit tested in
  `saturation.test.ts` independent of any UI or network concerns.
- `frontend/components/metrics_chart.tsx` (imported as `../../../components/metrics_chart`) — the shared, app-agnostic Recharts
  wrapper (multi-series lines, optional dashed trend-line overlay, optional
  threshold reference line). Deliberately has no dependency on this app so
  other `frontend/*` apps (e.g. `frontend/analytics`) can reuse it.
- Charts render smoothly over multi-day datasets by having the API layer
  (mock or real) return one sample per range-appropriate interval rather
  than raw high-frequency scrape data; `MetricsChart` itself does no
  downsampling.

## Screenshots

Not captured in this environment (no browser available in the sandbox this
PR was authored in). Run `npm run dev` and visit `http://localhost:5175` —
switching the PVC selector to `validator-1-data` reproduces the
near-saturation warning state shown by the tests.
