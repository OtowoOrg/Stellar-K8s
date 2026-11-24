export const MATRIX_MOCK_NODES = 120;

function nodeId(index) {
  return `GMOCK${String(index).padStart(4, '0')}`;
}

function clusterFor(index) {
  if (index % 3 === 0) return 'eu-west';
  if (index % 3 === 1) return 'us-east';
  return 'ap-south';
}

function phaseFor(index) {
  if (index % 17 === 0) return 'UNKNOWN';
  if (index % 5 === 0) return 'CONFIRM';
  if (index % 23 === 0) return 'PREPARE';
  return 'EXTERNALIZE';
}

export function buildMatrixMockTopology({ nodes = MATRIX_MOCK_NODES, edges = 10000 } = {}) {
  const count = Math.max(2, nodes);
  const nodeList = Array.from({ length: count }, (_, index) => ({
    id: nodeId(index),
    full_id: nodeId(index),
    node_name: `validator-${String(index + 1).padStart(3, '0')}`,
    cluster: clusterFor(index),
    phase: phaseFor(index),
    is_critical: index < 5,
    stalled: index % 41 === 0,
    threshold: 3,
    ballot_counter: 42 + index,
    public_key: nodeId(index),
    tps: 800 + (index % 200),
    ledger_time_ms: 3.6 + (index % 30) / 10,
  }));

  const edgeList = [];
  const seen = new Set();
  const push = (source, target) => {
    if (source === target) return;
    const key = source * count + target;
    if (seen.has(key) || edgeList.length >= edges) return;
    seen.add(key);
    edgeList.push({ source: nodeId(source), target: nodeId(target) });
  };

  for (let index = 0; index < count && edgeList.length < edges; index += 1) {
    push(index, (index + 1) % count);
    push(index, (index + 7) % count);
    push(index, (index + 31) % count);
    push(index, (index + 83) % count);
  }
  let step = 0;
  let ring = 1;
  while (edgeList.length < edges && ring < count) {
    const source = step % count;
    const target = (source + ring) % count;
    push(source, target);
    step += 1;
    if (step >= count) {
      step = 0;
      ring += 1;
    }
  }

  return {
    nodes: nodeList,
    edges: edgeList,
    timestamp: new Date().toISOString(),
    healthy: true,
  };
}
