import test from 'node:test';
import assert from 'node:assert/strict';

import {
  alignTimeSeries,
  calculateDelta,
  fetchWithTimeout,
  normalizePrometheusResponse,
  pollClusterMetrics,
} from './comparisonModel.js';

test('aligns lagging metric streams on a shared timestamp grid', () => {
  const aligned = alignTimeSeries(
    [
      { timestamp: 1000, ledgerCloseTime: 120, tps: 10, memoryUsage: 400 },
      { timestamp: 3000, ledgerCloseTime: 140, tps: 12, memoryUsage: 420 },
    ],
    [
      { timestamp: 2000, ledgerCloseTime: 130, tps: 11, memoryUsage: 410 },
      { timestamp: 4000, ledgerCloseTime: 150, tps: 13, memoryUsage: 420 },
    ],
  );

  assert.deepEqual(aligned.map((point) => point.timestamp), [1000, 2000, 3000, 4000]);
  assert.equal(aligned[1].clusterA, null);
  assert.equal(aligned[1].clusterB, 130);
  assert.equal(aligned[2].tpsDelta, 2);
  assert.equal(aligned[3].memoryUsageDelta, 10);
});

test('calculates delta as compare minus baseline and keeps missing values safe', () => {
  assert.equal(calculateDelta(100, 120), 20);
  assert.equal(calculateDelta(null, 30), 30);
  assert.equal(calculateDelta(70, null), null);
});

test('normalizes Prometheus payloads into comparable series', () => {
  const normalized = normalizePrometheusResponse({
    status: 'success',
    data: {
      resultType: 'matrix',
      result: [
        { metric: { cluster: 'a' }, values: [[1700, '100'], [2700, '110']] },
        { metric: { cluster: 'b' }, values: [[1700, '90'], [2700, '98']] },
      ],
    },
  });

  assert.deepEqual(normalized.map((row) => row.timestamp), [1700, 2700]);
  assert.equal(normalized[0].value, 100);
  assert.equal(normalized[1].value, 110);
});

test('fetchWithTimeout rejects when a request exceeds the timeout budget', async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () => new Promise((resolve) => {
    setTimeout(() => resolve({ ok: true, json: async () => ({ ok: true }) }), 200);
  });

  await assert.rejects(() => fetchWithTimeout('https://example.com', {}, 20), /timed out/i);
  globalThis.fetch = originalFetch;
});

test('pollClusterMetrics resolves each cluster independently and keeps partial results', async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (url) => {
    if (url.includes('cluster-a')) {
      return {
        ok: true,
        json: async () => ({
          status: 'success',
          data: {
            resultType: 'matrix',
            result: [{ values: [[1000, '50'], [2000, '51']] }],
          },
        }),
      };
    }

    return {
      ok: true,
      json: async () => ({
        status: 'success',
        data: {
          resultType: 'matrix',
          result: [{ values: [[1500, '60'], [2500, '62']] }],
        },
      }),
    };
  };

  const result = await pollClusterMetrics(
    { name: 'Cluster A', url: 'https://example.com/cluster-a' },
    { name: 'Cluster B', url: 'https://example.com/cluster-b' },
  );

  assert.equal(result.a.length, 2);
  assert.equal(result.b.length, 2);
  assert.equal(result.aligned[2].tpsDelta, 1);
  globalThis.fetch = originalFetch;
});
