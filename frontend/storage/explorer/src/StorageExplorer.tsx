import { useCallback, useEffect, useMemo, useState } from 'react';
import { MetricsChart } from '../../../components/metrics_chart';
import { getStorageApi } from './api/storageMetrics';
import { describeSaturation, projectSaturation } from './lib/saturation';
import type { BenchmarkJob, MetricsRange, PvcRef, StorageMetricSample } from './types';

const RANGES: MetricsRange[] = ['24h', '7d', '14d', '30d'];
const WARNING_WINDOW_DAYS = 14;
const SATURATION_THRESHOLD_PERCENT = 100;
const BENCHMARK_POLL_INTERVAL_MS = 1000;

function formatBytes(bytes: number): string {
  const gib = bytes / (1024 * 1024 * 1024);
  return `${gib.toFixed(0)} GiB`;
}

export function StorageExplorer() {
  const api = useMemo(() => getStorageApi(), []);

  const [pvcs, setPvcs] = useState<PvcRef[]>([]);
  const [selected, setSelected] = useState<PvcRef | null>(null);
  const [range, setRange] = useState<MetricsRange>('14d');
  const [samples, setSamples] = useState<StorageMetricSample[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const [benchmark, setBenchmark] = useState<BenchmarkJob | null>(null);
  const [benchmarkError, setBenchmarkError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .listPvcs()
      .then((list) => {
        if (cancelled) return;
        setPvcs(list);
        setSelected((current) => current ?? list[0] ?? null);
      })
      .catch((e) => !cancelled && setError(String(e)));
    return () => {
      cancelled = true;
    };
  }, [api]);

  const loadMetrics = useCallback(() => {
    if (!selected) return;
    setLoading(true);
    setError(null);
    api
      .getMetrics(selected.namespace, selected.name, range)
      .then((res) => setSamples(res.samples))
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false));
  }, [api, selected, range]);

  useEffect(() => {
    loadMetrics();
  }, [loadMetrics]);

  const projection = useMemo(
    () => projectSaturation(samples, SATURATION_THRESHOLD_PERCENT, WARNING_WINDOW_DAYS),
    [samples],
  );

  // Poll a running benchmark job until it settles. Polls immediately on
  // start (rather than waiting a full interval for the first check) so
  // fast-settling jobs surface their result without an artificial delay.
  useEffect(() => {
    if (!benchmark || benchmark.state !== 'running') return;
    let cancelled = false;

    const poll = () => {
      api
        .pollBenchmark(benchmark.jobId)
        .then((job) => !cancelled && setBenchmark(job))
        .catch((e) => !cancelled && setBenchmarkError(String(e)));
    };

    poll();
    const id = window.setInterval(poll, BENCHMARK_POLL_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [api, benchmark]);

  const runBenchmark = useCallback(() => {
    if (!selected) return;
    setBenchmarkError(null);
    api
      .triggerBenchmark(selected.namespace, selected.name)
      .then((job) => setBenchmark(job))
      .catch((e) => setBenchmarkError(String(e)));
  }, [api, selected]);

  const diskUsageData = samples.map((s) => ({
    timestamp: s.timestamp,
    diskUsagePercent: s.diskUsagePercent,
  }));
  const throughputData = samples.map((s) => ({
    timestamp: s.timestamp,
    readThroughputMBps: s.readThroughputMBps,
    writeThroughputMBps: s.writeThroughputMBps,
  }));
  const latencyData = samples.map((s) => ({
    timestamp: s.timestamp,
    ioWaitMs: s.ioWaitMs,
  }));

  return (
    <div className="storage-explorer">
      <header className="storage-explorer__header">
        <h1>Persistent Volume Storage Explorer</h1>
        <div className="storage-explorer__controls">
          <label>
            PVC
            <select
              value={selected ? `${selected.namespace}/${selected.name}` : ''}
              onChange={(e) => {
                const found = pvcs.find((p) => `${p.namespace}/${p.name}` === e.target.value);
                setSelected(found ?? null);
              }}
            >
              {pvcs.map((p) => (
                <option key={`${p.namespace}/${p.name}`} value={`${p.namespace}/${p.name}`}>
                  {p.namespace}/{p.name} ({formatBytes(p.capacityBytes)})
                </option>
              ))}
            </select>
          </label>
          <label>
            Range
            <select value={range} onChange={(e) => setRange(e.target.value as MetricsRange)}>
              {RANGES.map((r) => (
                <option key={r} value={r}>
                  {r}
                </option>
              ))}
            </select>
          </label>
          <button
            type="button"
            onClick={runBenchmark}
            disabled={!selected || benchmark?.state === 'running'}
          >
            {benchmark?.state === 'running' ? 'Running benchmark…' : 'Run Storage I/O Benchmark'}
          </button>
        </div>
      </header>

      {error && (
        <p className="storage-explorer__error" role="alert">
          Failed to load metrics: {error}
        </p>
      )}

      {projection.isWarning && (
        <div className="storage-explorer__warning-banner" role="alert" data-testid="saturation-warning">
          ⚠ {describeSaturation(projection)}
        </div>
      )}

      {benchmarkError && (
        <p className="storage-explorer__error" role="alert">
          Benchmark failed to start: {benchmarkError}
        </p>
      )}
      {benchmark && (
        <div className="storage-explorer__benchmark" data-testid="benchmark-panel">
          <strong>Benchmark {benchmark.jobId}</strong>: {benchmark.state}
          {benchmark.state === 'succeeded' && benchmark.result && (
            <ul>
              <li>Read IOPS: {benchmark.result.readIops}</li>
              <li>Write IOPS: {benchmark.result.writeIops}</li>
              <li>Read throughput: {benchmark.result.readThroughputMBps} MB/s</li>
              <li>Write throughput: {benchmark.result.writeThroughputMBps} MB/s</li>
              <li>Avg latency: {benchmark.result.avgLatencyMs} ms</li>
            </ul>
          )}
          {benchmark.state === 'failed' && <p>{benchmark.error ?? 'Benchmark job failed.'}</p>}
        </div>
      )}

      {loading ? (
        <p>Loading metrics…</p>
      ) : (
        <div className="storage-explorer__charts">
          <MetricsChart
            title="Disk Usage %"
            data={diskUsageData}
            series={[{ key: 'diskUsagePercent', label: 'Disk usage', color: '#2563eb', unit: '%' }]}
            trendLine={projection.trendLine}
            trendLineLabel="Projected usage"
            thresholdValue={SATURATION_THRESHOLD_PERCENT}
            thresholdLabel="Capacity"
            yDomain={[0, 100]}
            warning={projection.isWarning}
          />
          <MetricsChart
            title="Read / Write Throughput"
            data={throughputData}
            series={[
              { key: 'readThroughputMBps', label: 'Read', color: '#16a34a', unit: ' MB/s' },
              { key: 'writeThroughputMBps', label: 'Write', color: '#9333ea', unit: ' MB/s' },
            ]}
          />
          <MetricsChart
            title="I/O Wait Latency"
            data={latencyData}
            series={[{ key: 'ioWaitMs', label: 'I/O wait', color: '#ea580c', unit: ' ms' }]}
          />
        </div>
      )}
    </div>
  );
}

export default StorageExplorer;
