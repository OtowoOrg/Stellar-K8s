import { useEffect, useMemo, useState } from 'react';
import {
  CartesianGrid,
  Legend,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts';

import { buildMockComparisonSeries, pollClusterMetrics } from './comparisonModel.js';

const DEFAULT_CONFIG = {
  clusterA: { name: 'Cluster A', url: 'https://example.invalid/cluster-a', timeoutMs: 3500 },
  clusterB: { name: 'Cluster B', url: 'https://example.invalid/cluster-b', timeoutMs: 3500 },
};

function formatTimestamp(value) {
  if (value == null || Number.isNaN(Number(value))) return 'n/a';
  const date = new Date(Number(value) * 1000);
  return Number.isFinite(date.getTime()) ? date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' }) : 'n/a';
}

function formatMetric(value) {
  if (value == null) return '—';
  return Number(value).toFixed(2);
}

export default function ComparisonDashboard() {
  const [series, setSeries] = useState([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(null);

  useEffect(() => {
    let cancelled = false;

    const tick = async () => {
      try {
        const next = await pollClusterMetrics(DEFAULT_CONFIG.clusterA, DEFAULT_CONFIG.clusterB);
        if (cancelled) return;
        setSeries(next.aligned.length ? next.aligned : [
          ...buildMockComparisonSeries('Cluster A', 0).map((point) => ({
            timestamp: point.timestamp / 1000,
            clusterA: point.tps,
            clusterB: null,
            tpsA: point.tps,
            tpsB: null,
            tpsDelta: null,
            ledgerCloseTimeA: point.ledgerCloseTime,
            ledgerCloseTimeB: null,
            ledgerCloseTimeDelta: null,
            memoryUsageA: point.memoryUsage,
            memoryUsageB: null,
            memoryUsageDelta: null,
          })),
          ...buildMockComparisonSeries('Cluster B', 3).map((point) => ({
            timestamp: point.timestamp / 1000,
            clusterA: null,
            clusterB: point.tps,
            tpsA: null,
            tpsB: point.tps,
            tpsDelta: null,
            ledgerCloseTimeA: null,
            ledgerCloseTimeB: point.ledgerCloseTime,
            ledgerCloseTimeDelta: null,
            memoryUsageA: null,
            memoryUsageB: point.memoryUsage,
            memoryUsageDelta: null,
          })),
        ]);
        setLoading(false);
        setError(next.error ?? null);
      } catch (caughtError) {
        if (!cancelled) {
          setError(caughtError.message);
          setLoading(false);
        }
      }
    };

    tick();
    const timer = window.setInterval(tick, 15000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  const summary = useMemo(() => {
    if (!series.length) return null;
    const latest = series[series.length - 1];
    const delta = latest.tpsDelta ?? 0;
    return {
      latest,
      delta,
      ledgerDelta: latest.ledgerCloseTimeDelta ?? 0,
      memoryDelta: latest.memoryUsageDelta ?? 0,
    };
  }, [series]);

  return (
    <section className="comparison-dashboard" aria-live="polite">
      <div className="comparison-header">
        <div>
          <span className="eyebrow">MULTI-CLUSTER METRICS</span>
          <h2>Comparison dashboard</h2>
        </div>
        <div className="comparison-badges">
          <span className="badge badge--green">Cluster A</span>
          <span className="badge badge--blue">Cluster B</span>
        </div>
      </div>

      {error ? (
        <div className="comparison-error" role="alert">
          <strong>Telemetry timeout</strong>
          <span>{error}</span>
        </div>
      ) : null}

      <div className="comparison-summary" aria-label="Cluster delta summary">
        <MetricTile label="TPS delta" value={`${formatMetric(summary?.delta ?? 0)} tx/s`} tone="green" />
        <MetricTile label="Ledger delta" value={`${formatMetric(summary?.ledgerDelta ?? 0)} ms`} tone="amber" />
        <MetricTile label="Memory delta" value={`${formatMetric(summary?.memoryDelta ?? 0)} MB`} tone="red" />
        <MetricTile label="Last sync" value={summary ? formatTimestamp(summary.latest.timestamp) : '—'} tone="blue" />
      </div>

      {loading && !series.length ? (
        <div className="comparison-loading">Synchronizing metric streams…</div>
      ) : null}

      <div className="comparison-grid">
        <ChartCard title="TPS comparison" description="Transactions per second by cluster">
          <ResponsiveContainer width="100%" height={260}>
            <LineChart data={series} margin={{ top: 6, right: 12, left: 0, bottom: 6 }}>
              <CartesianGrid strokeDasharray="3 3" stroke="#2a3948" />
              <XAxis dataKey="timestamp" tickFormatter={formatTimestamp} stroke="#8ea2b2" />
              <YAxis stroke="#8ea2b2" />
              <Tooltip formatter={(value) => [formatMetric(value), '']} labelFormatter={(value) => formatTimestamp(value)} />
              <Legend />
              <Line dataKey="tpsA" name="Cluster A" stroke="#39d98a" dot={false} isAnimationActive={false} />
              <Line dataKey="tpsB" name="Cluster B" stroke="#5aa9ff" dot={false} isAnimationActive={false} />
            </LineChart>
          </ResponsiveContainer>
        </ChartCard>

        <ChartCard title="Ledger close time" description="Lower is better for sync performance">
          <ResponsiveContainer width="100%" height={260}>
            <LineChart data={series} margin={{ top: 6, right: 12, left: 0, bottom: 6 }}>
              <CartesianGrid strokeDasharray="3 3" stroke="#2a3948" />
              <XAxis dataKey="timestamp" tickFormatter={formatTimestamp} stroke="#8ea2b2" />
              <YAxis stroke="#8ea2b2" />
              <Tooltip formatter={(value) => [`${formatMetric(value)} ms`, '']} labelFormatter={(value) => formatTimestamp(value)} />
              <Legend />
              <Line dataKey="ledgerCloseTimeA" name="Cluster A" stroke="#f5b942" dot={false} isAnimationActive={false} />
              <Line dataKey="ledgerCloseTimeB" name="Cluster B" stroke="#9d7cff" dot={false} isAnimationActive={false} />
            </LineChart>
          </ResponsiveContainer>
        </ChartCard>

        <ChartCard title="Memory usage" description="Resident or working-set usage by cluster">
          <ResponsiveContainer width="100%" height={260}>
            <LineChart data={series} margin={{ top: 6, right: 12, left: 0, bottom: 6 }}>
              <CartesianGrid strokeDasharray="3 3" stroke="#2a3948" />
              <XAxis dataKey="timestamp" tickFormatter={formatTimestamp} stroke="#8ea2b2" />
              <YAxis stroke="#8ea2b2" />
              <Tooltip formatter={(value) => [`${formatMetric(value)} MB`, '']} labelFormatter={(value) => formatTimestamp(value)} />
              <Legend />
              <Line dataKey="memoryUsageA" name="Cluster A" stroke="#f05d5e" dot={false} isAnimationActive={false} />
              <Line dataKey="memoryUsageB" name="Cluster B" stroke="#45d7d0" dot={false} isAnimationActive={false} />
            </LineChart>
          </ResponsiveContainer>
        </ChartCard>
      </div>
    </section>
  );
}

function MetricTile({ label, value, tone }) {
  return (
    <div className={`comparison-metric ${tone ? `comparison-metric--${tone}` : ''}`}>
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function ChartCard({ title, description, children }) {
  return (
    <article className="comparison-card">
      <div className="comparison-card__header">
        <h3>{title}</h3>
        <p>{description}</p>
      </div>
      {children}
    </article>
  );
}
