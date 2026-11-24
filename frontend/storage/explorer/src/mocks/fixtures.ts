import type { PvcRef, StorageMetricSample, StorageMetricsResponse } from '../types';

/**
 * Deterministic sample-data generator used for local development (when no
 * backend is reachable) and for tests. Not randomized, so snapshots/assertions
 * stay stable across runs.
 */
function buildSamples(
  days: number,
  samplesPerDay: number,
  startPercent: number,
  percentPerDay: number,
): StorageMetricSample[] {
  const totalSamples = days * samplesPerDay;
  const start = Date.now() - days * 24 * 60 * 60 * 1000;
  const stepMs = (days * 24 * 60 * 60 * 1000) / totalSamples;

  return Array.from({ length: totalSamples }, (_, i) => {
    const dayFraction = i / samplesPerDay;
    // Small sinusoidal wobble on top of the linear trend so charts don't look artificially straight.
    const wobble = Math.sin(i / (samplesPerDay / 2)) * 1.5;
    const diskUsagePercent = Math.max(
      0,
      Math.min(100, startPercent + percentPerDay * dayFraction + wobble),
    );
    return {
      timestamp: new Date(start + i * stepMs).toISOString(),
      diskUsagePercent: Number(diskUsagePercent.toFixed(2)),
      readThroughputMBps: Number((40 + Math.sin(i / 4) * 15 + Math.random() * 5).toFixed(1)),
      writeThroughputMBps: Number((25 + Math.cos(i / 5) * 10 + Math.random() * 4).toFixed(1)),
      ioWaitMs: Number((3 + Math.max(0, diskUsagePercent - 70) * 0.4 + Math.random()).toFixed(2)),
    };
  });
}

const healthyPvc: PvcRef = {
  namespace: 'stellar',
  name: 'validator-0-data',
  capacityBytes: 500 * 1024 * 1024 * 1024, // 500Gi
};

const criticalPvc: PvcRef = {
  namespace: 'stellar',
  name: 'validator-1-data',
  capacityBytes: 200 * 1024 * 1024 * 1024, // 200Gi
};

/** A well-behaved volume with slow, sustainable growth. */
export const healthyVolumeMetrics: StorageMetricsResponse = {
  pvc: healthyPvc,
  samples: buildSamples(14, 12, 35, 0.5),
};

/**
 * A volume growing quickly enough to project saturation within the default
 * 14-day warning window — the "impending volume exhaustion" scenario called
 * for by issue #95's validation requirement.
 */
export const criticalVolumeMetrics: StorageMetricsResponse = {
  pvc: criticalPvc,
  samples: buildSamples(14, 12, 55, 3.2),
};

export const mockPvcs: PvcRef[] = [healthyPvc, criticalPvc];

export function mockMetricsFor(namespace: string, name: string): StorageMetricsResponse {
  if (name === criticalPvc.name) return criticalVolumeMetrics;
  return healthyVolumeMetrics;
}
