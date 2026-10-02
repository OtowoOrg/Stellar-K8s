/**
 * PrometheusClient unit tests — Issue #89
 *
 * Tests cover:
 *  1. Successful instant query returns typed result
 *  2. testAlertExpr: currently-firing path
 *  3. testAlertExpr: not-firing path (0 series)
 *  4. Prometheus API-level error surfaces as PrometheusClientError
 *  5. Network timeout surfaces as PrometheusClientError
 *  6. Non-JSON response surfaces as PrometheusClientError
 *  7. Empty expression rejected before fetch
 *  8. metricMetadata returns null for unknown metric
 *  9. metricMetadata returns typed entry for known metric
 */

import { PrometheusClient, PrometheusClientError } from './prometheus.js';

const mockFetch = jest.fn();
global.fetch = mockFetch;

function makeVectorResponse(series: Array<Record<string, string>>, value = '1') {
  return {
    status: 'success',
    data: {
      resultType: 'vector',
      result: series.map((metric) => ({ metric, value: [Date.now() / 1000, value] })),
    },
  };
}

const client = new PrometheusClient({ baseUrl: '/api/v1', timeoutMs: 5000 });

beforeEach(() => mockFetch.mockReset());

test('1 — successful instant query returns vector result', async () => {
  mockFetch.mockResolvedValueOnce({
    ok: true,
    json: async () => makeVectorResponse([{ job: 'stellar', instance: 'node-0' }]),
  });

  const resp = await client.query('stellar_fork_detector_consecutive_diverging_ledgers >= 3');
  expect(resp.status).toBe('success');
  expect((resp.data as { resultType: string }).resultType).toBe('vector');
});

test('2 — testAlertExpr: firing when series returned', async () => {
  mockFetch.mockResolvedValueOnce({
    ok: true,
    json: async () => makeVectorResponse([{ pod: 'validator-0' }, { pod: 'validator-1' }]),
  });

  const result = await client.testAlertExpr('stellar_fork_detector_sync_confidence < 500');
  expect(result.valid).toBe(true);
  expect(result.currentlyFiring).toBe(true);
  expect(result.sampleCount).toBe(2);
  expect(result.message).toMatch(/CURRENTLY FIRING/i);
});

test('3 — testAlertExpr: not firing when no series returned', async () => {
  mockFetch.mockResolvedValueOnce({
    ok: true,
    json: async () => makeVectorResponse([]),
  });

  const result = await client.testAlertExpr('stellar_fork_detector_responding_anchors == 0');
  expect(result.valid).toBe(true);
  expect(result.currentlyFiring).toBe(false);
  expect(result.sampleCount).toBe(0);
  expect(result.message).toMatch(/not currently true/i);
});

test('4 — Prometheus API error surfaces as PrometheusClientError', async () => {
  mockFetch.mockResolvedValueOnce({
    ok: false,
    status: 422,
    json: async () => ({
      status: 'error',
      errorType: 'bad_data',
      error: 'parse error at 1:5: unexpected identifier',
    }),
  });

  await expect(client.query('bad PromQL expr')).rejects.toMatchObject({
    name: 'PrometheusClientError',
  });
});

test('5 — timeout surfaces as PrometheusClientError', async () => {
  mockFetch.mockRejectedValueOnce(Object.assign(new Error('The operation was aborted'), { name: 'AbortError' }));

  const fastClient = new PrometheusClient({ baseUrl: '/api/v1', timeoutMs: 1 });
  await expect(fastClient.query('up')).rejects.toMatchObject({
    name: 'PrometheusClientError',
  });
});

test('6 — non-JSON response surfaces as PrometheusClientError', async () => {
  mockFetch.mockResolvedValueOnce({
    ok: true,
    status: 200,
    json: async () => { throw new SyntaxError('not JSON'); },
  });

  await expect(client.query('up')).rejects.toMatchObject({ name: 'PrometheusClientError' });
});

test('7 — empty expression rejected before fetch', async () => {
  await expect(client.query('   ')).rejects.toMatchObject({
    name: 'PrometheusClientError',
    message: expect.stringContaining('must not be empty'),
  });
  expect(mockFetch).not.toHaveBeenCalled();
});

test('8 — metricMetadata returns null for unknown metric', async () => {
  mockFetch.mockResolvedValueOnce({
    ok: true,
    json: async () => ({ status: 'success', data: {} }),
  });

  const meta = await client.metricMetadata('no_such_metric_xyz');
  expect(meta).toBeNull();
});

test('9 — metricMetadata returns typed entry for known metric', async () => {
  mockFetch.mockResolvedValueOnce({
    ok: true,
    json: async () => ({
      status: 'success',
      data: {
        stellar_fork_detector_sync_confidence: [
          { type: 'gauge', help: 'Sync confidence in permille', unit: 'permille' },
        ],
      },
    }),
  });

  const meta = await client.metricMetadata('stellar_fork_detector_sync_confidence');
  expect(meta).toEqual({
    metric: 'stellar_fork_detector_sync_confidence',
    type: 'gauge',
    help: 'Sync confidence in permille',
    unit: 'permille',
  });
});
