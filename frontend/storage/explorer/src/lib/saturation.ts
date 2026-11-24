import type { SaturationProjection, StorageMetricSample } from '../types';

const MS_PER_DAY = 24 * 60 * 60 * 1000;

/**
 * Fits an ordinary-least-squares line to `diskUsagePercent` over time and
 * projects the date at which usage crosses `thresholdPercent`.
 *
 * Requires at least two samples with distinct timestamps to produce a
 * meaningful slope; with fewer, growth is treated as zero (no projection).
 *
 * @param samples          Historical samples, any order (sorted internally by timestamp).
 * @param thresholdPercent Usage percentage considered "saturated" (default 100).
 * @param warningDays      Projection window, in days, that counts as an actionable warning (default 14).
 * @param forecastDays     How many days past the last sample to extend the returned trend line (default 14).
 */
export function projectSaturation(
  samples: StorageMetricSample[],
  thresholdPercent = 100,
  warningDays = 14,
  forecastDays = 14,
): SaturationProjection {
  const sorted = [...samples].sort(
    (a, b) => new Date(a.timestamp).getTime() - new Date(b.timestamp).getTime(),
  );

  if (sorted.length < 2) {
    return {
      growthPercentPerDay: 0,
      projectedSaturationDate: null,
      daysUntilSaturation: null,
      isWarning: false,
      trendLine: sorted.map((s) => ({ timestamp: s.timestamp, value: s.diskUsagePercent })),
    };
  }

  const t0 = new Date(sorted[0].timestamp).getTime();
  // x in days since the first sample, y = usage percent.
  const points = sorted.map((s) => ({
    x: (new Date(s.timestamp).getTime() - t0) / MS_PER_DAY,
    y: s.diskUsagePercent,
  }));

  const n = points.length;
  const sumX = points.reduce((acc, p) => acc + p.x, 0);
  const sumY = points.reduce((acc, p) => acc + p.y, 0);
  const sumXY = points.reduce((acc, p) => acc + p.x * p.y, 0);
  const sumXX = points.reduce((acc, p) => acc + p.x * p.x, 0);

  const denominator = n * sumXX - sumX * sumX;
  // denominator is 0 only when every x is identical, which can't happen once
  // timestamps are distinct (guaranteed by the n >= 2 + sort-then-diff check
  // implicit in real samples); guard anyway for degenerate/duplicate-timestamp input.
  const slope = denominator === 0 ? 0 : (n * sumXY - sumX * sumY) / denominator;
  const intercept = (sumY - slope * sumX) / n;

  const lastX = points[n - 1].x;
  const lastSampleTime = new Date(sorted[n - 1].timestamp).getTime();

  let projectedSaturationDate: string | null = null;
  let daysUntilSaturation: number | null = null;

  if (slope > 0) {
    const xAtThreshold = (thresholdPercent - intercept) / slope;
    if (xAtThreshold > lastX) {
      daysUntilSaturation = xAtThreshold - lastX;
      projectedSaturationDate = new Date(t0 + xAtThreshold * MS_PER_DAY).toISOString();
    }
  }

  const isWarning =
    daysUntilSaturation !== null && daysUntilSaturation >= 0 && daysUntilSaturation <= warningDays;

  // Extend the trend line across the historical range plus the forecast window.
  const trendEndX = lastX + forecastDays;
  const trendLine: Array<{ timestamp: string; value: number }> = [];
  const step = (trendEndX - points[0].x) / Math.max(1, Math.round(trendEndX - points[0].x));
  for (let x = points[0].x; x <= trendEndX + 1e-9; x += step) {
    trendLine.push({
      timestamp: new Date(t0 + x * MS_PER_DAY).toISOString(),
      value: slope * x + intercept,
    });
  }
  // Guarantee the final forecast point is included even if the step loop overshoots/undershoots it.
  const last = trendLine[trendLine.length - 1];
  if (!last || Math.abs(new Date(last.timestamp).getTime() - (t0 + trendEndX * MS_PER_DAY)) > 1) {
    trendLine.push({
      timestamp: new Date(t0 + trendEndX * MS_PER_DAY).toISOString(),
      value: slope * trendEndX + intercept,
    });
  }

  return {
    growthPercentPerDay: slope,
    projectedSaturationDate,
    daysUntilSaturation,
    isWarning,
    trendLine,
  };
}

/** Human-readable summary used by the UI's warning banner. */
export function describeSaturation(projection: SaturationProjection): string {
  if (projection.daysUntilSaturation === null) {
    return projection.growthPercentPerDay <= 0
      ? 'Disk usage is flat or decreasing — no saturation projected.'
      : 'Disk usage is growing, but projected saturation is beyond the forecast window.';
  }
  const days = Math.max(0, Math.round(projection.daysUntilSaturation));
  const date = new Date(projection.projectedSaturationDate!).toLocaleDateString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  });
  return `At current growth (${projection.growthPercentPerDay.toFixed(2)}%/day), this volume is projected to reach capacity in ~${days} day${days === 1 ? '' : 's'} (around ${date}).`;
}
