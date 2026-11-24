// Human-readable diagnostic messages for pods that block or gate a StellarNode
// rolling update. The tracker highlights blocked pods and renders exactly one
// message per replica, derived from the normalized rollout view.

const isCrashLoop = (replica) => /crashloop|error/i.test(replica.containerStatus);

/**
 * Derive a single human-readable diagnostic for a replica, or null when the
 * replica is healthy and unblocked. Priority order:
 *   1. crash-looping container
 *   2. stalled catchup (the classic rollout hang)
 *   3. waiting on a higher-ordinal gate
 *   4. active phase explanations (migration / catchup / peering)
 *   5. fully synced but readiness probe failing
 */
export function diagnoseReplica(replica) {
  if (replica.containerReady && !replica.blocked) return null;

  if (isCrashLoop(replica)) {
    return `Container is ${replica.containerStatus} after ${replica.restartCount} restart${replica.restartCount === 1 ? '' : 's'}. Inspect previous logs: \`kubectl logs ${replica.name} --previous\` — a crash here is usually a seed or quorum-set misconfiguration.`;
  }

  if (replica.stalled) {
    const detail = replica.phaseDetail?.currentLedger
      ? `stuck at ledger ${replica.phaseDetail.currentLedger.toLocaleString()}`
      : `stuck at ${Math.round(replica.phaseProgress * 100)}%`;
    return `${phaseLabel(replica.phase)} is stalled — ${detail} with no progress for several samples. Check history archive reachability, PVC throughput, and CATCHUP_* settings.`;
  }

  if (replica.blockedBy) {
    return `Waiting for ${replica.blockedBy.name} to become Ready before this pod is rolled (StatefulSet reverse-ordinal update gate). ${replica.name} is still on ${replica.image ?? 'the previous image'}.`;
  }

  switch (replica.phase) {
    case 'database-schema-migration':
      return `Database schema migration ${percent(replica.phaseProgress)} complete — the init container must finish before stellar-core starts. Check for migration lock contention or a hung \`db migrate\`.`;
    case 'history-catchup': {
      const { currentLedger, targetLedger } = replica.phaseDetail ?? {};
      if (currentLedger != null && targetLedger != null) {
        return `Catching up: ledger ${currentLedger.toLocaleString()} → ${targetLedger.toLocaleString()} (${percent(replica.phaseProgress)}). The node cannot serve traffic or join consensus until it reaches the live tip.`;
      }
      return `History catchup in progress (${percent(replica.phaseProgress)}). This is usually the longest phase of a StellarNode rollout.`;
    }
    case 'quorum-peering':
      return `Catchup finished; node is peering with known peers and forming its quorum set (${percent(replica.phaseProgress)}). Check KNOWN_PEERS and quorum-set configuration if this phase never completes.`;
    case 'fully-synced':
      return `Node reports fully synced but the container readiness probe is failing. Verify the stellar-core /info endpoint and probe settings.`;
    default:
      return null;
  }
}

function phaseLabel(phaseId) {
  return phaseId
    .split('-')
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ');
}

function percent(value) {
  return `${Math.round((value ?? 0) * 100)}%`;
}
