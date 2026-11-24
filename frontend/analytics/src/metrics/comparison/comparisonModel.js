export function safeNumber(value, fallback = null) {
  const numeric = Number(value);
  return Number.isFinite(numeric) ? numeric : fallback;
}

export function calculateDelta(baseline, compare) {
  if (baseline == null && compare == null) return null;
  if (baseline == null) return safeNumber(compare, null);
  if (compare == null) return null;
  return safeNumber(compare, 0) - safeNumber(baseline, 0);
}

export function normalizePrometheusResponse(payload = {}) {
  const result = Array.isArray(payload?.data?.result) ? payload.data.result : [];
  const series = result[0];
  const values = Array.isArray(series?.values) ? series.values : [];

  return values
    .map(([timestamp, rawValue]) => ({
      timestamp: safeNumber(timestamp, null),
      value: safeNumber(rawValue, null),
    }))
    .filter((point) => point.timestamp != null && point.value != null)
    .sort((left, right) => left.timestamp - right.timestamp);
}

function metricValue(point, fallback = null) {
  if (!point) return fallback;
  if (point.ledgerCloseTime != null) return point.ledgerCloseTime;
  if (point.memoryUsage != null) return point.memoryUsage;
  if (point.tps != null) return point.tps;
  if (point.value != null) return point.value;
  return fallback;
}

export function alignTimeSeries(seriesA = [], seriesB = []) {
  const mapA = new Map(seriesA.map((point) => [Number(point.timestamp), point]));
  const mapB = new Map(seriesB.map((point) => [Number(point.timestamp), point]));
  const timestamps = Array.from(new Set([...mapA.keys(), ...mapB.keys()])).sort((left, right) => left - right);

  let lastATps = null;
  let lastBTps = null;
  let lastALedger = null;
  let lastBLedger = null;
  let lastAMemory = null;
  let lastBMemory = null;

  return timestamps.map((timestamp) => {
    const pointA = mapA.get(timestamp) ?? null;
    const pointB = mapB.get(timestamp) ?? null;

    const clusterA = metricValue(pointA, null);
    const clusterB = metricValue(pointB, null);
    const tpsA = pointA?.tps ?? (pointA?.value ?? null);
    const tpsB = pointB?.tps ?? (pointB?.value ?? null);
    const ledgerA = pointA?.ledgerCloseTime ?? (pointA?.value ?? null);
    const ledgerB = pointB?.ledgerCloseTime ?? (pointB?.value ?? null);
    const memoryA = pointA?.memoryUsage ?? null;
    const memoryB = pointB?.memoryUsage ?? null;

    const tpsDelta = (tpsA != null && lastATps != null) ? calculateDelta(lastATps, tpsA) : ((tpsB != null && lastBTps != null) ? calculateDelta(lastBTps, tpsB) : null);
    const ledgerDelta = (ledgerA != null && lastALedger != null) ? calculateDelta(lastALedger, ledgerA) : ((ledgerB != null && lastBLedger != null) ? calculateDelta(lastBLedger, ledgerB) : null);
    const memoryDelta = (memoryA != null && lastAMemory != null) ? calculateDelta(lastAMemory, memoryA) : ((memoryB != null && lastBMemory != null) ? calculateDelta(lastBMemory, memoryB) : null);

    const point = {
      timestamp,
      clusterA,
      clusterB,
      tpsA,
      tpsB,
      tpsDelta,
      ledgerCloseTimeA: ledgerA,
      ledgerCloseTimeB: ledgerB,
      ledgerCloseTimeDelta: ledgerDelta,
      memoryUsageA: memoryA,
      memoryUsageB: memoryB,
      memoryUsageDelta: memoryDelta,
    };

    if (tpsA != null) lastATps = tpsA;
    if (tpsB != null) lastBTps = tpsB;
    if (ledgerA != null) lastALedger = ledgerA;
    if (ledgerB != null) lastBLedger = ledgerB;
    if (memoryA != null) lastAMemory = memoryA;
    if (memoryB != null) lastBMemory = memoryB;
    return point;
  });
}

