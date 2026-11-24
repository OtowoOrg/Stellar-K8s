# Stellar Rollout Timeline

A standalone React SPA that visualizes **Stellar-specific initialization
phases** for each replica of a `StellarNode` StatefulSet during a rolling
update.

Standard Kubernetes UIs only show raw container status (`Pending`, `Running`,
`CrashLoopBackOff`…) while a node comes up. This tracker overlays the
micro-phases that actually gate a Stellar node's availability:

```
Database Schema Migration → History Catchup → Quorum Peering → Fully Synced
```

Each replica card shows the phase stepper, custom progress bars for internal
catch-up status (ledger numbers included), the raw Kubernetes container status,
and — when a pod blocks or gates the rollout — a highlighted human-readable
diagnostic.

The layout is inspired by the Argo Rollouts visualizer: one card per workload
replica with status badges, step lists, and a banner that isolates the rollout
bottleneck.

## Run It

From `frontend/timeline`:

```bash
npm install
npm run dev
```

Open the Vite URL (default `http://localhost:5175`). The default data source is
the deterministic in-browser simulation:

- **Simulated rollout (3 pods)** — a 3-replica `StellarNode` rolling update.
  Pod `my-validator-2` rolls first (reverse ordinal order), then
  `my-validator-1` freezes in **History Catchup** at 53%. The tracker highlights
  it as the bottleneck, and `my-validator-0` shows as **WAITING** behind the
  StatefulSet update gate. Click **Resume stuck replica** to release the gate
  and watch the rollout complete.

Other sources:

- **Operator REST poll** — polls `GET /api/v1/nodes/…` every 1.5s. Point it at
  a rollout payload with a `replicas` array (see *Data shape* below).
- **Rollout WebSocket** — listens on `/api/v1/rollout/stream` for the same
  payload shape.

URL parameters: `?source=rest|ws|simulation&replicas=3`.

## Rendering Efficiency

The `useRolloutStream` hook decouples ingestion from rendering:

- WebSocket frames / poll responses are queued into a single pending snapshot.
- At most **one React render per animation frame** (drained via
  `requestAnimationFrame`), regardless of how chatty the source is.
- Snapshots with an unchanged `revision` are dropped without re-rendering.

This prevents the render thrashing a naive `setState`-per-message approach
would cause during a fast catchup stream.

## Data shape

Polled/WebSocket sources should emit snapshots in this normalized shape
(`simulate.js` produces exactly this):

```js
{
  revision: 42,                 // bump to force a re-render
  nodeName: 'my-validator',
  namespace: 'stellar-system',
  desiredReplicas: 3,
  strategy: 'RollingUpdate',
  image: { old: 'stellar/core:20.4.0', new: 'stellar/core:20.5.0' },
  replicas: [
    {
      ordinal: 1,
      name: 'my-validator-1',
      image: 'stellar/core:20.5.0',
      updated: true,
      phase: 'history-catchup',           // one of the four phase ids
      phaseProgress: 0.53,                // 0..1 within the phase
      phaseDetail: { currentLedger: 5132900, targetLedger: 5142300 },
      containerStatus: 'Running',         // raw Kubernetes status
      containerReady: false,
      restartCount: 0,
    },
  ],
}
```

## Validation: simulated rollout with a stuck catchup

The acceptance scenario — *3-pod rolling update where Pod #1 is stuck in
History Catchup* — is exercised both by the UI simulation and by unit tests
(`npm test`):

1. `my-validator-2` rolls through Schema → Catchup → Peering → Synced.
2. `my-validator-1` reaches **History Catchup**, freezes at 53% for several
   samples, and is flagged **BLOCKED** with the diagnostic:
   > History Catchup is stalled — stuck at ledger 5,132,900 with no progress
   > for several samples. Check history archive reachability, PVC throughput,
   > and CATCHUP_* settings.
3. `my-validator-0` stays on the previous image, marked **WAITING**:
   > Waiting for my-validator-1 to become Ready before this pod is rolled
   > (StatefulSet reverse-ordinal update gate).
4. The banner isolates the bottleneck: *ROLLOUT BLOCKED —
   `my-validator-1` stuck in History Catchup at 53%*.

Press **Resume stuck replica** (or call `simulation.unstick()`) and the gate
releases: catchup resumes, `my-validator-1` peers and syncs, and
`my-validator-0` rolls last.

## Checks

```bash
npm test
npm run build
```

`rollout.test.js` covers phase derivation, stall detection, diagnostics, and
the full stuck-catchup simulation lifecycle (including the unstick path).

## Recording a demo

For PR review, run the app with the simulated source and screen-record ~60
seconds: the rollout starts immediately, `my-validator-2` syncs, then the
tracker isolates `my-validator-1` stuck in History Catchup before you click
**Resume stuck replica** and watch the update complete.
