// Deterministic simulation of a StellarNode StatefulSet rolling update.
//
// Drives the same reverse-ordinal semantics as the real StatefulSet controller:
// pods update highest-ordinal-first and each pod waits for the next-higher
// ordinal to become Ready. The default scenario matches the acceptance
// validation: a 3-pod rollout where pod #1 freezes in History Catchup, gating
// pod #0 and isolating the bottleneck step for the UI.
//
// The simulation is tick-based (no timers) so tests can fast-forward the whole
// rollout deterministically. The UI drives it with setInterval; the hook
// batches each snapshot through requestAnimationFrame.

export const OLD_IMAGE = 'stellar/core:20.4.0';
export const NEW_IMAGE = 'stellar/core:20.5.0';
export const TARGET_LEDGER = 5142300;

// Number of ticks each phase takes to complete.
const PHASE_TICKS = {
  'database-schema-migration': 3,
  'history-catchup': 14,
  'quorum-peering': 3,
};

function makePod(ordinal) {
  return {
    ordinal,
    name: `my-validator-${ordinal}`,
    image: OLD_IMAGE,
    updated: false,
    phase: 'fully-synced',
    phaseTicks: 0,
    phaseProgress: 1,
    ready: true,
    containerStatus: 'Ready',
    restartCount: 0,
    startLedger: TARGET_LEDGER,
    currentLedger: TARGET_LEDGER,
    targetLedger: TARGET_LEDGER,
  };
}

/**
 * Create a simulation. Options:
 *  - replicaCount: pods in the StatefulSet (default 3)
 *  - stuckOrdinal: pod frozen mid-catchup (default 1), or -1 for no stall
 *  - catchupFreezeAt: ledger progress fraction where the stuck pod freezes
 */
export function createSimulation({ replicaCount = 3, stuckOrdinal = 1, catchupFreezeAt = 0.53 } = {}) {
  const pods = Array.from({ length: replicaCount }, (_, ordinal) => makePod(ordinal));
  let tickCount = 0;
  let released = false;

  function tick() {
    tickCount += 1;

    // 1. Start the next roll: the highest not-yet-updated pod whose upper
    //    neighbour is Ready (or which has no upper neighbour).
    for (let i = pods.length - 1; i >= 0; i -= 1) {
      const pod = pods[i];
      if (pod.updated) continue;
      if (!(pod.phase === 'fully-synced' && pod.ready)) continue;
      // Gate: pod i may only roll once the next-higher ordinal has finished
      // its own update and become Ready (StatefulSet reverse-ordinal semantics).
      const above = pods[i + 1];
      if (above && !(above.updated && above.ready)) continue;
      pod.image = NEW_IMAGE;
      pod.updated = true;
      pod.phase = 'database-schema-migration';
      pod.phaseProgress = 0;
      pod.ready = false;
      pod.containerStatus = 'Init:0/1';
      pod.startLedger = Math.max(TARGET_LEDGER - 19000 - pod.ordinal * 1000, 0);
      pod.currentLedger = pod.startLedger;
    }

    // 2. Advance every in-flight phase. Progress is tick-counter based so a
    //    phase completes on exactly its configured number of ticks (no
    //    floating-point drift from repeated 1/n addition).
    for (const pod of pods) {
      if (!pod.updated || pod.ready) continue;

      if (pod.phase === 'history-catchup' && pod.ordinal === stuckOrdinal && !released) {
        if (pod.phaseTicks / PHASE_TICKS[pod.phase] >= catchupFreezeAt) {
          pod.phaseProgress = catchupFreezeAt; // frozen — no forward progress
          continue;
        }
      }

      pod.phaseTicks += 1;
      const total = PHASE_TICKS[pod.phase];
      pod.phaseProgress = Math.min(pod.phaseTicks / total, 1);

      if (pod.phase === 'database-schema-migration' && pod.phaseTicks >= total) {
        pod.phase = 'history-catchup';
        pod.phaseTicks = 0;
        pod.phaseProgress = 0;
        pod.containerStatus = 'Running';
      } else if (pod.phase === 'history-catchup' && pod.phaseTicks >= total) {
        pod.phase = 'quorum-peering';
        pod.phaseTicks = 0;
        pod.phaseProgress = 0;
        pod.currentLedger = pod.targetLedger;
      } else if (pod.phase === 'quorum-peering' && pod.phaseTicks >= total) {
        pod.phase = 'fully-synced';
        pod.phaseTicks = 0;
        pod.phaseProgress = 1;
        pod.ready = true;
        pod.containerStatus = 'Ready';
      }

      if (pod.phase === 'history-catchup') {
        // Ledger display is derived from phase progress so the two bars agree.
        const span = pod.targetLedger - pod.startLedger;
        pod.currentLedger = Math.round(pod.targetLedger - (1 - pod.phaseProgress) * span);
      }
    }
  }

  /** Release the stuck pod so the rollout can finish (used by the demo UI). */
  function unstick() {
    released = true;
  }

  function snapshot() {
    return {
      revision: tickCount,
      nodeName: 'my-validator',
      namespace: 'stellar-system',
      desiredReplicas: pods.length,
      strategy: 'RollingUpdate',
      image: { old: OLD_IMAGE, new: NEW_IMAGE },
      replicas: pods.map((pod) => ({
        ordinal: pod.ordinal,
        name: pod.name,
        image: pod.image,
        updated: pod.updated,
        phase: pod.phase,
        phaseProgress: pod.phaseProgress,
        phaseDetail:
          pod.phase === 'history-catchup' || pod.phase === 'fully-synced'
            ? { currentLedger: pod.currentLedger, targetLedger: pod.targetLedger }
            : null,
        containerStatus: pod.containerStatus,
        containerReady: pod.ready,
        restartCount: pod.restartCount,
      })),
    };
  }

  return { tick, unstick, snapshot, get tickCount() { return tickCount; } };
}

/**
 * Run a simulation to completion (or until a stuck pod blocks it), returning
 * the sequence of raw snapshots. Used by tests and available for tooling that
 * wants to replay a recorded rollout.
 */
export function runSimulation(options = {}, maxTicks = 120) {
  const simulation = createSimulation(options);
  const frames = [];
  for (let i = 0; i < maxTicks; i += 1) {
    simulation.tick();
    frames.push(simulation.snapshot());
  }
  return { frames, simulation };
}
