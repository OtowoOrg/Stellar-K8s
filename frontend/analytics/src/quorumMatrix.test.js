import test from 'node:test';
import assert from 'node:assert/strict';
import { buildQuorumMatrix, inspectMatrixCell, MATRIX_MAX_NODES, normalizeMatrixNodes } from './quorumMatrix.js';

test('builds bounded matrix values from trust and quorum overlap', () => {
  const matrix = buildQuorumMatrix({ nodes: [
    { id: 'A', trust: 0.8, quorum_set: { validators: ['B', 'C'] } },
    { id: 'B', trust: 1, quorum_set: { validators: ['A', 'C'] } },
    { id: 'C', trust: 0.6, quorum_set: { validators: ['A'] } },
  ] });
  assert.equal(matrix.size, 3);
  assert.equal(matrix.overlaps[0], 2);
  assert.equal(matrix.overlaps[1], 1);
  assert.ok(matrix.values[0] > 0 && matrix.values[0] <= 1);
});

test('returns cell details and shared dependencies', () => {
  const matrix = buildQuorumMatrix({ nodes: [
    { id: 'A', name: 'Alpha', quorum_set: { validators: ['B', 'C'] } },
    { id: 'B', name: 'Beta', quorum_set: { validators: ['A', 'C'] } },
    { id: 'C', name: 'Gamma', quorum_set: { validators: ['A'] } },
  ] });
  const cell = inspectMatrixCell(matrix, 0, 1);
  assert.equal(cell.source.name, 'Alpha');
  assert.deepEqual(cell.commonDependencies, ['C']);
  assert.equal(inspectMatrixCell(matrix, 9, 0), null);
});

test('limits the rendered dataset to 200 validators', () => {
  const nodes = Array.from({ length: MATRIX_MAX_NODES + 10 }, (_, index) => ({ id: `N${index}` }));
  assert.equal(normalizeMatrixNodes({ nodes }).length, MATRIX_MAX_NODES);
});
