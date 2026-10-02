/**
 * Prometheus query service — Issue #89
 *
 * Typed client for the Prometheus HTTP API v1 endpoints consumed by the
 * Visual PromQL Alerting Rule Builder.  Supports instant queries (used for
 * live rule testing) and metadata queries (used to validate that a PromQL
 * expression produces matching series before export).
 *
 * The module is a pure fetch wrapper with no framework dependency so it can
 * be used from React components, test harnesses, or Node.js scripts without
 * modification.
 */

// ─── Prometheus API response shapes ─────────────────────────────────────────

/** Single scalar or string result. */
export interface InstantScalar {
  resultType: 'scalar' | 'string';
  result: [number, string];   // [unix_ts, value]
}

/** Single time-series sample (instant query). */
export interface InstantVector {
  metric: Record<string, string>;
  value:  [number, string];   // [unix_ts, value_string]
}

/** Instant vector query result. */
export interface InstantVectorResult {
  resultType: 'vector';
  result:     InstantVector[];
}

/** Instant matrix result (range query). */
export interface MatrixResult {
  resultType: 'matrix';
  result: Array<{
    metric: Record<string, string>;
    values: Array<[number, string]>;
  }>;
}

export type PrometheusQueryResult =
  | InstantVectorResult
  | MatrixResult
  | InstantScalar;

/** Wrapper returned by the Prometheus HTTP API /query endpoint. */
export interface PrometheusApiResponse<D = PrometheusQueryResult> {
  status:    'success' | 'error';
  data:      D;
  errorType?: string;
  error?:    string;
  warnings?: string[];
}

/** Result returned by the live-test feature in the Alert Builder. */
export interface AlertTestResult {
  /** True when the PromQL expression is syntactically valid and matched series. */
  valid:           boolean;
  /** True when the condition is currently firing (at least one series matched). */
  currentlyFiring: boolean;
  /** Number of time-series evaluated by the instant query. */
  sampleCount:     number;
  /** Sample of matching metric labels for display (at most 5). */
  samples:         Array<Record<string, string>>;
  /** Human-readable status message. */
  message:         string;
  /** Raw Prometheus warnings if any. */
  warnings:        string[];
}

/** Metadata entry from /api/v1/label/__name__/values or /metadata. */
export interface MetricMetadata {
  metric: string;
  type:   'gauge' | 'counter' | 'histogram' | 'summary' | 'untyped';
  help:   string;
  unit:   string;
}

// ─── Error class ─────────────────────────────────────────────────────────────

export class PrometheusClientError extends Error {
  constructor(
    public readonly statusCode: number | null,
    message: string,
    public readonly prometheusError?: string,
  ) {
    super(message);
    this.name = 'PrometheusClientError';
  }
}

// ─── Client options ──────────────────────────────────────────────────────────

export interface PrometheusClientOptions {
  /**
   * Base URL of the Prometheus-compatible endpoint.
   * Default: '' (relative — proxied through the operator backend at /api/v1).
   */
  baseUrl?: string;
  /** Request timeout in milliseconds. Default: 10 000. */
  timeoutMs?: number;
  /** Extra HTTP headers (e.g. Authorization for remote-write-capable endpoints). */
  headers?: Record<string, string>;
}

// ─── Client ──────────────────────────────────────────────────────────────────

/**
 * PrometheusClient
 *
 * Thin, typed HTTP client for the Prometheus query API.  All public methods
 * throw `PrometheusClientError` on network or API-level failures so callers
 * can distinguish "bad PromQL syntax" from "Prometheus unreachable".
 *
 * Usage:
 * ```ts
 * const client = new PrometheusClient({ baseUrl: '/api/v1' });
 * const result = await client.testAlertExpr('stellar_fork_detector_consecutive_diverging_ledgers >= 3');
 * ```
 */
export class PrometheusClient {
  private readonly baseUrl:   string;
  private readonly timeoutMs: number;
  private readonly headers:   Record<string, string>;

  constructor(opts: PrometheusClientOptions = {}) {
    this.baseUrl   = (opts.baseUrl ?? '').replace(/\/$/, '');
    this.timeoutMs = opts.timeoutMs ?? 10_000;
    this.headers   = { 'Content-Type': 'application/x-www-form-urlencoded', ...opts.headers };
  }

