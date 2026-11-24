/**
 * heatmapModel.test.js
 *
 * Unit tests for the heatmap data model.
 * Run with: node --test src/heatmapModel.test.js
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  saturationBand,
  mergeSamples,
  parsePrometheusResponse,
  applyPrometheusResponse,
  materializeNodes,
  MAX_NODES,
  BAND_COLORS,
} from './heatmapModel.js';

// ---------------------------------------------------------------------------
// saturationBand
// ---------------------------------------------------------------------------

test('saturationBand: classifies zero as idle', () => {
  assert.equal(saturationBand(0), 'idle');
});

test('saturationBand: boundary at 40% is moderate', () => {
  assert.equal(saturationBand(40), 'moderate');
});

test('saturationBand: boundary at 70% is elevated', () => {
  assert.equal(saturationBand(70), 'elevated');
});

test('saturationBand: boundary at 85% is high', () => {
  assert.equal(saturationBand(85), 'high');
});

test('saturationBand: boundary at 95% is critical', () => {
  assert.equal(saturationBand(95), 'critical');
});

test('saturationBand: 100% is critical', () => {
  assert.equal(saturationBand(100), 'critical');
});

test('saturationBand: 39.9% is still idle', () => {
  assert.equal(saturationBand(39), 'idle');
});

// ---------------------------------------------------------------------------
// BAND_COLORS
// ---------------------------------------------------------------------------

test('BAND_COLORS: has an entry for every band returned by saturationBand', () => {
  const bands = ['idle', 'moderate', 'elevated', 'high', 'critical'];
  for (const band of bands) {
    assert.ok(BAND_COLORS[band], `Missing color for band: ${band}`);
  }
});

// ---------------------------------------------------------------------------
// mergeSamples
// ---------------------------------------------------------------------------

function makeSample(id, resource, valueRatio, extraLabels = {}) {
  return {
    metric: { node: id, ...extraLabels },
    value: [Date.now() / 1000, String(valueRatio)],
    resource,
  };
}

test('mergeSamples: creates a NodeMetric for a cpu sample', () => {
  const map = mergeSamples([makeSample('node-1', 'cpu', 0.5)]);
  assert.equal(map.size, 1);
  const node = map.get('node-1');
  assert.ok(node, 'node-1 should exist');
  assert.equal(node.cpuPct, 50);
  assert.equal(node.memPct, 0);
  assert.equal(node.saturationPct, 50);
  assert.equal(node.band, 'moderate');
});

test('mergeSamples: merges cpu and memory for same node into one record', () => {
  const samples = [
    makeSample('node-2', 'cpu', 0.8),
    makeSample('node-2', 'memory', 0.6),
  ];
  const map = mergeSamples(samples);
  assert.equal(map.size, 1);
  const node = map.get('node-2');
  assert.equal(node.cpuPct, 80);
  assert.equal(node.memPct, 60);
  assert.equal(node.saturationPct, 80); // max(80, 60)
  // 80% falls in the 70–84 range → elevated (high threshold starts at 85)
  assert.equal(node.band, 'elevated');
});

test('mergeSamples: sets missing=true for nodes absent from new samples', () => {
  const prev = mergeSamples([makeSample('node-a', 'cpu', 0.1)]);
  const next = mergeSamples([makeSample('node-b', 'cpu', 0.2)], prev);
  assert.equal(next.size, 2);
  assert.equal(next.get('node-a').missing, true);
  assert.equal(next.get('node-b').missing, false);
});

test('mergeSamples: clamps values above 1.0 (ratio) to 100%', () => {
  const map = mergeSamples([makeSample('node-x', 'cpu', 1.5)]);
  assert.equal(map.get('node-x').cpuPct, 100);
});

test('mergeSamples: handles non-finite value strings gracefully', () => {
  const sample = {
    metric: { node: 'bad' },
    value: [Date.now() / 1000, 'NaN'],
    resource: 'cpu',
  };
  const map = mergeSamples([sample]);
  assert.equal(map.get('bad').cpuPct, 0);
});

test('mergeSamples: uses pod label as key when present', () => {
  const sample = makeSample('node-1', 'cpu', 0.3, { namespace: 'stellar', pod: 'core-0' });
  const map = mergeSamples([sample]);
  assert.ok(map.has('stellar/core-0'), 'Key should be namespace/pod');
});

test('mergeSamples: drops oldest entries when over MAX_NODES', () => {
  const samples = Array.from({ length: MAX_NODES + 10 }, (_, i) =>
    makeSample(`node-${i}`, 'cpu', 0.1),
  );
  // Stagger lastSeen by index via explicit nowMs manipuation.
  let map = new Map();
  for (let i = 0; i < samples.length; i++) {
    map = mergeSamples([samples[i]], map, Date.now() + i);
  }
  assert.ok(map.size <= MAX_NODES, `Expected ≤${MAX_NODES} nodes, got ${map.size}`);
});

// ---------------------------------------------------------------------------
// parsePrometheusResponse
// ---------------------------------------------------------------------------

function makeVectorResponse(samples) {
  return {
    status: 'success',
    data: {
      resultType: 'vector',
      result: samples,
    },
  };
}

test('parsePrometheusResponse: parses a minimal instant-vector response', () => {
  const body = makeVectorResponse([
    { metric: { node: 'n1', resource: 'cpu' }, value: [1000, '0.45'] },
    { metric: { node: 'n1', resource: 'memory' }, value: [1000, '0.72'] },
  ]);
  const samples = parsePrometheusResponse(body);
  assert.equal(samples.length, 2);
  assert.equal(samples[0].resource, 'cpu');
  assert.equal(samples[1].resource, 'memory');
});

test('parsePrometheusResponse: returns empty array for failed status', () => {
  const body = { status: 'error', error: 'bad query', errorType: 'bad_data' };
  assert.deepEqual(parsePrometheusResponse(body), []);
});

test('parsePrometheusResponse: returns empty array for null input', () => {
  assert.deepEqual(parsePrometheusResponse(null), []);
});

test('parsePrometheusResponse: infers resource from metric name when label absent', () => {
  const body = makeVectorResponse([
    { metric: { __name__: 'stellar_operator_cpu_usage', node: 'n2' }, value: [1000, '0.3'] },
    { metric: { __name__: 'stellar_operator_memory_usage', node: 'n2' }, value: [1000, '0.5'] },
  ]);
  const samples = parsePrometheusResponse(body);
  assert.equal(samples[0].resource, 'cpu');
  assert.equal(samples[1].resource, 'memory');
});

test('parsePrometheusResponse: handles matrix result type using latest value', () => {
  const body = {
    status: 'success',
    data: {
      resultType: 'matrix',
      result: [
        {
          metric: { node: 'n3', resource: 'cpu' },
          values: [
            [990, '0.1'],
            [995, '0.2'],
            [1000, '0.9'],
          ],
        },
      ],
    },
  };
  const samples = parsePrometheusResponse(body);
  assert.equal(samples.length, 1);
  assert.equal(samples[0].value[1], '0.9');
});

// ---------------------------------------------------------------------------
// applyPrometheusResponse (integration)
// ---------------------------------------------------------------------------

test('applyPrometheusResponse: returns populated map from valid response', () => {
  const body = makeVectorResponse([
    { metric: { node: 'worker-1', resource: 'cpu' }, value: [1000, '0.92'] },
    { metric: { node: 'worker-1', resource: 'memory' }, value: [1000, '0.88'] },
  ]);
  const map = applyPrometheusResponse(body);
  const node = map.get('worker-1');
  assert.ok(node);
  assert.equal(node.cpuPct, 92);
  assert.equal(node.memPct, 88);
  // max(92, 88) = 92 → 85–94 range → high
  assert.equal(node.band, 'high');
});

// ---------------------------------------------------------------------------
// materializeNodes
// ---------------------------------------------------------------------------

test('materializeNodes: sorts by zone first then by saturation descending', () => {
  const map = new Map([
    ['a', { id: 'a', zone: 'us-east', saturationPct: 30, band: 'idle', missing: false, lastSeen: 0, node: 'a', namespace: '', pod: '', cpuPct: 30, memPct: 0 }],
    ['b', { id: 'b', zone: 'eu-west', saturationPct: 80, band: 'high', missing: false, lastSeen: 0, node: 'b', namespace: '', pod: '', cpuPct: 80, memPct: 0 }],
    ['c', { id: 'c', zone: 'us-east', saturationPct: 60, band: 'moderate', missing: false, lastSeen: 0, node: 'c', namespace: '', pod: '', cpuPct: 60, memPct: 0 }],
    ['d', { id: 'd', zone: 'eu-west', saturationPct: 50, band: 'moderate', missing: false, lastSeen: 0, node: 'd', namespace: '', pod: '', cpuPct: 50, memPct: 0 }],
  ]);

  const nodes = materializeNodes(map);
  // eu-west comes first alphabetically
  assert.equal(nodes[0].zone, 'eu-west');
  assert.equal(nodes[1].zone, 'eu-west');
  // within eu-west, highest saturation first
  assert.equal(nodes[0].saturationPct, 80);
  assert.equal(nodes[1].saturationPct, 50);
  // us-east second
  assert.equal(nodes[2].zone, 'us-east');
  assert.equal(nodes[3].zone, 'us-east');
  // within us-east, highest saturation first
  assert.equal(nodes[2].saturationPct, 60);
  assert.equal(nodes[3].saturationPct, 30);
});

test('materializeNodes: returns array (not Map)', () => {
  const map = new Map([
    ['x', { id: 'x', zone: '', saturationPct: 10, band: 'idle', missing: false, lastSeen: 0, node: 'x', namespace: '', pod: '', cpuPct: 10, memPct: 0 }],
  ]);
  const result = materializeNodes(map);
  assert.ok(Array.isArray(result));
});
