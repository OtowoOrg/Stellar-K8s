# Stellar Network Topology Visualizer

A standalone React + Three.js SPA for inspecting multi-cluster SCP quorum topology. The graph uses instanced WebGL spheres for validators and one batched line buffer for quorum links. This keeps the scene object count constant while the graph updates.

## Run It

From `frontend/analytics`:

```bash
npm install
npm run dev
```

Open the Vite URL printed by the command. The default source is the operator WebSocket endpoint:

```text
/api/v1/quorum/topology/stream
```

The operator stream sends `QuorumTopologyResponse` snapshots every five seconds. The frontend also accepts individual JSON `ScpMessage` records. For a real Kafka topic, run `npm run stream:kafka`; the KafkaJS bridge consumes `KAFKA_TOPIC` from `KAFKA_BROKERS` and broadcasts JSON messages to browser clients over WebSocket.

## Mock Load Test

Start the deterministic 500-node / 2,000-edge stream in a second terminal:

```bash
npm run mock:stream
```

Choose **Mock Kafka stream** in the app, or open the app with `?source=mock`. Customize the workload when measuring hardware:

```bash
node scripts/mock-kafka-stream.mjs --serve --nodes 500 --edges 2000 --interval 120
```

For a real topic bridge:

```bash
KAFKA_BROKERS=localhost:9092 KAFKA_TOPIC=stellar-scp-messages npm run stream:kafka
```

Set `KAFKA_FROM_BEGINNING=true` to replay retained messages. The bridge expects JSON values, matching the existing JSON serialization path in the operator pipeline.

The generator sends one initial snapshot and then individual SCP messages containing phase, ballot, TPS, ledger time, and quorum-set updates. Without `--serve`, it writes newline-delimited JSON records to stdout for replay or piping into a Kafka producer.

## Data Mapping

Snapshot nodes use the existing operator fields: `id`, `full_id`, `phase`, `is_critical`, `threshold`, and `stalled`. Individual messages use the fields in `schemas/scp_message.proto`. TPS and ledger time are read from `metrics.tps`, `metrics.ledger_time_ms`, or equivalent snake/camel-case fields and metadata. The current repository SCP schemas do not define those two measurements, so live producers must enrich the message or metadata for the inspector to show them; the mock stream includes both.

Node colors indicate health: green is synced, amber is degraded, and red is falling behind or unknown. Click a node to inspect cluster, SCP phase, ballot, TPS, ledger time, and quorum threshold. OrbitControls provides drag-to-orbit, scroll-to-zoom, and pan interaction.

## Resource Saturation Heatmap

`src/heatmap/` adds a real-time CPU/memory saturation heatmap for Kubernetes worker
nodes. Switch to it with the **Saturation** toggle in the toolbar or open the app
with `?view=heatmap`.

The grid is a zone-banded, GitHub-contribution-graph layout: one cell per worker
node, grouped into availability-zone rows, colored on a cool (idle) to hot
(saturated) ramp. Cells carry `data-node`, `data-zone`, `data-level`, `data-state`,
and `data-saturation` attributes and a native `<title>` tooltip; hover or focus a
cell for the CPU/memory/pod breakdown.

The component polls a Prometheus HTTP API endpoint every 5 seconds for
`stellar_operator_resource_usage` (default `/api/v1/query`, override with
`?prom=<url>`). It accepts the vector JSON response, a bare `result` array, or the
`/metrics` text exposition. Per-poll work is O(samples): parsing and
re-materialization happen off the render path, results are coalesced into a single
`requestAnimationFrame`, handed to React via `startTransition`, and each cell is a
memoized `<rect>`, so a 100-node cluster refresh never blocks the main thread.

Edge cases: a worker node that drops out of the scrape is shown dashed
("draining") and evicted after `staleAfterMs` (15s); a stale scrape dims every
cell; an endpoint error keeps the last good grid and surfaces a banner.

### Mock 100-node cluster

```bash
npm run mock:prometheus            # 100 nodes, 3 zones, port 9091
node scripts/mock-prometheus.mjs --nodes 100 --zones 3 --spike-period 45 --port 9091
```

The mock simulates a CPU spike that rolls across every availability zone once per
`--spike-period` seconds and drops one worker node from the series every 20s
(disable with `--no-drop`). Point the app at it via the Vite dev proxy, e.g.
`?view=heatmap&prom=/mock-prom/api/v1/query` with a proxy entry, or run the mock on
the same origin the operator API uses.

## Checks

```bash
npm test
npm run build
npm run matrix:perf
```



```bash
node scripts/mock-fee-stream.mjs --serve --history-hours 48 --spike-hours 9,21 --interval 1000
```

Without `--serve` the generator emits newline-delimited JSON to stdout for replay. Live mode reads fee-enriched frames from `/api/v1/quorum/topology/stream`; when a frame carries no fee field, the feed infers a base fee from `tps`. The estimator model (`src/fees/feeModel.js`) and its tests (`npm test`) validate that historical fee spike data moves the congestion level and recommended tiers.