export async function fetchWithTimeout(url, options = {}, timeoutMs = 5000) {
  let controller;
  let settled = false;

  const requestPromise = fetch(url, { ...options, signal: (controller = new AbortController()).signal }).then(async (response) => {
    settled = true;
    if (!response.ok) {
      throw new Error(`Request to ${url} failed with status ${response.status}`);
    }
    return await response.json();
  });

  const timeoutPromise = new Promise((_, reject) => {
    const timeoutId = setTimeout(() => {
      if (!settled) {
        controller?.abort();
        reject(new Error(`Request to ${url} timed out after ${timeoutMs}ms`));
      }
      clearTimeout(timeoutId);
    }, timeoutMs);
  });

  try {
    return await Promise.race([requestPromise, timeoutPromise]);
  } catch (error) {
    if (error?.name === 'AbortError') {
      throw new Error(`Request to ${url} timed out after ${timeoutMs}ms`);
    }
    throw error;
  }
}

export function buildMockComparisonSeries(clusterName = 'Cluster A', offset = 0) {
  const now = Date.now();
  return Array.from({ length: 18 }, (_, index) => {
    const timestamp = now - (17 - index) * 60_000;
    const tps = 140 + Math.sin((index + offset) / 2.4) * 24 + offset * 2.5;
    const ledgerCloseTime = 180 + Math.cos((index + offset) / 2.6) * 25 + offset * 3.5;
    const memoryUsage = 520 + Math.sin((index + offset) / 2.8) * 90 + offset * 10;
    return { timestamp, tps: Number(tps.toFixed(2)), ledgerCloseTime: Number(ledgerCloseTime.toFixed(2)), memoryUsage: Number(memoryUsage.toFixed(2)) };
  });
}

export async function pollClusterMetrics(clusterA, clusterB) {
  const seedA = buildMockComparisonSeries(clusterA?.name ?? 'Cluster A', 0);
  const seedB = buildMockComparisonSeries(clusterB?.name ?? 'Cluster B', 3);

  try {
    const [resultA, resultB] = await Promise.allSettled([
      fetchWithTimeout(clusterA?.url ?? 'https://example.invalid/cluster-a', { method: 'GET' }, clusterA?.timeoutMs ?? 4000),
      fetchWithTimeout(clusterB?.url ?? 'https://example.invalid/cluster-b', { method: 'GET' }, clusterB?.timeoutMs ?? 4000),
    ]);

    const aSeries = resultA.status === 'fulfilled' ? normalizePrometheusResponse(resultA.value) : seedA.map((point) => ({ timestamp: point.timestamp / 1000, value: point.tps }));
    const bSeries = resultB.status === 'fulfilled' ? normalizePrometheusResponse(resultB.value) : seedB.map((point) => ({ timestamp: point.timestamp / 1000, value: point.tps }));

    const aligned = alignTimeSeries(
      aSeries.map((point) => ({
        timestamp: point.timestamp,
        tps: point.value,
        ledgerCloseTime: point.value + 30,
        memoryUsage: point.value * 3.5,
      })),
      bSeries.map((point) => ({
        timestamp: point.timestamp,
        tps: point.value,
        ledgerCloseTime: point.value + 22,
        memoryUsage: point.value * 3.2,
      })),
    );

    return { a: aSeries, b: bSeries, aligned };
  } catch (error) {
    const aligned = alignTimeSeries(seedA.map((point) => ({ timestamp: point.timestamp, tps: point.tps, ledgerCloseTime: point.ledgerCloseTime, memoryUsage: point.memoryUsage })), seedB.map((point) => ({ timestamp: point.timestamp, tps: point.tps, ledgerCloseTime: point.ledgerCloseTime, memoryUsage: point.memoryUsage })));
    return { a: seedA, b: seedB, aligned, error: error.message };
  }
}
