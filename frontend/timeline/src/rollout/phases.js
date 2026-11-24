// Stellar-specific initialization phases shown per replica during a rolling
// update. Standard Kubernetes UIs only expose raw container status (Pending,
// Running, CrashLoopBackOff...). These micro-phases describe what stellar-core
// is actually doing inside the container: schema migration, history catchup,
// quorum peering, then finally serving live ledgers.
//
// The order is fixed: a node cannot peer until it has caught up, and it cannot
// catch up until the database schema exists.

export const PHASES = [
  {
    id: 'database-schema-migration',
    label: 'Database Schema Migration',
    shortLabel: 'Schema',
    step: 1,
    tone: 'blue',
    description: 'Init container runs DB migrations (e.g. `horizon db migrate`) before the main process starts.',
  },
  {
    id: 'history-catchup',
    label: 'History Catchup',
    shortLabel: 'Catchup',
    step: 2,
    tone: 'amber',
    description: 'stellar-core replays ledgers from history archives to reach the live tip. This is usually the longest phase.',
  },
  {
    id: 'quorum-peering',
    label: 'Quorum Peering',
    shortLabel: 'Peering',
    step: 3,
    tone: 'cyan',
    description: 'Node finished catchup and is connecting to known peers / building its quorum set.',
  },
  {
    id: 'fully-synced',
    label: 'Fully Synced',
    shortLabel: 'Synced',
    step: 4,
    tone: 'green',
    description: 'Node externalizes live ledgers, joins consensus, and is ready to serve traffic.',
  },
];

export const PHASE_INDEX = Object.fromEntries(PHASES.map((phase, index) => [phase.id, index]));

/** 0-based index of a phase id, or -1 when unknown. */
export function phaseIndex(id) {
  return PHASE_INDEX[id] ?? -1;
}

export function clamp01(value) {
  if (!Number.isFinite(value)) return 0;
  return Math.min(Math.max(value, 0), 1);
}

/**
 * Derive the renderable rollout view from a raw snapshot.
 *
 * `previousView` is the view produced for the previous revision. It is used to
 * detect stalls (a phase that makes no forward progress across samples) so the
 * UI can flag a pod whose catchup has silently frozen — the classic reason a
 * StellarNode rollout hangs.
 *
 * The derivation is pure and cheap: one pass over the replica list plus one
 * pass to resolve update gates. The hook that feeds it batches revisions, so
 * this runs at most once per animation frame.
 */
export function deriveRolloutView(snapshot, previousView = null) {
  const previousByOrdinal = new Map((previousView?.replicas ?? []).map((replica) => [replica.ordinal, replica]));

  const replicas = (snapshot.replicas ?? []).map((raw) => {
    const previous = previousByOrdinal.get(raw.ordinal);
    const phase = PHASE_INDEX[raw.phase] !== undefined ? raw.phase : 'fully-synced';
    const phaseIdx = phaseIndex(phase);
    const phaseProgress = clamp01(raw.phaseProgress);
    const stallSamples = detectStall(previous, phase, phaseProgress);
    const stalled = stallSamples >= STALL_THRESHOLD_SAMPLES;
    const overallProgress = phaseIdx < 0 ? 0 : clamp01((phaseIdx + phaseProgress) / PHASES.length);

    return {
      ordinal: raw.ordinal,
      name: raw.name,
      image: raw.image,
      updated: Boolean(raw.updated),
      phase,
      phaseIdx,
      phaseProgress,
      stallSamples,
      stalled,
      overallProgress,
      containerStatus: raw.containerStatus ?? 'Unknown',
      containerReady: Boolean(raw.containerReady),
      restartCount: raw.restartCount ?? 0,
      phaseDetail: raw.phaseDetail ?? null,
      blockedBy: null,
      blocked: false,
      bottleneck: false,
      diagnostic: null,
    };
  });

  // Resolve StatefulSet update gates. Pods roll in reverse ordinal order and
  // each pod waits for the next-higher ordinal to be Ready. An updated pod that
  // is not Ready gates every lower ordinal; the lowest such pod is the
  // bottleneck the whole rollout waits on.
  const gate = replicas
    .filter((replica) => replica.updated && !replica.containerReady)
    .sort((a, b) => a.ordinal - b.ordinal)[0];

  for (const replica of replicas) {
    const blocker = replicas.find((candidate) => candidate.ordinal > replica.ordinal && candidate.updated && !candidate.containerReady);
    if (blocker) {
      replica.blocked = true;
      replica.blockedBy = blocker;
    }
    replica.bottleneck = gate ? gate.ordinal === replica.ordinal : false;
  }

  return {
    revision: snapshot.revision ?? 0,
    nodeName: snapshot.nodeName ?? 'stellar-node',
    namespace: snapshot.namespace ?? 'default',
    desiredReplicas: snapshot.desiredReplicas ?? replicas.length,
    strategy: snapshot.strategy ?? 'RollingUpdate',
    image: snapshot.image ?? { old: null, new: null },
    replicas,
    bottleneck: gate ?? null,
  };
}

/**
 * Stall detection: the same phase making zero forward progress across samples.
 * Returns a running counter of consecutive unchanged samples (0 = moving). A
 * fresh phase start is not a stall; neither is a ready pod. With the default
 * 1.5s poll interval, ~7.5s of frozen progress crosses the threshold.
 */
export const STALL_THRESHOLD_SAMPLES = 5;

function detectStall(previous, phase, phaseProgress) {
  if (!previous) return 0;
  if (phase === 'fully-synced') return 0;
  if (previous.phase !== phase) return 0;
  return previous.phaseProgress >= phaseProgress ? (previous.stallSamples ?? 0) + 1 : 0;
}

/** Per-phase step state for the stepper: done | active | blocked | pending. */
export function stepStates(replica) {
  return PHASES.map((phase, index) => {
    if (index < replica.phaseIdx) return 'done';
    if (index > replica.phaseIdx) return 'pending';
    if (replica.stalled) return 'blocked';
    return replica.blocked ? 'blocked' : 'active';
  });
}

/** Human label for the current overall state of a replica's rollout. */
export function replicaStatus(replica) {
  // A ready pod that is gated by a higher ordinal still reads as waiting — it
  // is not eligible to roll until the gate releases.
  if (replica.bottleneck) return 'bottleneck';
  if (replica.blocked) return 'gated';
  if (replica.containerReady && replica.phase === 'fully-synced') return 'ready';
  return 'rolling';
}
