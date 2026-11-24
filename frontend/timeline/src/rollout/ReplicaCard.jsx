import PhaseStepper from './PhaseStepper.jsx';
import ProgressBar from './ProgressBar.jsx';
import { PHASES, replicaStatus } from './phases.js';
import { diagnoseReplica } from './diagnostics.js';

const STATUS_LABEL = {
  ready: 'READY',
  bottleneck: 'BLOCKED',
  gated: 'WAITING',
  rolling: 'ROLLING',
};

/**
 * One replica in the StatefulSet. Shows:
 *  - raw Kubernetes container status alongside the Stellar-specific stepper
 *  - custom progress bars for overall init progress and ledger catch-up
 *  - a highlighted diagnostic when the pod blocks or gates the rollout
 */
export default function ReplicaCard({ replica }) {
  const status = replicaStatus(replica);
  const diagnostic = diagnoseReplica(replica);
  const phase = PHASES[replica.phaseIdx];
  const phasePercent = Math.round(replica.phaseProgress * 100);
  const { currentLedger, targetLedger } = replica.phaseDetail ?? {};

  return (
    <article className={`replica-card ${replica.bottleneck ? 'card-bottleneck' : ''} ${replica.blocked ? 'card-gated' : ''}`}>
      <header className="replica-head">
        <div className="replica-id">
          <code className="pod-name">{replica.name}</code>
          <span className="muted">{replica.updated ? 'updated' : 'previous image'}</span>
        </div>
        <div className="replica-badges">
          <span className={`status-badge status-${status}`}>{STATUS_LABEL[status]}</span>
          <span className="container-chip" title="Raw Kubernetes container status">
            {replica.containerStatus}
            {replica.restartCount > 0 ? ` ×${replica.restartCount}` : ''}
          </span>
        </div>
      </header>

      <PhaseStepper replica={replica} />

      <div className="replica-progress">
        <ProgressBar
          value={replica.overallProgress}
          label="Stellar init"
          detail={phase ? `${phase.shortLabel} ${phasePercent}%` : null}
          tone={replica.stalled ? 'red' : 'default'}
        />
        {replica.phase === 'history-catchup' || replica.phase === 'fully-synced' ? (
          <ProgressBar
            value={replica.phaseProgress}
            label="Ledger catch-up"
            detail={currentLedger != null && targetLedger != null ? `${currentLedger.toLocaleString()} / ${targetLedger.toLocaleString()}` : null}
            tone={replica.stalled ? 'red' : 'amber'}
          />
        ) : (
          <ProgressBar value={replica.phaseProgress} label={phase?.label ?? 'Phase'} detail={phasePercent > 0 ? `${phasePercent}%` : null} tone="default" />
        )}
      </div>

      {diagnostic ? (
        <p className={`diagnostic ${replica.bottleneck ? 'diagnostic-red' : replica.blocked ? 'diagnostic-grey' : 'diagnostic-amber'}`} role="status">
          <span className="diagnostic-icon">{replica.bottleneck ? '⛔' : replica.blocked ? '⏸' : '⏳'}</span>
          {diagnostic}
        </p>
      ) : (
        <p className="diagnostic diagnostic-ok" role="status">
          <span className="diagnostic-icon">✓</span>
          Fully synced and Ready on {replica.image}.
        </p>
      )}
    </article>
  );
}
