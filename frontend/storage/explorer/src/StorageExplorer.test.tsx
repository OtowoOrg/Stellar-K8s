import { render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { StorageExplorer } from './StorageExplorer';
import type { StorageMetricsApi } from './api/storageMetrics';
import * as storageMetricsModule from './api/storageMetrics';
import type { BenchmarkJob, PvcRef, StorageMetricSample } from './types';

const PVC: PvcRef = { namespace: 'stellar', name: 'validator-1-data', capacityBytes: 200 * 1024 ** 3 };

function sample(dayOffset: number, diskUsagePercent: number): StorageMetricSample {
  const base = Date.now() - 10 * 24 * 60 * 60 * 1000;
  return {
    timestamp: new Date(base + dayOffset * 24 * 60 * 60 * 1000).toISOString(),
    diskUsagePercent,
    readThroughputMBps: 50,
    writeThroughputMBps: 30,
    ioWaitMs: 4,
  };
}

function makeApi(samples: StorageMetricSample[]): StorageMetricsApi {
  return {
    listPvcs: async () => [PVC],
    getMetrics: async () => ({ pvc: PVC, samples }),
    triggerBenchmark: async () => ({ jobId: 'job-1', state: 'running', startedAt: new Date().toISOString() }) as BenchmarkJob,
    pollBenchmark: async () => ({
      jobId: 'job-1',
      state: 'succeeded',
      startedAt: new Date().toISOString(),
      finishedAt: new Date().toISOString(),
      result: {
        readIops: 100,
        writeIops: 80,
        readThroughputMBps: 10,
        writeThroughputMBps: 8,
        avgLatencyMs: 2,
      },
    }),
  };
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe('StorageExplorer saturation warning (#95 validation)', () => {
  it('displays a warning indicator when supplied metrics show impending volume exhaustion', async () => {
    // Steep, sustained growth — 40% -> 85% over 9 days — projects crossing
    // 100% well within the 14-day warning window.
    const steepGrowth = Array.from({ length: 10 }, (_, i) => sample(i, 40 + i * 5));
    vi.spyOn(storageMetricsModule, 'getStorageApi').mockReturnValue(makeApi(steepGrowth));

    render(<StorageExplorer />);

    const banner = await screen.findByTestId('saturation-warning');
    expect(banner).toHaveTextContent(/projected to reach capacity/i);

    // The Disk Usage chart itself should also carry the warning badge.
    expect(await screen.findByText('⚠ Saturation warning')).toBeInTheDocument();
  });

  it('does not display a warning indicator for a healthy, slow-growth volume', async () => {
    const slowGrowth = Array.from({ length: 10 }, (_, i) => sample(i, 30 + i * 0.2));
    vi.spyOn(storageMetricsModule, 'getStorageApi').mockReturnValue(makeApi(slowGrowth));

    render(<StorageExplorer />);

    await waitFor(() => expect(screen.queryByText('Loading metrics…')).not.toBeInTheDocument());

    expect(screen.queryByTestId('saturation-warning')).not.toBeInTheDocument();
    expect(screen.queryByText('⚠ Saturation warning')).not.toBeInTheDocument();
  });

  it('renders all three metric charts', async () => {
    vi.spyOn(storageMetricsModule, 'getStorageApi').mockReturnValue(
      makeApi(Array.from({ length: 5 }, (_, i) => sample(i, 30 + i))),
    );

    render(<StorageExplorer />);

    expect(await screen.findByRole('figure', { name: 'Disk Usage %' })).toBeInTheDocument();
    expect(screen.getByRole('figure', { name: 'Read / Write Throughput' })).toBeInTheDocument();
    expect(screen.getByRole('figure', { name: 'I/O Wait Latency' })).toBeInTheDocument();
  });
});

describe('StorageExplorer benchmark trigger', () => {
  it('runs a benchmark and displays its result', async () => {
    vi.spyOn(storageMetricsModule, 'getStorageApi').mockReturnValue(
      makeApi(Array.from({ length: 5 }, (_, i) => sample(i, 30 + i))),
    );

    const { default: userEvent } = await import('@testing-library/user-event');
    const user = userEvent.setup();

    render(<StorageExplorer />);

    const button = await screen.findByRole('button', { name: /run storage i\/o benchmark/i });
    await user.click(button);

    const panel = await screen.findByTestId('benchmark-panel');
    await waitFor(() => expect(panel).toHaveTextContent('succeeded'));
    expect(panel).toHaveTextContent('Read IOPS: 100');
  });
});
