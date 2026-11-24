#!/usr/bin/env node
/**
 * mock-prometheus.mjs
 *
 * Lightweight HTTP server that mimics the Prometheus HTTP API
 * (`GET /api/v1/query`) for the Stellar-K8s heatmap dev/test workflow.
 *
 * Simulates 100 worker nodes across three availability zones experiencing a
 * rolling CPU spike that propagates across all zones over ~60 seconds.
 * Memory utilization varies independently per node.
 *
 * Usage:
 *   node scripts/mock-prometheus.mjs              # default port 9091
 *   node scripts/mock-prometheus.mjs --port 9091  # explicit port
 *   node scripts/mock-prometheus.mjs --nodes 100 --spike-window 20
 *
 * The Vite dev proxy (`/api/prometheus`) forwards requests to this server.
 * Pass `?prometheusUrl=/api/prometheus` to the heatmap component (the default)
 * or open the app with `?heatmap=mock` to activate mock mode.
 *
 * Response shape (Prometheus instant-vector format):
 * {
 *   "status": "success",
 *   "data": {
 *     "resultType": "vector",
 *     "result": [
 *       {
 *         "metric": { "__name__": "stellar_operator_resource_usage",
 *                     "node": "worker-042",
 *                     "resource": "cpu",
 *                     "zone": "us-east-1a",
 *                     "namespace": "stellar",
 *                     "pod": "stellar-core-042" },
 *         "value": [<unix_timestamp>, "<ratio_0_to_1>"]
 *       },
 *       ...
 *     ]
 *   }
 * }
 */
import http from 'node:http';

// ── CLI arg parsing ────────────────────────────────────────────────────────
const args = new Map();
for (let i = 2; i < process.argv.length; i++) {
  const v = process.argv[i];
  if (v.startsWith('--')) args.set(v, process.argv[i + 1] ?? true);
}

const PORT = Number(args.get('--port') ?? 9091);
const NODE_COUNT = Math.max(1, Number(args.get('--nodes') ?? 100));
const SPIKE_WINDOW = Math.max(5, Number(args.get('--spike-window') ?? 20)); // nodes in spike at once
const UPDATE_INTERVAL_MS = Math.max(500, Number(args.get('--interval') ?? 1000));

// ── Zone / node topology ───────────────────────────────────────────────────
const ZONES = ['us-east-1a', 'eu-west-1b', 'ap-southeast-1c'];

/**
 * @typedef {Object} WorkerNode
 * @property {string} id
 * @property {string} zone
 * @property {string} namespace
 * @property {string} pod
 * @property {number} baseCpu      baseline CPU ratio [0,1]
 * @property {number} baseMem      baseline memory ratio [0,1]
 * @property {number} cpuNoise     per-tick noise amplitude
 */

/** @type {WorkerNode[]} */
const WORKERS = Array.from({ length: NODE_COUNT }, (_, i) => ({
  id: `worker-${String(i).padStart(3, '0')}`,
  zone: ZONES[i % ZONES.length],
  namespace: 'stellar',
  pod: `stellar-core-${String(i).padStart(3, '0')}`,
  baseCpu: 0.15 + (i % 20) * 0.007,
  baseMem: 0.25 + (i % 15) * 0.009,
  cpuNoise: 0.02 + (i % 7) * 0.003,
}));

// ── State ──────────────────────────────────────────────────────────────────

/** Index of the "front" of the rolling spike wave. */
let spikeHead = 0;

/** Current CPU ratio per node (updated every tick). */
const cpuRatios = new Float64Array(NODE_COUNT);
const memRatios = new Float64Array(NODE_COUNT);

function tick() {
  const now = Date.now();
  spikeHead = (spikeHead + 1) % NODE_COUNT;

  for (let i = 0; i < NODE_COUNT; i++) {
    const w = WORKERS[i];
    // Distance from spike head (circular).
    const dist = (i - spikeHead + NODE_COUNT) % NODE_COUNT;
    const inSpike = dist < SPIKE_WINDOW;

    const spikeMagnitude = inSpike
      ? 0.5 + 0.45 * Math.sin((Math.PI * (SPIKE_WINDOW - dist)) / SPIKE_WINDOW)
      : 0;

    const noise = (Math.random() - 0.5) * w.cpuNoise;
    cpuRatios[i] = Math.min(1, Math.max(0, w.baseCpu + spikeMagnitude + noise));

    // Memory has slower drift, unrelated to the spike.
    const memDrift = 0.04 * Math.sin(now / 60_000 + i * 0.3);
    memRatios[i] = Math.min(1, Math.max(0, w.baseMem + memDrift + (Math.random() - 0.5) * 0.01));
  }
}

// Run the simulation on its own timer independent of HTTP requests.
tick(); // initialise immediately
setInterval(tick, UPDATE_INTERVAL_MS);

// ── HTTP handler ───────────────────────────────────────────────────────────

/**
 * Builds a Prometheus instant-vector response from the current state.
 * Returns both cpu and memory samples for every node.
 */
function buildResponse() {
  const ts = Date.now() / 1000;
  const result = [];

  for (let i = 0; i < NODE_COUNT; i++) {
    const w = WORKERS[i];
    const baseMetric = {
      __name__: 'stellar_operator_resource_usage',
      node: w.id,
      zone: w.zone,
      namespace: w.namespace,
      pod: w.pod,
    };

    result.push({
      metric: { ...baseMetric, resource: 'cpu' },
      value: [ts, cpuRatios[i].toFixed(4)],
    });
    result.push({
      metric: { ...baseMetric, resource: 'memory' },
      value: [ts, memRatios[i].toFixed(4)],
    });
  }

  return {
    status: 'success',
    data: { resultType: 'vector', result },
  };
}

const server = http.createServer((req, res) => {
  // CORS – allow the Vite dev server origin.
  res.setHeader('Access-Control-Allow-Origin', '*');
  res.setHeader('Access-Control-Allow-Methods', 'GET, OPTIONS');
  res.setHeader('Access-Control-Allow-Headers', 'Content-Type');

  if (req.method === 'OPTIONS') {
    res.writeHead(204);
    res.end();
    return;
  }

  const url = new URL(req.url, `http://localhost:${PORT}`);

  if (url.pathname === '/api/v1/query' || url.pathname === '/api/v1/query_range') {
    const body = JSON.stringify(buildResponse());
    res.writeHead(200, { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(body) });
    res.end(body);
    return;
  }

  // Health check.
  if (url.pathname === '/-/healthy' || url.pathname === '/') {
    const info = JSON.stringify({ ok: true, nodes: NODE_COUNT, spikeHead });
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(info);
    return;
  }

  res.writeHead(404, { 'Content-Type': 'text/plain' });
  res.end('Not found');
});

server.listen(PORT, () => {
  console.log(`Mock Prometheus listening on http://localhost:${PORT}`);
  console.log(`  Simulating ${NODE_COUNT} worker nodes across ${ZONES.length} zones`);
  console.log(`  Rolling spike window: ${SPIKE_WINDOW} nodes`);
  console.log(`  State update interval: ${UPDATE_INTERVAL_MS} ms`);
  console.log();
  console.log('Endpoints:');
  console.log(`  GET http://localhost:${PORT}/api/v1/query?query=stellar_operator_resource_usage`);
  console.log(`  GET http://localhost:${PORT}/-/healthy`);
  console.log();
  console.log('In another terminal, start the Vite dev server:');
  console.log('  npm run dev');
  console.log('Then open the app and select "Heatmap" to visualise the mock data.');
});

server.on('error', (err) => {
  console.error('Mock Prometheus server error:', err.message);
  process.exit(1);
});
