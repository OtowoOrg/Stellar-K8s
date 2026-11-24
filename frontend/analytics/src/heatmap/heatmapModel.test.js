import test from 'node:test';
import assert from 'node:assert/strict';
import {
  classifySaturation,
  createHeatmapState,
  ingestSamples,
  markError,
  materializeHeatmap,
  normalizeRatio,
  parsePrometheusResponse,
  parsePrometheusText,
  saturationColor,
} from './heatmapModel.js';

function vector(result) {
  return { status: 'success', data: { resultType: 'vector', result } };
}

test('normalizes ratios from fractions, percentages, and junk', () => {
  assert.equal(normalizeRatio(0.42), 0.42);
  assert.equal(normalizeRatio(73), 0.73);
  assert.equal(normalizeRatio(250), 1);
  assert.equal(normalizeRatio(-3), 0);
  assert.equal(normalizeRatio('NaN'), 0);
});

test('classifies saturation against the threshold bands', () => {
  assert.equal(classifySaturation(0), 'idle');
  assert.equal(classifySaturation(0.2), 'cool');
  assert.equal(classifySaturation(0.6), 'warm');
  assert.equal(classifySaturation(0.8), 'hot');
  assert.equal(classifySaturation(0.95), 'critical');
});

test('saturation color ramps monotonically from cool blue to hot red', () => {
  const idle = saturationColor(0);
  const hot = saturationColor(1);
  assert.match(idle, /^#[0-9a-f]{6}$/);
  assert.equal(idle, '#2563eb');
  assert.equal(hot, '#dc2626');
  const redAt = (ratio) => parseInt(saturationColor(ratio).slice(1, 3), 16);
  assert.ok(redAt(0.9) > redAt(0.3), 'red channel should grow with saturation');
});

test('parses the Prometheus HTTP vector response into samples', () => {
  const samples = parsePrometheusResponse(
    vector([
      { metric: { node: 'worker-000', zone: 'az-a', pod: 'p1', resource: 'cpu' }, value: [1, '0.8'] },
      { metric: { node: 'worker-000', zone: 'az-a', pod: 'p1', resource: 'memory' }, value: [1, '0.4'] },
    ]),
  );
  assert.equal(samples.length, 2);
  assert.deepEqual(samples[0], { node: 'worker-000', zone: 'az-a', pod: 'p1', resource: 'cpu', value: 0.8 });
});

test('parses the Prometheus text exposition format and ignores other metrics', () => {
  const text = [
    '# HELP stellar_operator_resource_usage help',
    '# TYPE stellar_operator_resource_usage gauge',
    'stellar_operator_resource_usage{node="worker-001",zone="az-b",pod="p2",resource="cpu"} 0.55',
    'some_other_metric{node="worker-001"} 12',
  ].join('\n');
  const samples = parsePrometheusText(text);
  assert.equal(samples.length, 1);
  assert.equal(samples[0].node, 'worker-001');
  assert.equal(samples[0].value, 0.55);
});

test('aggregates pods per node and reports the busiest resource', () => {
  const state = createHeatmapState();
  ingestSamples(
    state,
    parsePrometheusResponse(
      vector([
        { metric: { node: 'w1', zone: 'az-a', pod: 'a', resource: 'cpu' }, value: [1, '0.30'] },
        { metric: { node: 'w1', zone: 'az-a', pod: 'b', resource: 'cpu' }, value: [1, '0.90'] },
        { metric: { node: 'w1', zone: 'az-a', pod: 'b', resource: 'memory' }, value: [1, '0.50'] },
      ]),
    ),
    1000,
  );
  const { cells } = materializeHeatmap(state, { now: 1000 });
  assert.equal(cells.length, 1);
  assert.equal(cells[0].podCount, 2);
  assert.equal(cells[0].cpu, 0.9);
  assert.equal(cells[0].saturation, 0.9);
  assert.equal(cells[0].level, 'critical');
});

test('groups cells by zone, sorts hottest first, and summarizes', () => {
  const state = createHeatmapState();
  ingestSamples(
    state,
    [
      { node: 'w1', zone: 'az-b', pod: 'p', resource: 'cpu', value: 0.2 },
      { node: 'w2', zone: 'az-a', pod: 'p', resource: 'cpu', value: 0.95 },
      { node: 'w3', zone: 'az-a', pod: 'p', resource: 'cpu', value: 0.1 },
    ],
    2000,
  );
  const view = materializeHeatmap(state, { now: 2000 });
  assert.deepEqual(
    view.zones.map((zone) => zone.zone),
    ['az-a', 'az-b'],
  );
  assert.deepEqual(
    view.zones[0].cells.map((cell) => cell.id),
    ['w2', 'w3'],
  );
  assert.equal(view.summary.nodeCount, 3);
  assert.equal(view.summary.hottest.id, 'w2');
  assert.equal(view.summary.byLevel.critical, 1);
});

test('flags a vanished worker node as draining, then evicts it after the stale window', () => {
  const state = createHeatmapState();
  ingestSamples(state, [{ node: 'w1', zone: 'az-a', pod: 'p', resource: 'cpu', value: 0.5 }], 0, 10000);
  ingestSamples(state, [{ node: 'w1', zone: 'az-a', pod: 'p', resource: 'cpu', value: 0.5 }], 0, 10000);

  // Next poll no longer contains w1.
  ingestSamples(state, [], 5000, 10000);
  let view = materializeHeatmap(state, { now: 5000, staleAfterMs: 10000 });
  assert.equal(view.cells.length, 1);
  assert.equal(view.cells[0].state, 'draining');

  ingestSamples(state, [], 20000, 10000);
  view = materializeHeatmap(state, { now: 20000, staleAfterMs: 10000 });
  assert.equal(view.cells.length, 0);
  assert.equal(view.summary.nodeCount, 0);
});

test('marks stale cells when the whole scrape ages out without eviction', () => {
  const state = createHeatmapState();
  ingestSamples(state, [{ node: 'w1', zone: 'az-a', pod: 'p', resource: 'cpu', value: 0.5 }], 0, 60000);
  const view = materializeHeatmap(state, { now: 30000, staleAfterMs: 10000 });
  assert.equal(view.cells[0].state, 'stale');
});

test('records an endpoint error while keeping the last good cells', () => {
  const state = createHeatmapState();
  ingestSamples(state, [{ node: 'w1', zone: 'az-a', pod: 'p', resource: 'cpu', value: 0.7 }], 1000);
  markError(state, new Error('prometheus responded 503'), 2000);
  const view = materializeHeatmap(state, { now: 2000 });
  assert.equal(view.cells.length, 1);
  assert.equal(view.summary.lastError.message, 'prometheus responded 503');
});

test('handles a 100-node three-zone cluster in a single pass', () => {
  const state = createHeatmapState();
  const samples = [];
  for (let index = 0; index < 100; index += 1) {
    const zone = `az-${'abc'[index % 3]}`;
    for (let pod = 0; pod < 5; pod += 1) {
      samples.push({ node: `worker-${index}`, zone, pod: `pod-${index}-${pod}`, resource: 'cpu', value: (index % 100) / 100 });
      samples.push({ node: `worker-${index}`, zone, pod: `pod-${index}-${pod}`, resource: 'memory', value: 0.3 });
    }
  }
  ingestSamples(state, samples, 1000);
  const view = materializeHeatmap(state, { now: 1000 });
  assert.equal(view.summary.nodeCount, 100);
  assert.equal(view.zones.reduce((sum, zone) => sum + zone.cells.length, 0), 100);
});
