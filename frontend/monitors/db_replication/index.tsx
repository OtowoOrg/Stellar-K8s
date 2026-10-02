import React, { useCallback, useEffect, useRef, useState } from 'react';
import { ReplicaTree } from '../../components/replica_tree';
import { PrometheusClient, DEFAULT_QUERY_INTERVAL_MS } from './prometheus';
import { ReplicationSnapshot } from './types';

export interface DbReplicationMonitorProps {
  /** Prometheus API base URL, e.g. https://prom.internal/api/v1. */
  prometheusUrl: string;
  /** Optional bearer token. */
  prometheusToken?: string;
  /** Host label for the primary writer. */
  primaryHost: string;
  /** Polling interval in milliseconds. */
  intervalMs?: number;
  /** Override the lag threshold in seconds. */
  lagThresholdSeconds?: number;
  /** Optional fetch implementation override (tests). */
  fetchFn?: typeof fetch;
}

export const DbReplicationMonitor: React.FC<DbReplicationMonitorProps> = ({
  prometheusUrl,
  prometheusToken,
  primaryHost,
  intervalMs = DEFAULT_QUERY_INTERVAL_MS,
  lagThresholdSeconds,
  fetchFn,
}) => {
  const [snapshot, setSnapshot] = useState<ReplicationSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const clientRef = useRef<PrometheusClient | null>(null);

  if (!clientRef.current) {
    clientRef.current = new PrometheusClient({
      baseUrl: prometheusUrl,
      token: prometheusToken,
      fetchFn,
    });
  }

  const refresh = useCallback(async () => {
    const client = clientRef.current;
    if (!client) {
      return;
    }
    try {
      const next = await client.fetchSnapshot(primaryHost);
      setSnapshot(next);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [primaryHost]);

  useEffect(() => {
    let cancelled = false;
    const tick = async () => {
      if (cancelled) {
        return;
      }
      await refresh();
    };
    void tick();
    const id = setInterval(() => {
      void tick();
    }, intervalMs);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, [refresh, intervalMs]);

  return (
    <section className="db-replication-monitor" data-testid="db-replication-monitor">
      <header>
        <h1>Horizon DB Replication Monitor</h1>
        <button type="button" onClick={() => void refresh()}>
          Refresh now
        </button>
      </header>
      <ReplicaTree
        snapshot={snapshot}
        error={error}
        lagThresholdSeconds={lagThresholdSeconds}
      />
    </section>
  );
};

export default DbReplicationMonitor;
