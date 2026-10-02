import {
  PrimaryMetrics,
  ReplicaMetrics,
  ReplicationSnapshot,
  ReplicationState,
} from './types';

export interface PrometheusQueryResult {
  metric: Record<string, string>;
  value?: [number, string];
}

export interface PrometheusQueryResponse {
  status: 'success' | 'error';
  data?: {
    resultType: 'matrix' | 'vector';
    result: PrometheusQueryResult[];
  };
  error?: string;
}

export interface PrometheusClientOptions {
  /** Base URL of the Prometetheus API, e.g. https://prom.internal/api/v1. */
  baseUrl: string;
  /** Optional bearer token for authenticated endpoints. */
  token?: string;
  /** Fetch implementation override (useful for tests). */
  fetchFn?: typeof fetch;
}

export const DEFAULT_QUERY_INTERVAL_MS = 5000;

export const QUERY_BYTE_LAG =
  'max(pg_stat_replication_bytes_behind) by (application_name, client_addr)';

export const QUERY_REPLAY_LATENCY =
  'max(pg_stat_replication_replay_latency_seconds) by (application_name, client_addr)';

export const QUERY_STATE =
  'max(pg_stat_replication_state == 1) by (application_name, client_addr, state)';

export const QUERY_PRIMARY_REPLICA_COUNT =
  'count(pg_stat_replication_state == 1)';

function normalizeState(raw: string | undefined): ReplicationState {
  if (!raw) {
    return 'unknown';
  }
  const value = raw.toLowerCase();
  if (value === 'streaming' || value === 'true' || value === '1') {
    return 'streaming';
  }
  if (value === 'catchup') {
    return 'catchup';
  }
  if (value === 'disconnected' || value === 'false' || value === '0') {
    return 'disconnected';
  }
  return 'unknown';
}

function replicaKey(metric: Record<string, string>): string {
  const name = metric.application_name || metric.client_addr || 'unknown';
  return `${name}@${metric.client_addr ?? 'unknown'}`;
}

export function parseReplicaState(
  results: PrometheusQueryResult[],
): Map<string, ReplicationState> {
  const out = new Map<string, ReplicationState>();
  for (const r of results) {
    out.set(replicaKey(r.metric), normalizeState(r.metric.state));
  }
  return out;
}

export function parseReplicationSnapshot(
  byteLagResults: PrometheusQueryResult[],
  replayLatencyResults: PrometheusQueryResult[],
  stateResults: PrometheusQueryResult[],
  primaryHost: string,
  now: number = Date.now(),
): ReplicationSnapshot {
  const stateByKey = parseReplicaState(stateResults);
  const replicas = new Map<string, ReplicaMetrics>();

  const ensureReplica = (metric: Record<string, string>) => {
    const key = replicaKey(metric);
    let replica = replicas.get(key);
    if (!replica) {
      replica = {
        name: metric.application_name ?? metric.client_addr ?? 'unknown',
        host: metric.client_addr ?? 'unknown',
        state: 'unknown',
        byteLag: 0,
        replayLatencySeconds: 0,
        lastUpdated: now,
      };
      replicas.set(key, replica);
    }
    return replica;
  };

  for (const r of byteLagResults) {
    const replica = ensureReplica(r.metric);
    replica.byteLag = r.value ? Number(r.value[1]) : 0;
  }

  for (const r of replayLatencyResults) {
    const replica = ensureReplica(r.metric);
    replica.replayLatencySeconds = r.value ? Number(r.value[1]) : 0;
  }

  for (const r of stateResults) {
    const replica = ensureReplica(r.metric);
    replica.state = stateByKey.get(replicaKey(r.metric)) ?? 'unknown';
  }

  const replicaList = Array.from(replicas.values()).sort((a, b) => a.name.localeCompare(b.name));

  const primary: PrimaryMetrics = {
    host: primaryHost,
    replicaCount: replicaList.length,
    lastUpdated: now,
  };

  return { primary, replicas: replicaList };
}

export class PrometheusClient {
  private readonly baseUrl: string;
  private readonly token?: string;
  private readonly fetchFn: typeof fetch;

  constructor(options: PrometheusClientOptions) {
    this.baseUrl = options.baseUrl.replace(/\/$/, '');
    this.token = options.token;
    this.fetchFn = options.fetchFn ?? fetch;
  }

  async query(query: string): PrometheusQueryResult[] {
    const url = new URL(`${this.baseUrl}/query`);
    url.searchParams.set('query', query);

    const headers: Record<string, string> = { Accept: 'application/json' };
    if (this.token) {
      headers['Authorization'] = `Bearer ${this.token}`;
    }

    const response = await this.fetchFn(url.toString(), { headers });
    if (!response.ok) {
      throw new Error(`Prometheus query failed: ${response.status} ${response.statusText}`);
    }

    const body = (wait response.json()) as PrometheusQueryResponse;
    if (body.status !== 'success' || !body.data) {
      throw new Error(body.error ?? 'Prometheus query returned an error');
    }
    return body.data.result;
  }

  async fetchSnapshot(primaryHost: string): Promise<ReplicationSnapshot> {
    const [byteLag, replayLatency, state] = await Promise.all([
      this.query(QUERY_BYTE_LAG),
      this.query(QUERY_REPLAY_LATENCY),
      this.query(QUERY_STATE),
    ]);
    return parseReplicationSnapshot(byteLag, replayLatency, state, primaryHost);
  }
}
