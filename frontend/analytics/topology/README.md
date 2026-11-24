# Stellar Network Topology Visualizer

Standalone React + Three.js SPA for inspecting Stellar-K8s multi-cluster SCP quorum topology. The renderer uses one instanced mesh for validators and one batched line buffer for quorum links, so scene object count stays stable while live stream data changes.

## Run

From `frontend/analytics/topology`:

```bash
npm install
npm run dev
```

Open the Vite URL printed by the command. The default stream is the operator WebSocket endpoint:

```text
/api/v1/quorum/topology/stream
```

The app also supports:

```bash
npm run mock:stream
```

Then choose `Mock stream` in the UI or open `?source=mock`. Customize the 500-node / 2,000-edge workload:

```bash
node scripts/mock-kafka-stream.mjs --serve --nodes 500 --edges 2000 --interval 120
```

For Kafka:

```bash
KAFKA_BROKERS=localhost:9092 KAFKA_TOPIC=stellar-scp-messages npm run stream:kafka
```

Set `KAFKA_FROM_BEGINNING=true` to replay retained messages.

## Data Mapping

Snapshots accept `nodes` and `edges` matching the operator topology response. Individual SCP messages accept both snake_case and camelCase producer fields, including `node_id`, `nodeId`, `phase`, `ballot_counter`, `ballotCounter`, `quorum_set`, and `quorumSet`.

Node health colors:

- Green: synced / externalized
- Amber: degraded / prepare or confirm phase
- Red: stalled or unknown phase

Click a validator to inspect cluster, phase, ballot, TPS, ledger time, and quorum threshold. Orbit controls provide zoom, pan, and rotate. The toolbar includes source selection, pause/resume, live connection state, FPS, and heap usage when the browser exposes it.

## Checks

```bash
npm test
npm run build
```

The model tests cover snapshot normalization, message ingestion, edge de-duplication, deferred edge resolution, metric extraction, and status classification. Browser performance should be checked against a production preview with the mock stream; the target workload is 500 validators and 2,000 links.
