import { StrictMode, useState } from 'react';
import { createRoot } from 'react-dom/client';
import RolloutTracker from './rollout/RolloutTracker.jsx';
import { useRolloutStream } from './rollout/useRolloutStream.js';
import './styles.css';

const query = new URLSearchParams(window.location.search);
const sourceFromQuery = query.get('source');
const initialSource = sourceFromQuery === 'rest' || sourceFromQuery === 'ws' ? sourceFromQuery : 'simulation';
const initialReplicas = Math.min(Math.max(Number(query.get('replicas')) || 3, 1), 9);

function App() {
  const [source, setSource] = useState(initialSource);
  const [replicaCount, setReplicaCount] = useState(initialReplicas);
  const { view, connection, error, unstick } = useRolloutStream({ source, replicaCount });

  const sourceLabel = source === 'simulation' ? 'Simulated rollout' : source === 'rest' ? 'Operator REST poll' : 'Rollout WebSocket';
  const hasBottleneck = Boolean(view.bottleneck);

  return (
    <main className="app-shell">
      <header className="topbar">
        <div className="brand-block">
          <span className="eyebrow">STELLAR / ROLLOUTS</span>
          <h1>Rollout timeline</h1>
          <p>Stellar-specific initialization phases per replica during rolling updates.</p>
        </div>
        <div className="toolbar" role="toolbar" aria-label="Rollout tracker controls">
          <label className="select-wrap">
            <span>Data source</span>
            <select value={source} onChange={(event) => setSource(event.target.value)}>
              <option value="simulation">Simulated rollout (3 pods)</option>
              <option value="rest">Operator REST poll</option>
              <option value="ws">Rollout WebSocket</option>
            </select>
          </label>
          <label className="select-wrap">
            <span>Replicas</span>
            <select value={replicaCount} onChange={(event) => setReplicaCount(Number(event.target.value))}>
              {[1, 2, 3, 4, 5, 6, 7, 8, 9].map((count) => (
                <option key={count} value={count}>{count}</option>
              ))}
            </select>
          </label>
          <button className="tool-button" type="button" disabled={!hasBottleneck} onClick={unstick} title="Release the stuck replica so the rollout can finish">
            Resume stuck replica
          </button>
        </div>
      </header>

      <section className="metric-strip" aria-label="Rollout summary">
        <Metric label="Node" value={view.nodeName} detail={view.namespace} />
        <Metric label="Strategy" value={view.strategy} detail={view.image.new ? `${view.image.old} → ${view.image.new}` : '—'} />
        <Metric label="Updated" value={view.replicas.filter((r) => r.updated).length} detail={`of ${view.desiredReplicas} replicas`} tone="cyan" />
        <Metric label="Ready" value={view.replicas.filter((r) => r.containerReady).length} detail="container readiness" tone="green" />
      </section>

      <section className="workspace">
        <div className="tracker-panel">
          <div className="panel-heading">
            <div>
              <span className={`status-dot ${connection}`} />
              <strong>{sourceLabel}</strong>
              <span className="muted">{error ? `poll failed: ${error}` : 'streaming every 1.5s, rendered once per frame'}</span>
            </div>
            <span className="muted">Step 1: Schema · Step 2: Catchup · Step 3: Peering · Step 4: Synced</span>
          </div>
          <RolloutTracker view={view} />
        </div>
      </section>
    </main>
  );
}

function Metric({ label, value, detail, tone }) {
  return (
    <div className="metric">
      <span className="metric-label">{label}</span>
      <strong className={tone ? `tone-${tone}` : ''}>{value}</strong>
      <span className="muted">{detail}</span>
    </div>
  );
}

createRoot(document.getElementById('root')).render(<StrictMode><App /></StrictMode>);
