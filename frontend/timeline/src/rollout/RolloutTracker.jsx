import ReplicaCard from './ReplicaCard.jsx';
import { PHASES } from './phases.js';

/**
 * Top-level rollout tracker: summary metrics, a bottleneck banner when a pod
 * gates the rollout, and one ReplicaCard per StatefulSet replica. The layout
 * mirrors Argo Rollouts' visualizer — a card per workload with status and
 * step-by-step progress.
 */
export default function RolloutTracker({ view }) {
  const { replicas, bottleneck } = view;
  const updated = replicas.filter((replica) => replica.updated).length;
  const ready = replicas.filter((replica) => replica.containerReady).length;
  const blocked = replicas.filter((replica) => replica.blocked || replica.bottleneck).length;
  const gatedCount = replicas.filter((replica) => replica.blocked).length;
  const bottleneckPhase = bottleneck ? PHASES[bottleneck.phaseIdx] : null;

  return (
    <section className="tracker" aria-label="Rollout tracker">
      <div className="rollout-summary">
        <Summary label="Desired" value={view.desiredReplicas} />
        <Summary label="Updated" value={updated} tone="cyan" />
        <Summary label="Ready" value={ready} tone="green" />
        <Summary label="Blocked" value={blocked} tone={blocked > 0 ? 'red' : 'green'} />
      </div>

      {bottleneck && bottleneckPhase ? (
        <div className="bottleneck-banner" role="alert">
          <span className="bottleneck-title">ROLLOUT BLOCKED</span>
          <span>
            <code>{bottleneck.name}</code> is stuck in <strong>{bottleneckPhase.label}</strong> at{' '}
            {Math.round(bottleneck.phaseProgress * 100)}% — the update gate holds {gatedCount} pod{gatedCount === 1 ? '' : 's'} behind it.
          </span>
        </div>
      ) : null}

      <div className="replica-grid">
        {replicas.map((replica) => (
          <ReplicaCard key={replica.ordinal} replica={replica} />
        ))}
      </div>
    </section>
  );
}

function Summary({ label, value, tone }) {
  return (
    <div className="summary-cell">
      <span className="muted">{label}</span>
      <strong className={tone ? `tone-${tone}` : ''}>{value}</strong>
    </div>
  );
}
