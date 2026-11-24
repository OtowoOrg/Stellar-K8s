import type { BenchmarkJob, MetricsRange, PvcRef, StorageMetricsResponse } from '../types';
import { mockMetricsFor, mockPvcs } from '../mocks/fixtures';

/**
 * Client for the storage explorer's backend API.
 *
 * ## API contract (#95)
 *
 * This frontend is built against the following REST contract, following the
 * conventions already established by `src/rest_api/dashboard_handlers.rs`
 * and `src/rest_api/job_handlers.rs` (same `/api/v1/...` prefix, same
 * `{ items, total }` / job-record response shapes):
 *
 * | Method | Path                                                  | Description                          |
 * |--------|-------------------------------------------------------|---------------------------------------|
 * | GET    | `/api/v1/storage/pvcs`                                 | List PVCs available to the explorer   |
 * | GET    | `/api/v1/storage/pvcs/:namespace/:name/metrics?range=` | Historical metric samples for one PVC |
 * | POST   | `/api/v1/storage/pvcs/:namespace/:name/benchmark`      | Trigger a temporary I/O benchmark job |
 * | GET    | `/api/v1/storage/benchmarks/:jobId`                    | Poll a benchmark job's status/result  |
 *
 * **These backend routes do not exist yet** — this issue's own scope
 * (`frontend/storage/explorer/` and `frontend/components/metrics_chart.tsx`)
 * is frontend-only. Until the corresponding `src/rest_api` handlers are
 * added (mirroring `dashboard_handlers::get_node_metrics` for read paths and
 * `dashboard_handlers::execute_node_action` for the POST trigger), the app
 * runs against `mockStorageApi` so the explorer, its charts, and its
 * saturation warnings are fully demonstrable and testable today. Flip
 * `VITE_USE_MOCKS=false` once the backend endpoints above are implemented.
 */
export interface StorageMetricsApi {
  listPvcs(): Promise<PvcRef[]>;
  getMetrics(namespace: string, name: string, range: MetricsRange): Promise<StorageMetricsResponse>;
  triggerBenchmark(namespace: string, name: string): Promise<BenchmarkJob>;
  pollBenchmark(jobId: string): Promise<BenchmarkJob>;
}

async function fetchJson<T>(input: string, init?: RequestInit): Promise<T> {
  const res = await fetch(input, init);
  if (!res.ok) {
    const body = await res.text().catch(() => '');
    throw new Error(`Request to ${input} failed with ${res.status}: ${body}`);
  }
  return (await res.json()) as T;
}

/** Real backend implementation, per the contract documented above. */
export const restStorageApi: StorageMetricsApi = {
  listPvcs: () => fetchJson<PvcRef[]>('/api/v1/storage/pvcs'),

  getMetrics: (namespace, name, range) =>
    fetchJson<StorageMetricsResponse>(
      `/api/v1/storage/pvcs/${encodeURIComponent(namespace)}/${encodeURIComponent(name)}/metrics?range=${range}`,
    ),

  triggerBenchmark: (namespace, name) =>
    fetchJson<BenchmarkJob>(
      `/api/v1/storage/pvcs/${encodeURIComponent(namespace)}/${encodeURIComponent(name)}/benchmark`,
      { method: 'POST' },
    ),

  pollBenchmark: (jobId) => fetchJson<BenchmarkJob>(`/api/v1/storage/benchmarks/${encodeURIComponent(jobId)}`),
};

const BENCHMARK_DURATION_MS = 4000;
const mockJobs = new Map<string, { job: BenchmarkJob; settleAt: number }>();
let mockJobCounter = 0;

/** Deterministic-enough mock implementation used until the real backend routes ship. */
export const mockStorageApi: StorageMetricsApi = {
  listPvcs: async () => mockPvcs,

  getMetrics: async (namespace, name, range) => {
    const full = mockMetricsFor(namespace, name);
    const rangeDays: Record<MetricsRange, number> = { '24h': 1, '7d': 7, '14d': 14, '30d': 30 };
    const cutoff = Date.now() - rangeDays[range] * 24 * 60 * 60 * 1000;
    return {
      pvc: full.pvc,
      samples: full.samples.filter((s) => new Date(s.timestamp).getTime() >= cutoff),
    };
  },

  triggerBenchmark: async (namespace, name) => {
    void namespace;
    void name;
    mockJobCounter += 1;
    const jobId = `mock-benchmark-${mockJobCounter}`;
    const job: BenchmarkJob = { jobId, state: 'running', startedAt: new Date().toISOString() };
    mockJobs.set(jobId, { job, settleAt: Date.now() + BENCHMARK_DURATION_MS });
    return job;
  },

  pollBenchmark: async (jobId) => {
    const entry = mockJobs.get(jobId);
    if (!entry) throw new Error(`Unknown benchmark job: ${jobId}`);
    if (entry.job.state === 'running' && Date.now() >= entry.settleAt) {
      entry.job = {
        ...entry.job,
        state: 'succeeded',
        finishedAt: new Date().toISOString(),
        result: {
          readIops: 4200,
          writeIops: 2850,
          readThroughputMBps: 210.4,
          writeThroughputMBps: 132.7,
          avgLatencyMs: 1.8,
        },
      };
    }
    return entry.job;
  },
};

/**
 * Selects the mock API by default (no backend routes exist yet — see the
 * contract note above). Set `VITE_USE_MOCKS=false` in the environment once
 * `src/rest_api` implements the real `/api/v1/storage/*` routes.
 */
export function getStorageApi(): StorageMetricsApi {
  const useMocks = (import.meta.env.VITE_USE_MOCKS ?? 'true') !== 'false';
  return useMocks ? mockStorageApi : restStorageApi;
}
