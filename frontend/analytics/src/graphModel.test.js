import test from 'node:test';
import assert from 'node:assert/strict';
import { applyMessage, createStreamState, ingest, materialize, normalizeSnapshot, statusForNode } from './graphModel.js';

test('normalizes operator topology snapshots and removes invalid duplicate edges', () => {
  const snapshot = normalizeSnapshot({
    nodes: [{ id: 'A', phase: 'externalize' }, { id: 'B', phase: 'CONFIRM' }],
    edges: [{ source: 'A', target: 'B' }, { source: 'A', target: 'B' }, { source: 'A', target: 'missing' }],
  });
  assert.equal(snapshot.nodes[0].phase, 'EXTERNALIZE');
  assert.deepEqual(snapshot.edges, [{ source: 'A', target: 'B' }]);
});

test('matches full-key messages to Unicode-short snapshot IDs and keeps metrics', () => {
  const fullId = 'GCEZWKCA5VLDNRLN3RPRJMRZOX3Z6G5CHCGBWRXSJHEG8VORHEA3PUO';
  const state = createStreamState({
    nodes: [{ id: 'GCEZ…PUO', full_id: fullId }, { id: 'GDEST…1234', full_id: 'GDEST1234' }],
    edges: [],
  });
  applyMessage(state, {
    node_id: fullId,
    phase: 'EXTERNALIZE',
    metrics: { tps: 400, ledger_time_ms: 3.5 },
    quorum_set: { validators: ['GDEST…1234'] },
  });
  const graph = materialize(state);
  assert.equal(graph.nodes.length, 2);
  assert.equal(graph.nodes.find((node) => node.id === 'GCEZ…PUO').tps, 400);
  assert.deepEqual(graph.edges, [{ source: 'GCEZ…PUO', target: 'GDEST…1234' }]);
});

test('accepts the Rust producer camelCase JSON contract', () => {
  const state = createStreamState({ nodes: [{ id: 'A' }, { id: 'B' }], edges: [] });
  applyMessage(state, {
    nodeId: 'A',
    phase: 'EXTERNALIZE',
    ballotCounter: 12,
    quorumSet: { t: 2, v: ['B'], innerSets: [] },
    metadata: { tps: '99.5', ledger_time_ms: '5.25' },
  });
  const node = materialize(state).nodes.find((item) => item.id === 'A');
  assert.equal(node.ballotCounter, 12);
  assert.equal(node.tps, 99.5);
  assert.equal(node.ledgerTimeMs, 5.25);
  assert.equal(node.threshold, 2);
  assert.deepEqual(materialize(state).edges, [{ source: 'A', target: 'B' }]);
});

test('updates a node from an SCP message without replacing the existing graph', () => {
  const state = createStreamState({ nodes: [{ id: 'A' }, { id: 'B' }], edges: [] });
  applyMessage(state, {
    node_id: 'A',
    phase: 'EXTERNALIZE',
    ballot_counter: 9,
    metrics: { tps: 123.4, ledger_time_ms: 4.2 },
    quorum_set: { validators: ['B'] },
  });
  const graph = materialize(state);
  assert.equal(graph.nodes.find((node) => node.id === 'A').tps, 123.4);
  assert.deepEqual(graph.edges, [{ source: 'A', target: 'B' }]);
});

test('defers an edge until its target validator arrives', () => {
  const state = createStreamState({ nodes: [{ id: 'A' }], edges: [] });
  applyMessage(state, { node_id: 'A', quorum_set: { validators: ['B'] } });
  assert.deepEqual(materialize(state).edges, []);
  applyMessage(state, { node_id: 'B', phase: 'EXTERNALIZE' });
  assert.deepEqual(materialize(state).edges, [{ source: 'A', target: 'B' }]);
});

test('accepts both snapshots and individual messages through ingest', () => {
  let state = createStreamState({ nodes: [{ id: 'A' }], edges: [] });
  state = ingest(state, { node_id: 'A', phase: 'CONFIRM' });
  assert.equal(materialize(state).nodes[0].phase, 'CONFIRM');
  state = ingest(state, { nodes: [{ id: 'B', phase: 'EXTERNALIZE' }], edges: [] });
  assert.equal(materialize(state).nodes[0].id, 'B');
});

test('converts ledger time seconds to milliseconds without changing millisecond fields', () => {
  const state = createStreamState({ nodes: [{ id: 'A' }], edges: [] });
  applyMessage(state, { node_id: 'A', ledger_time: 0.0045 });
  assert.equal(materialize(state).nodes[0].ledgerTimeMs, 4.5);
  applyMessage(state, { node_id: 'A', ledger_time_ms: 6.25 });
  assert.equal(materialize(state).nodes[0].ledgerTimeMs, 6.25);
});

test('classifies health statuses for the visual legend', () => {
  assert.equal(statusForNode({ phase: 'EXTERNALIZE' }), 'synced');
  assert.equal(statusForNode({ phase: 'CONFIRM' }), 'degraded');
  assert.equal(statusForNode({ phase: 'UNKNOWN' }), 'falling-behind');
});
