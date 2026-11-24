#!/usr/bin/env node
//
// Mock Prometheus endpoint for the worker-node saturation heatmap.
//
// Simulates a multi-availability-zone Kubernetes cluster running Stellar Core
// where a CPU spike rolls across the zones. Every zone takes its turn at the
// top of the wave once per `--spike-period` seconds, so the heatmap shows a
// travelling hot band. One worker node also drops out of the series on a
// rotating schedule to exercise the "node disappeared" edge case.
//
// Endpoints:
//   GET /api/v1/query?query=stellar_operator_resource_usage  -> vector JSON
//   GET /metrics                                              -> text exposition
//   GET /-/healthy                                            -> liveness
//
// Usage:
//   node scripts/mock-prometheus.mjs --nodes 100 --zones 3 --port 9091
//   node scripts/mock-prometheus.mjs --spike-period 45 --no-drop

import http from 'node:http';

const args = new Map();
for (let index = 2; index < process.argv.length; index += 1) {
  const token = process.argv[index];
  if (!token.startsWith('--')) continue;
  const next = process.argv[index + 1];
  args.set(token.slice(2), next && !next.startsWith('--') ? next : true);
}

const NODE_COUNT = Math.max(1, Number(args.get('nodes') ?? 100));
const ZONE_COUNT = Math.max(1, Number(args.get('zones') ?? 3));
const PORT = Number(args.get('port') ?? 9091);
const SPIKE_PERIOD_MS = Math.max(5000, Number(args.get('spike-period') ?? 60) * 1000);
const PODS_MIN = Math.max(1, Number(args.get('pods-min') ?? 2));
const PODS_MAX = Math.max(PODS_MIN, Number(args.get('pods-max') ?? 6));
const DROP_NODES = args.get('no-drop') ? false : true;
const DROP_PERIOD_MS = Math.max(5000, Number(args.get('drop-period') ?? 20) * 1000);
const METRIC = 'stellar_operator_resource_usage';

const ZONES = Array.from({ length: ZONE_COUNT }, (_, index) => `az-${String.fromCharCode(97 + (index % 26))}`);

function hash01(text) {
  let hash = 2166136261;
  for (let index = 0; index < text.length; index += 1) {
    hash = Math.imul(hash ^ text.charCodeAt(index), 16777619);
  }
  return ((hash >>> 0) % 1000000) / 1000000;
}

function clamp01(value) {
  return Math.min(1, Math.max(0, value));
}

function circularDistance(a, b, size) {
  const raw = Math.abs(a - b) % size;
  return Math.min(raw, size - raw);
}

const nodes = Array.from({ length: NODE_COUNT }, (_, index) => {
  const zoneIndex = index % ZONE_COUNT;
  const podCount = PODS_MIN + Math.floor(hash01(`pods-${index}`) * (PODS_MAX - PODS_MIN + 1));
  return {
    index,
    name: `worker-${String(index).padStart(3, '0')}`,
    zoneIndex,
    zone: ZONES[zoneIndex],
    instance: `10.${zoneIndex}.${Math.floor(index / 250)}.${10 + (index % 240)}:9100`,
    pods: Array.from({ length: podCount }, (_, pod) => `stellar-core-${index}-${pod}`),
  };
});

function droppedNodeIndex(now) {
  if (!DROP_NODES) return -1;
  return Math.floor(now / DROP_PERIOD_MS) % NODE_COUNT;
}

function buildSamples(now) {
  const seconds = now / 1000;
  const spikeCenter = ((now / SPIKE_PERIOD_MS) * ZONE_COUNT) % ZONE_COUNT;
  const dropped = droppedNodeIndex(now);
  const samples = [];

  for (const node of nodes) {
    if (node.index === dropped) continue;
    const zoneHeat = 0.55 * Math.exp(-(circularDistance(node.zoneIndex, spikeCenter, ZONE_COUNT) ** 2) / 0.5);
    const drift = 0.06 * Math.sin(seconds / 7 + node.index);

    for (const pod of node.pods) {
      const bucket = Math.floor(now / 5000);
      const cpuJitter = 0.14 * (hash01(`${pod}:${bucket}`) - 0.5);
      const cpu = clamp01(0.16 + zoneHeat + drift + cpuJitter);
      const memory = clamp01(0.32 + 0.26 * hash01(`${pod}:mem`) + 0.08 * Math.sin(seconds / 31 + node.index));
      samples.push({ node, pod, resource: 'cpu', value: cpu });
      samples.push({ node, pod, resource: 'memory', value: memory });
    }
  }
  return samples;
}

function vectorResponse(now) {
  const timestamp = now / 1000;
  return {
    status: 'success',
    data: {
      resultType: 'vector',
      result: buildSamples(now).map((sample) => ({
        metric: {
          __name__: METRIC,
          node: sample.node.name,
          zone: sample.node.zone,
          pod: sample.pod,
          resource: sample.resource,
          instance: sample.node.instance,
        },
        value: [timestamp, sample.value.toFixed(4)],
      })),
    },
  };
}

function metricsText(now) {
  const lines = [
    `# HELP ${METRIC} Fraction (0-1) of a resource limit consumed by a Stellar Core pod.`,
    `# TYPE ${METRIC} gauge`,
  ];
  for (const sample of buildSamples(now)) {
    const labels = [
      `node="${sample.node.name}"`,
      `zone="${sample.node.zone}"`,
      `pod="${sample.pod}"`,
      `resource="${sample.resource}"`,
      `instance="${sample.node.instance}"`,
    ].join(',');
    lines.push(`${METRIC}{${labels}} ${sample.value.toFixed(4)}`);
  }
  return `${lines.join('\n')}\n`;
}

const server = http.createServer((req, res) => {
  res.setHeader('Access-Control-Allow-Origin', '*');
  res.setHeader('Access-Control-Allow-Headers', '*');
  if (req.method === 'OPTIONS') {
    res.writeHead(204);
    res.end();
    return;
  }

  const url = new URL(req.url, `http://${req.headers.host ?? 'localhost'}`);
  const now = Date.now();

  if (url.pathname === '/api/v1/query') {
    res.writeHead(200, { 'content-type': 'application/json' });
    res.end(JSON.stringify(vectorResponse(now)));
    return;
  }
  if (url.pathname === '/metrics') {
    res.writeHead(200, { 'content-type': 'text/plain; version=0.0.4; charset=utf-8' });
    res.end(metricsText(now));
    return;
  }
  if (url.pathname === '/-/healthy' || url.pathname === '/-/ready' || url.pathname === '/') {
    res.writeHead(200, { 'content-type': 'text/plain' });
    res.end('mock prometheus ok\n');
    return;
  }

  res.writeHead(404, { 'content-type': 'text/plain' });
  res.end('not found\n');
});

server.listen(PORT, () => {
  const podTotal = nodes.reduce((sum, node) => sum + node.pods.length, 0);
  console.log(`Mock Prometheus listening on http://localhost:${PORT}`);
  console.log(`  ${NODE_COUNT} worker nodes / ${podTotal} pods across ${ZONES.join(', ')}`);
  console.log(`  CPU spike sweeps every zone every ${SPIKE_PERIOD_MS / 1000}s`);
  if (DROP_NODES) console.log(`  one worker node leaves the series every ${DROP_PERIOD_MS / 1000}s (edge case)`);
  console.log(`  query:   http://localhost:${PORT}/api/v1/query?query=${METRIC}`);
  console.log(`  metrics: http://localhost:${PORT}/metrics`);
});
