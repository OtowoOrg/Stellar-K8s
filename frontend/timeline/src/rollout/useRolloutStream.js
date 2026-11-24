import { useEffect, useRef, useState } from 'react';
import { deriveRolloutView } from './phases.js';
import { createSimulation } from './simulate.js';

export const EMPTY_SNAPSHOT = {
  revision: 0,
  nodeName: 'stellar-node',
  namespace: 'default',
  desiredReplicas: 0,
  strategy: 'RollingUpdate',
  image: { old: null, new: null },
  replicas: [],
  bottleneck: null,
};

// Poll cadence. Snapshots are drained at most once per animation frame
// regardless of how often the source emits, so a fast WebSocket stream or a
// chatty poller never causes render thrashing.
export const DEFAULT_POLL_INTERVAL_MS = 1500;

function streamUrl(source) {
  const protocol = window.location.protocol === 'https:' ? 'wss' : 'ws';
  if (source === 'ws') return `${protocol}://${window.location.host}/api/v1/rollout/stream`;
  return `${window.location.protocol}//${window.location.host}/api/v1/nodes/stellar-system/my-validator`;
}

/**
 * Best-effort normalization of the operator's node-detail response
 * (NodeDetailResponse with a StellarNodeStatus) into the rollout snapshot
 * shape. Payloads that already carry a `replicas` array pass through.
 */
export function normalizeApiSnapshot(payload) {
  if (Array.isArray(payload.replicas)) return payload;

  const status = payload.status ?? {};
  const count = Math.max(Number(status.replicas ?? payload.replicas ?? 1), 1);
  const ready = Math.min(Math.max(Number(status.ready_replicas ?? payload.readyReplicas ?? 0), 0), count);
  const syncing = /syncing|catch/i.test(String(status.phase ?? ''));
  const phase = syncing ? 'history-catchup' : 'fully-synced';

  return {
    revision: Date.now(),
    nodeName: payload.name ?? 'stellar-node',
    namespace: payload.namespace ?? 'default',
    desiredReplicas: count,
    strategy: 'RollingUpdate',
    image: { old: payload.version ?? null, new: null },
    replicas: Array.from({ length: count }, (_, ordinal) => ({
      ordinal,
      name: `${payload.name ?? 'stellar-node'}-${ordinal}`,
      image: payload.version ?? null,
      updated: false,
      phase,
      phaseProgress: syncing ? 0.5 : 1,
      containerStatus: ordinal < ready ? 'Ready' : 'Running',
      containerReady: ordinal < ready,
      restartCount: 0,
      phaseDetail: status.ledger_sequence
        ? { currentLedger: status.ledger_sequence, targetLedger: status.ledger_sequence }
        : null,
    })),
  };
}

/**
 * Subscribe to rollout telemetry. Supports three sources:
 *  - `simulation`: deterministic in-browser 3-pod rolling update (default;
 *    used for the acceptance demo and manual validation)
 *  - `rest`: polls the operator REST API on an interval
 *  - `ws`: listens on a WebSocket rollout stream
 *
 * Every snapshot is normalized through `deriveRolloutView` and flushed through
 * requestAnimationFrame, so at most one React render happens per animation
 * frame, and unchanged revisions are dropped entirely.
 */
export function useRolloutStream({
  source = 'simulation',
  pollIntervalMs = DEFAULT_POLL_INTERVAL_MS,
  replicaCount = 3,
  stuckOrdinal = 1,
}) {
  const [view, setView] = useState(() => deriveRolloutView(EMPTY_SNAPSHOT));
  const [connection, setConnection] = useState('connecting');
  const [error, setError] = useState(null);
  const simulationRef = useRef(null);
  const unstickRef = useRef(() => {});

  useEffect(() => {
    let disposed = false;
    let socket = null;
    let timer = null;
    let frame = null;
    let pending = null;

    const flush = () => {
      frame = null;
      if (disposed || pending === null) return;
      const next = pending;
      pending = null;
      setView((previous) => {
        if (previous && previous.revision === next.revision) return previous;
        return deriveRolloutView(next, previous);
      });
    };

    // Queue a snapshot; coalesce bursts into a single frame.
    const enqueue = (snapshot) => {
      pending = snapshot;
      if (frame === null) frame = requestAnimationFrame(flush);
    };

    const cleanup = () => {
      disposed = true;
      if (socket) socket.close();
      if (timer) clearInterval(timer);
      if (frame !== null) cancelAnimationFrame(frame);
      simulationRef.current = null;
    };

    if (source === 'simulation') {
      const simulation = createSimulation({ replicaCount, stuckOrdinal });
      simulationRef.current = simulation;
      unstickRef.current = simulation.unstick;
      enqueue(simulation.snapshot());
      setConnection('live');
      timer = setInterval(() => {
        simulation.tick();
        enqueue(simulation.snapshot());
      }, pollIntervalMs);
      return cleanup;
    }

    if (source === 'rest') {
      const poll = async () => {
        try {
          const response = await fetch(streamUrl(source));
          if (!response.ok) throw new Error(`HTTP ${response.status}`);
          enqueue(normalizeApiSnapshot(await response.json()));
          setConnection('live');
          setError(null);
        } catch (cause) {
          setConnection('error');
          setError(cause instanceof Error ? cause.message : String(cause));
        }
      };
      poll();
      timer = setInterval(poll, pollIntervalMs);
      return cleanup;
    }

    if (source === 'ws') {
      try {
        socket = new WebSocket(streamUrl(source));
        socket.onopen = () => setConnection('live');
        socket.onmessage = (event) => {
          try {
            enqueue(normalizeApiSnapshot(JSON.parse(event.data)));
            setError(null);
          } catch {
            setConnection('error');
          }
        };
        socket.onerror = () => setConnection('error');
        socket.onclose = () => { if (!disposed) setConnection('offline'); };
      } catch {
        setConnection('error');
      }
      return cleanup;
    }

    setConnection('error');
    setError(`Unknown data source: ${source}`);
    return cleanup;
  }, [source, pollIntervalMs, replicaCount, stuckOrdinal]);

  // Read through the ref so the returned function always targets the live
  // simulation instance, even before the effect has run.
  const unstick = () => unstickRef.current();

  return { view, connection, error, unstick };
}
