import test from 'node:test';
import assert from 'node:assert/strict';
import {
  SHADE_LATENCY_CEILING_MS,
  SHADE_LATENCY_FLOOR_MS,
  agreementForPair,
  buildQuorumMatrix,
  cellAt,
  cellForPosition,
  cellColor,
  cellShade,
  emptyMatrix,
  matrixStats,
  trustWeight,
} from './quorumMatrixModel.js';

test('builds an N×N cell grid from a topology snapshot', () => {
  const matrix = buildQuorumMatrix({
    nodes: [
      { id: 'A', phase: 'EXTERNALIZE' },
      { id: 'B', phase: 'EXTERNALIZE' },
      { id: 'C', phase: 'CONFIRM' },
    ],
    edges: [
      { source: 'A', target: 'B' },
      { source: 'A', target: 'C' },
      { source: 'A', target: 'missing' },
      { source: 'A', target: 'B' },
    ],
  });
  assert.equal(matrix.size, 3);
  assert.equal(matrix.cells.length, 2);
  assert.deepEqual(matrix.cells[0], {
    sourceIndex: 0, targetIndex: 1, agreement: 'agreeing', trust: 0.9, latencyMs: 0,
  });
});

test('classifies agreement from validator phases and stalls', () => {
  const externalize = { phase: 'EXTERNALIZE', stalled: false };
  const confirm = { phase: 'CONFIRM', stalled: false };
  const stalled = { phase: 'EXTERNALIZE', stalled: true };
  const unknown = { phase: 'UNKNOWN', stalled: false };
  assert.equal(agreementForPair(externalize, externalize), 'agreeing');
  assert.equal(agreementForPair(confirm, confirm), 'confirming');
  assert.equal(agreementForPair(externalize, confirm), 'lagging');
  assert.equal(agreementForPair(stalled, externalize), 'diverged');
  assert.equal(agreementForPair(unknown, externalize), 'unknown');
});

test('trust weight clamps to [0,1] and penalizes stalls', () => {
  const healthy = { phase: 'EXTERNALIZE', stalled: false };
  const stalled = { phase: 'EXTERNALIZE', stalled: true };
  assert.ok(trustWeight(healthy, healthy) > 0.8);
  assert.ok(trustWeight(stalled, healthy) < 0.2);
  assert.equal(trustWeight(null, null), 0);
});

test('matrixStats aggregates counts, trust and latency', () => {
  const matrix = buildQuorumMatrix({
    nodes: [
      { id: 'A', phase: 'EXTERNALIZE', ledger_time_ms: 4 },
      { id: 'B', phase: 'EXTERNALIZE', ledger_time_ms: 6 },
    ],
    edges: [{ source: 'A', target: 'B' }],
  });
  const stats = matrixStats(matrix);
  assert.equal(stats.cells, 1);
  assert.equal(stats.counts.agreeing, 1);
  assert.ok(stats.avgTrust > 0);
  assert.ok(stats.avgLatencyMs > 0);
});

test('cellForPosition resolves matrix coordinates and rejects out of range', () => {
  const matrix = buildQuorumMatrix({
    nodes: [{ id: 'A' }, { id: 'B' }, { id: 'C' }],
    edges: [{ source: 'B', target: 'C' }],
  });
  const cell = cellForPosition(matrix, 1, 2);
  assert.equal(cell.sourceIndex, 1);
  assert.equal(cell.targetIndex, 2);
  assert.equal(cellForPosition(matrix, 0, 0), null);
  assert.equal(cellForPosition(matrix, -1, 0), null);
  assert.equal(cellForPosition(matrix, 5, 5), null);
});

test('cellAt matches cellForPosition and caches lookups per matrix', () => {
  const matrix = buildQuorumMatrix({
    nodes: [{ id: 'A' }, { id: 'B' }],
    edges: [{ source: 'A', target: 'B' }],
  });
  assert.equal(cellAt(matrix, 0, 1), cellForPosition(matrix, 0, 1));
  assert.equal(cellAt(matrix, 0, 1), matrix.cells[0]);
  assert.equal(cellAt(matrix, 1, 0), null);
  assert.equal(cellAt(matrix, -1, -1), null);
  assert.equal(cellAt(null, 0, 0), null);
});

test('cellShade dims by trust weight and latency delta', () => {
  const healthy = { agreement: 'agreeing', trust: 1, latencyMs: 0 };
  const weak = { agreement: 'agreeing', trust: 0, latencyMs: SHADE_LATENCY_CEILING_MS };
  const healthyShade = cellShade(healthy);
  const weakShade = cellShade(weak);
  assert.deepEqual(healthyShade.color, cellColor(healthy));
  assert.equal(healthyShade.opacity, 0.95);
  assert.ok(weakShade.color[0] < healthyShade.color[0]);
  assert.ok(weakShade.opacity >= 0.4 && weakShade.opacity < healthyShade.opacity);
  assert.deepEqual(cellShade(null), cellShade({ agreement: 'unknown', trust: 0, latencyMs: 0 }));
});

test('cellShade clamps latency past the ceiling', () => {
  const atCeiling = cellShade({ agreement: 'agreeing', trust: 0.5, latencyMs: SHADE_LATENCY_CEILING_MS });
  const farBeyond = cellShade({ agreement: 'agreeing', trust: 0.5, latencyMs: SHADE_LATENCY_CEILING_MS * 10 });
  assert.equal(atCeiling.opacity, farBeyond.opacity);
  assert.ok(atCeiling.opacity >= 0.4);
});

test('cellColor maps agreement states to RGB triples', () => {
  assert.deepEqual(cellColor({ agreement: 'agreeing' }), [0.22, 0.85, 0.54]);
  assert.deepEqual(cellColor({ agreement: 'diverged' }), [0.94, 0.36, 0.37]);
  assert.deepEqual(cellColor({ agreement: 'nope' }), [0.42, 0.47, 0.55]);
});

test('emptyMatrix returns a safe zero-size matrix', () => {
  assert.equal(emptyMatrix().size, 0);
  assert.deepEqual(emptyMatrix().cells, []);
});

test('scales to a 10,000-cell topology', () => {
  const nodes = Array.from({ length: 120 }, (_, index) => ({
    id: `N${index}`, phase: index % 2 ? 'EXTERNALIZE' : 'CONFIRM', ledger_time_ms: 4 + (index % 7),
  }));
  const edges = [];
  for (let ring = 1; edges.length < 10000 && ring < 120; ring += 1) {
    for (let source = 0; source < 120 && edges.length < 10000; source += 1) {
      edges.push({ source: `N${source}`, target: `N${(source + ring) % 120}` });
    }
  }
  const matrix = buildQuorumMatrix({ nodes, edges });
  assert.equal(matrix.size, 120);
  assert.equal(matrix.cells.length, 10000);
});
