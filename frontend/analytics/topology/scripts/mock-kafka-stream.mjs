#!/usr/bin/env node
import { WebSocketServer } from 'ws';

const args = new Map();
for (let index = 2; index < process.argv.length; index += 1) {
  const value = process.argv[index];
  if (value.startsWith('--')) args.set(value, process.argv[index + 1] ?? true);
}

const nodeCount = Math.max(1, Number(args.get('--nodes') ?? 500));
const edgeCount = Math.max(nodeCount, Number(args.get('--edges') ?? 2000));
const intervalMs = Math.max(20, Number(args.get('--interval') ?? 120));
const port = Number(args.get('--port') ?? 8787);
const serve = args.has('--serve');

const phases = ['PREPARE', 'CONFIRM', 'EXTERNALIZE'];
const clusters = ['eu-west', 'us-east', 'ap-south', 'us-west', 'af-south'];
const nodes = Array.from({ length: nodeCount }, (_, index) => {
  const id = `GMOCK${String(index).padStart(4, '0')}`;
  return {
    id,
    full_id: id,
    node_name: `validator-${String(index + 1).padStart(3, '0')}`,
    cluster: clusters[index % clusters.length],
    phase: 'EXTERNALIZE',
    is_critical: index < 7,
    stalled: false,
    threshold: 3,
    ballot_counter: 42,
    tps: 850 + (index % 70),
    ledger_time_ms: 4.1 + (index % 8) / 10,
  };
});

const edges = [];
const edgeKeys = new Set();
function addEdge(source, target) {
  const key = `${source}\u0000${target}`;
  if (source !== target && !edgeKeys.has(key) && edges.length < edgeCount) {
    edgeKeys.add(key);
    edges.push({ source, target });
  }
}

for (let index = 0; index < nodeCount && edges.length < edgeCount; index += 1) {
  addEdge(nodes[index].id, nodes[(index + 1) % nodeCount].id);
  addEdge(nodes[index].id, nodes[(index + 17) % nodeCount].id);
  addEdge(nodes[index].id, nodes[(index + 61) % nodeCount].id);
  addEdge(nodes[index].id, nodes[(index + 113) % nodeCount].id);
}
for (let index = edges.length; index < edgeCount; index += 1) {
  addEdge(nodes[index % nodeCount].id, nodes[(index * 37 + 11) % nodeCount].id);
}

const initialSnapshot = {
  nodes,
  edges,
  timestamp: new Date().toISOString(),
  healthy: true,
};

function update(sequence) {
  const index = sequence % nodeCount;
  const node = nodes[index];
  const phase = phases[sequence % phases.length];
  const fallingBehind = sequence % 47 === 0;
  return {
    message_id: `mock-${sequence}`,
    timestamp: new Date().toISOString(),
    node_id: node.id,
    node_name: node.node_name,
    cluster: node.cluster,
    phase: fallingBehind ? 'UNKNOWN' : phase,
    stalled: fallingBehind,
    ballot_counter: 43 + Math.floor(sequence / nodeCount),
    metrics: {
      tps: fallingBehind ? 0 : 820 + ((sequence * 13) % 180),
      ledger_time_ms: fallingBehind ? 28 + (sequence % 5) : 3.8 + ((sequence * 7) % 20) / 10,
    },
    quorum_set: {
      threshold: node.threshold,
      validators: edges.filter((edge) => edge.source === node.id).map((edge) => edge.target),
    },
  };
}

function emit(record) {
  if (!process.stdout.destroyed) process.stdout.write(`${JSON.stringify(record)}\n`);
}

process.stdout.on('error', (error) => {
  if (error.code === 'EPIPE') process.exit(0);
});

if (!serve) {
  emit(initialSnapshot);
  let sequence = 0;
  setInterval(() => emit(update(sequence++)), intervalMs);
} else {
  const server = new WebSocketServer({ port });
  server.on('listening', () => {
    console.log(`Mock Kafka stream listening on ws://localhost:${port}`);
    console.log(`Topology: ${nodes.length} nodes, ${edges.length} edges, ${intervalMs}ms updates`);
  });
  server.on('connection', (socket) => {
    socket.send(JSON.stringify(initialSnapshot));
    let sequence = 0;
    const timer = setInterval(() => {
      if (socket.readyState === socket.OPEN) socket.send(JSON.stringify(update(sequence++)));
    }, intervalMs);
    socket.on('close', () => clearInterval(timer));
  });
}
