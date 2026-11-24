#!/usr/bin/env node

// Deterministic cache-load harness for the fail-open Soroban RPC proxy.
// Usage: node benchmarks/soroban-cache-load-test.js [proxy-url]

const proxyUrl = process.argv[2] || process.env.CACHE_PROXY_URL || "http://127.0.0.1:18000";
const totalRequests = 10_000;
const uniqueReads = 100;
const concurrency = 100;
const runPrefix = `run-${Date.now()}-${Math.random().toString(16).slice(2)}`;

async function request(params) {
  const response = await fetch(proxyUrl, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: params.id,
      method: "getLedgerEntries",
      params: { keys: [params.key] },
    }),
  });
  if (!response.ok) throw new Error(`proxy returned HTTP ${response.status}`);
  const body = await response.json();
  if (body.error) throw new Error(`RPC error: ${JSON.stringify(body.error)}`);
  return body;
}

async function stats() {
  const response = await fetch(`${proxyUrl}/stats`);
  if (!response.ok) throw new Error(`stats endpoint returned HTTP ${response.status}`);
  return response.json();
}

async function run() {
  const baseline = await stats();
  // Warm each key once so concurrent request scheduling cannot create duplicate
  // misses for the same key; the measured phase then represents steady-state load.
  for (let index = 0; index < uniqueReads; index += 1) {
    await request({ id: `warm-${index}`, key: `${runPrefix}-state-${index}` });
  }

  const started = performance.now();
  let next = 0;
  let completed = 0;
  const workers = Array.from({ length: concurrency }, async () => {
    while (true) {
      const index = next++;
      if (index >= totalRequests) return;
      await request({
        id: index,
        key: `${runPrefix}-state-${index % uniqueReads}`,
      });
      completed++;
    }
  });
  await Promise.all(workers);

  const elapsedMs = performance.now() - started;
  const observed = await stats();
  const upstreamDelta = observed.upstreamRequests - baseline.upstreamRequests;
  const hitDelta = observed.hits - baseline.hits;
  if (upstreamDelta !== uniqueReads) {
    throw new Error(
      `expected ${uniqueReads} upstream reads for this run, observed ${upstreamDelta}`,
    );
  }
  if (hitDelta < totalRequests) {
    throw new Error(`expected at least ${totalRequests} cache hits for this run, observed ${hitDelta}`);
  }

  console.log(JSON.stringify({
    proxyUrl,
    totalRequests: completed,
    uniqueReads,
    expectedUpstreamReads: uniqueReads,
    expectedCacheHits: totalRequests - uniqueReads,
    concurrency,
    elapsedMs: Number(elapsedMs.toFixed(2)),
    requestsPerSecond: Number((completed / (elapsedMs / 1000)).toFixed(2)),
    baseline,
    observed,
    upstreamDelta,
    hitDelta,
    assertions: [
      "all requests returned successfully",
      "the warmed proxy produced exactly 100 upstream reads for this run",
      "proxy cache failures must still return the upstream response",
    ],
  }, null, 2));
}

run().catch((error) => {
  console.error(error.message);
  process.exitCode = 1;
});
