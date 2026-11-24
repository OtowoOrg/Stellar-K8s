import { describe, expect, it } from 'vitest';
import { describeSaturation, projectSaturation } from './saturation';
import type { StorageMetricSample } from '../types';

function sample(dayOffset: number, diskUsagePercent: number): StorageMetricSample {
  const base = new Date('2026-08-01T00:00:00.000Z').getTime();
  return {
    timestamp: new Date(base + dayOffset * 24 * 60 * 60 * 1000).toISOString(),
    diskUsagePercent,
    readThroughputMBps: 50,
    writeThroughputMBps: 30,
    ioWaitMs: 5,
  };
}

describe('projectSaturation', () => {
  it('returns no projection with fewer than two samples', () => {
    const result = projectSaturation([sample(0, 40)]);
    expect(result.daysUntilSaturation).toBeNull();
    expect(result.projectedSaturationDate).toBeNull();
    expect(result.isWarning).toBe(false);
  });

  it('projects imminent saturation from steep historical growth (#95 validation scenario)', () => {
    // Disk usage climbing ~5%/day for 10 days, currently at 85% — should
    // project crossing 100% in ~3 days and trip the warning indicator.
    const samples: StorageMetricSample[] = Array.from({ length: 10 }, (_, i) =>
      sample(i, 40 + i * 5),
    );

    const result = projectSaturation(samples, 100, 14);

    expect(result.growthPercentPerDay).toBeCloseTo(5, 1);
    expect(result.daysUntilSaturation).not.toBeNull();
    expect(result.daysUntilSaturation!).toBeGreaterThan(0);
    expect(result.daysUntilSaturation!).toBeLessThan(5);
    expect(result.projectedSaturationDate).not.toBeNull();
    expect(result.isWarning).toBe(true);
  });

  it('does not warn when saturation is projected beyond the warning window', () => {
    // Slow growth: 0.1%/day from 20% — centuries away from saturation.
    const samples: StorageMetricSample[] = Array.from({ length: 5 }, (_, i) =>
      sample(i, 20 + i * 0.1),
    );

    const result = projectSaturation(samples, 100, 14);

    expect(result.growthPercentPerDay).toBeCloseTo(0.1, 2);
    expect(result.isWarning).toBe(false);
  });

  it('reports no projection when usage is flat or decreasing', () => {
    const flat: StorageMetricSample[] = [sample(0, 50), sample(1, 50), sample(2, 50)];
    const decreasing: StorageMetricSample[] = [sample(0, 60), sample(1, 55), sample(2, 50)];

    expect(projectSaturation(flat).daysUntilSaturation).toBeNull();
    expect(projectSaturation(decreasing).daysUntilSaturation).toBeNull();
    expect(projectSaturation(decreasing).growthPercentPerDay).toBeLessThan(0);
  });

  it('is not thrown off by out-of-order input samples', () => {
    const inOrder = [sample(0, 40), sample(1, 45), sample(2, 50)];
    const shuffled = [inOrder[2], inOrder[0], inOrder[1]];

    const a = projectSaturation(inOrder);
    const b = projectSaturation(shuffled);

    expect(b.growthPercentPerDay).toBeCloseTo(a.growthPercentPerDay, 6);
    expect(b.daysUntilSaturation).toBeCloseTo(a.daysUntilSaturation ?? NaN, 6);
  });

  it('produces a trend line spanning the historical range plus the forecast window', () => {
    const samples = [sample(0, 40), sample(5, 60)];
    const result = projectSaturation(samples, 100, 14, 10);

    expect(result.trendLine.length).toBeGreaterThan(2);
    const firstTs = new Date(result.trendLine[0].timestamp).getTime();
    const lastTs = new Date(result.trendLine[result.trendLine.length - 1].timestamp).getTime();
    expect(firstTs).toBe(new Date(samples[0].timestamp).getTime());
    // last point should be ~15 days (5 historical + 10 forecast) after the first sample
    expect(lastTs - firstTs).toBeCloseTo(15 * 24 * 60 * 60 * 1000, -3);
  });

  it('respects a custom saturation threshold below 100%', () => {
    // Growth toward an 80%-full "danger zone" rather than absolute capacity.
    const samples = Array.from({ length: 6 }, (_, i) => sample(i, 50 + i * 5));
    const result = projectSaturation(samples, 80, 14);

    expect(result.daysUntilSaturation).not.toBeNull();
    expect(result.daysUntilSaturation!).toBeLessThan(3);
  });
});

describe('describeSaturation', () => {
  it('produces a human-readable warning message with day count and date', () => {
    const samples = Array.from({ length: 10 }, (_, i) => sample(i, 40 + i * 5));
    const projection = projectSaturation(samples);
    const message = describeSaturation(projection);

    expect(message).toMatch(/projected to reach capacity in ~\d+ day/);
    expect(message).toMatch(/5\.00%\/day/);
  });

  it('produces a reassuring message when usage is flat or decreasing', () => {
    const projection = projectSaturation([sample(0, 50), sample(1, 48), sample(2, 46)]);
    expect(describeSaturation(projection)).toMatch(/flat or decreasing/);
  });
});