  // ── Internal helpers ───────────────────────────────────────────────────────

  private async fetch<T>(
    path: string,
    params: Record<string, string>,
  ): Promise<PrometheusApiResponse<T>> {
    const controller = new AbortController();
    const timer      = setTimeout(() => controller.abort(), this.timeoutMs);

    const qs  = new URLSearchParams(params).toString();
    const url = `${this.baseUrl}${path}?${qs}`;

    let res: Response;
    try {
      res = await fetch(url, { headers: this.headers, signal: controller.signal });
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      const isAbort = controller.signal.aborted;
      throw new PrometheusClientError(
        null,
        isAbort ? `Prometheus request timed out after ${this.timeoutMs}ms` : `Network error: ${msg}`,
      );
    } finally {
      clearTimeout(timer);
    }

    let body: PrometheusApiResponse<T>;
    try {
      body = await res.json() as PrometheusApiResponse<T>;
    } catch {
      throw new PrometheusClientError(res.status, `Non-JSON response from Prometheus (HTTP ${res.status})`);
    }

    if (body.status === 'error') {
      throw new PrometheusClientError(
        res.status,
        `Prometheus query error: ${body.error ?? 'unknown'}`,
        body.error,
      );
    }

    if (!res.ok) {
      throw new PrometheusClientError(res.status, `Prometheus returned HTTP ${res.status}`);
    }

    return body;
  }

  // ── Public API ─────────────────────────────────────────────────────────────

  /**
   * Instant query — evaluates `expr` at the current time.
   * Returns the raw Prometheus response.
   */
  async query(expr: string): Promise<PrometheusApiResponse<PrometheusQueryResult>> {
    if (!expr.trim()) {
      throw new PrometheusClientError(null, 'Expression must not be empty');
    }
    return this.fetch<PrometheusQueryResult>('/query', { query: expr });
  }

  /**
   * Test an alert PromQL expression and return a structured result suitable
   * for display in the Alert Builder test panel.
   *
   * This is the method called by `AlertBuilder.runPrometheusTest()`.
   */
  async testAlertExpr(expr: string): Promise<AlertTestResult> {
    const resp = await this.query(expr);
    const data = resp.data as InstantVectorResult;

    const samples = (data.result ?? [])
      .slice(0, 5)
      .map((s) => s.metric);

    const sampleCount     = data.result?.length ?? 0;
    const currentlyFiring = sampleCount > 0;

    const message = currentlyFiring
      ? `Valid PromQL. Condition is CURRENTLY FIRING (${sampleCount} series matched).`
      : `Valid PromQL. Condition is not currently true (${sampleCount} series evaluated, none matched).`;

    return {
      valid: true,
      currentlyFiring,
      sampleCount,
      samples,
      message,
      warnings: resp.warnings ?? [],
    };
  }

  /**
   * Fetch metric metadata for a given metric name.
   * Useful for confirming a metric exists on the connected Prometheus instance.
   */
  async metricMetadata(metricName: string): Promise<MetricMetadata | null> {
    type MetadataResp = Record<string, Array<{ type: string; help: string; unit: string }>>;
    const resp = await this.fetch<MetadataResp>('/metadata', { metric: metricName });
    const entries = (resp.data as MetadataResp)[metricName];
    if (!entries || entries.length === 0) return null;
    const first = entries[0];
    return {
      metric: metricName,
      type:   first.type as MetricMetadata['type'],
      help:   first.help,
      unit:   first.unit,
    };
  }

  /**
   * Return all metric names known to this Prometheus instance whose name
   * contains `filter` (case-insensitive).  Useful for an autocomplete
   * dropdown in future builder iterations.
   */
  async listMetrics(filter = ''): Promise<string[]> {
    type LabelValues = { status: string; data: string[] };
    const resp = await this.fetch<string[]>('/label/__name__/values', {});
    const data = (resp as unknown as LabelValues).data ?? [];
    const lower = filter.toLowerCase();
    return lower ? data.filter((m) => m.toLowerCase().includes(lower)) : data;
  }
}

// ─── Default singleton ────────────────────────────────────────────────────────

/** Default client pointing at the operator backend proxy (/api/v1). */
export const prometheusClient = new PrometheusClient({ baseUrl: '/api/v1' });
export default prometheusClient;
