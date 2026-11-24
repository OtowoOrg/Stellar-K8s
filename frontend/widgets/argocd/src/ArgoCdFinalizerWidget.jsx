/**
 * ArgoCdFinalizerWidget.jsx
 *
 * React widget that polls the ArgoCD API and renders:
 *  - Per-application sync/health status cards
 *  - A detailed list of resources stuck in Terminating due to Finalizers
 *  - Contextual resolution hints for each stuck resource
 */

import { useCallback, useEffect, useReducer, useRef, useState } from 'react';
import { ArgoCdPoller, parseAppState } from './argoCdParser.js';

// ── Simulated ArgoCD response for demonstration & testing ───────────────────

/** Built-in mock data that exercises every code path in the parser. */
export const MOCK_ARGO_RESPONSE = {
  items: [
    {
      metadata: { name: 'stellar-mainnet' },
      status: {
        sync: { status: 'OutOfSync' },
        health: { status: 'Degraded' },
        resources: [
          {
            kind: 'StellarNode',
            name: 'sn-mainnet-0',
            namespace: 'stellar-system',
            syncStatus: 'OutOfSync',
            phase: 'Draining',
            deletionTimestamp: new Date(Date.now() - 8 * 60_000).toISOString(),
            finalizers: [
              'stellarnode.k8s.stellar.org/network-drain',
              'stellarnode.k8s.stellar.org/pv-cleanup',
            ],
            children: [
              {
                kind: 'Pod',
                name: 'validator-mainnet-0',
                namespace: 'stellar-system',
                syncStatus: 'OutOfSync',
                deletionTimestamp: new Date(Date.now() - 7 * 60_000).toISOString(),
                finalizers: ['stellarnode.k8s.stellar.org/peer-deregister'],
              },
              {
                kind: 'PersistentVolumeClaim',
                name: 'data-mainnet-0',
                namespace: 'stellar-system',
                syncStatus: 'OutOfSync',
                deletionTimestamp: new Date(Date.now() - 6 * 60_000).toISOString(),
                finalizers: ['kubernetes.io/pvc-protection'],
              },
            ],
          },
          {
            kind: 'StellarNode',
            name: 'sn-mainnet-1',
            namespace: 'stellar-system',
            syncStatus: 'Synced',
            finalizers: [],
          },
          {
            kind: 'Pod',
            name: 'validator-mainnet-1',
            namespace: 'stellar-system',
            syncStatus: 'Synced',
            finalizers: [],
          },
          {
            kind: 'Service',
            name: 'stellar-rpc',
            namespace: 'stellar-system',
            syncStatus: 'Synced',
            finalizers: [],
          },
        ],
      },
    },
    {
      metadata: { name: 'stellar-testnet' },
      status: {
        sync: { status: 'Synced' },
        health: { status: 'Healthy' },
        resources: [
          {
            kind: 'StellarNode',
            name: 'sn-testnet-0',
            namespace: 'stellar-testnet',
            syncStatus: 'Synced',
            finalizers: [],
          },
          {
            kind: 'StellarNode',
            name: 'sn-testnet-1',
            namespace: 'stellar-testnet',
            syncStatus: 'Synced',
            finalizers: [],
          },
          {
            kind: 'Pod',
            name: 'validator-testnet-0',
            namespace: 'stellar-testnet',
            syncStatus: 'Synced',
            finalizers: [],
          },
        ],
      },
    },
  ],
};

// ── State reducer ────────────────────────────────────────────────────────────

const INITIAL_STATE = {
  apps: [],
  loading: true,
  error: null,
  lastUpdated: null,
};

function reducer(state, action) {
  switch (action.type) {
    case 'UPDATE':
      return { ...state, apps: action.apps, loading: false, error: null, lastUpdated: new Date() };
    case 'ERROR':
      return { ...state, loading: false, error: action.message, lastUpdated: new Date() };
    case 'RESET':
      return INITIAL_STATE;
    default:
      return state;
  }
}

// ── Main widget component ────────────────────────────────────────────────────

/**
 * @param {object} props
 * @param {string} [props.argoCdBaseUrl]  — ArgoCD API base, e.g. 'https://argocd.example.com'
 * @param {string} [props.token]          — ArgoCD bearer token
 * @param {number} [props.pollIntervalMs] — polling interval (default 10 000)
 * @param {'live'|'mock'} [props.mode]    — 'mock' uses built-in demo data
 */
export default function ArgoCdFinalizerWidget({
  argoCdBaseUrl = '',
  token = '',
  pollIntervalMs = 10_000,
  mode = 'live',
}) {
  const [state, dispatch] = useReducer(reducer, INITIAL_STATE);
  const [selectedApp, setSelectedApp] = useState(null);
  const [expandedHints, setExpandedHints] = useState(new Set());
  const pollerRef = useRef(null);

  const toggleHint = useCallback((key) => {
    setExpandedHints((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);

  useEffect(() => {
    dispatch({ type: 'RESET' });

    if (mode === 'mock') {
      const apps = (MOCK_ARGO_RESPONSE.items ?? []).map(parseAppState);
      dispatch({ type: 'UPDATE', apps });
      if (apps.length > 0) setSelectedApp(apps[0].appName);
      return;
    }

    const poller = new ArgoCdPoller({
      baseUrl: argoCdBaseUrl,
      token,
      intervalMs: pollIntervalMs,
      onUpdate: (apps) => {
        dispatch({ type: 'UPDATE', apps });
        if (apps.length > 0 && !selectedApp) setSelectedApp(apps[0].appName);
      },
      onError: (err) => dispatch({ type: 'ERROR', message: err.message }),
    });
    pollerRef.current = poller;
    poller.start();
    return () => poller.stop();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, argoCdBaseUrl, token, pollIntervalMs]);

  const currentApp = state.apps.find((a) => a.appName === selectedApp) ?? state.apps[0] ?? null;
  const stuckTotal = state.apps.reduce((n, a) => n + a.terminatingResources.length, 0);

  return (
    <div className="argo-widget">
      {/* ── Header ── */}
      <header className="argo-header">
        <div className="argo-brand">
          <span className="argo-eyebrow">STELLAR / ARGOCD</span>
          <h1>Sync Status &amp; Finalizer Tracker</h1>
          <p>Monitor StellarNode lifecycle locks blocking ArgoCD synchronization.</p>
        </div>
        <div className="argo-header-meta">
          {state.lastUpdated && (
            <span className="argo-updated">
              Updated {state.lastUpdated.toLocaleTimeString()}
            </span>
          )}
          {mode === 'mock' && <span className="argo-badge badge-mock">MOCK DATA</span>}
          {state.loading && <span className="argo-badge badge-loading">Loading…</span>}
          {state.error && <span className="argo-badge badge-error" title={state.error}>API Error</span>}
        </div>
      </header>

      {/* ── Global summary strip ── */}
      <section className="argo-summary-strip" aria-label="Overall summary">
        <SummaryTile label="Applications" value={state.apps.length} />
        <SummaryTile
          label="Stuck (Finalizers)"
          value={stuckTotal}
          tone={stuckTotal > 0 ? 'red' : 'green'}
        />
        <SummaryTile
          label="Synced"
          value={state.apps.filter((a) => a.syncStatus === 'Synced').length}
          tone="green"
        />
        <SummaryTile
          label="Out of Sync"
          value={state.apps.filter((a) => a.syncStatus !== 'Synced' && a.syncStatus !== 'Unknown').length}
          tone={state.apps.some((a) => a.syncStatus === 'OutOfSync') ? 'amber' : 'green'}
        />
      </section>

      {/* ── Application list + detail ── */}
      <div className="argo-workspace">
        {/* App list sidebar */}
        <nav className="argo-app-list" aria-label="ArgoCD applications">
          <span className="argo-panel-label">Applications</span>
          {state.apps.length === 0 && !state.loading && (
            <p className="argo-empty-note">No applications found.</p>
          )}
          {state.apps.map((app) => (
            <AppListItem
              key={app.appName}
              app={app}
              isSelected={app.appName === selectedApp}
              onClick={() => setSelectedApp(app.appName)}
            />
          ))}
        </nav>

        {/* Detail panel */}
        <main className="argo-detail-panel">
          {currentApp ? (
            <AppDetail
              app={currentApp}
              expandedHints={expandedHints}
              onToggleHint={toggleHint}
            />
          ) : (
            <div className="argo-empty-state">
              <div className="argo-empty-icon">⟳</div>
              <h2>Waiting for ArgoCD data…</h2>
              <p>Connect to an ArgoCD instance or switch to mock mode.</p>
            </div>
          )}
        </main>
      </div>
    </div>
  );
}

// ── Sub-components ────────────────────────────────────────────────────────────

function SummaryTile({ label, value, tone }) {
  return (
    <div className="argo-summary-tile">
      <span className="argo-tile-label">{label}</span>
      <strong className={tone ? `tone-${tone}` : ''}>{value}</strong>
    </div>
  );
}

function AppListItem({ app, isSelected, onClick }) {
  const hasLock = app.isStuck;
  return (
    <button
      type="button"
      className={`argo-app-item ${isSelected ? 'is-selected' : ''} ${hasLock ? 'has-lock' : ''}`}
      onClick={onClick}
      aria-current={isSelected}
    >
      <span className="app-item-name">{app.appName}</span>
      <span className="app-item-badges">
        <SyncBadge status={app.syncStatus} />
        <HealthBadge status={app.healthStatus} />
        {hasLock && (
          <span className="lock-badge" title={`${app.terminatingResources.length} stuck finalizer(s)`}>
            🔒 {app.terminatingResources.length}
          </span>
        )}
      </span>
    </button>
  );
}

function AppDetail({ app, expandedHints, onToggleHint }) {
  return (
    <div className="argo-app-detail">
      {/* App header */}
      <div className="detail-heading">
        <h2>{app.appName}</h2>
        <div className="detail-badges">
          <SyncBadge status={app.syncStatus} large />
          <HealthBadge status={app.healthStatus} large />
        </div>
      </div>

      {/* Resource counts */}
      <div className="detail-counts">
        <CountChip label="Total resources" value={app.totalResources} />
        <CountChip label="Synced" value={app.syncedCount} tone="green" />
        <CountChip label="Out of sync" value={app.outOfSyncCount} tone="amber" />
        <CountChip
          label="Terminating (locked)"
          value={app.terminatingResources.length}
          tone={app.isStuck ? 'red' : 'green'}
        />
      </div>

      {/* Finalizer lock list */}
      {app.isStuck ? (
        <section className="argo-lock-section">
          <h3 className="lock-section-title">
            <span className="lock-icon">🔒</span>
            Finalizer Locks ({app.terminatingResources.length})
          </h3>
          <p className="lock-section-desc">
            The following resources are blocking ArgoCD sync. Each holds active Kubernetes
            Finalizers that prevent deletion from completing.
          </p>
          <div className="lock-list">
            {app.terminatingResources.map((res, i) => {
              const hintKey = `${app.appName}::${res.kind}::${res.name}`;
              return (
                <TerminatingCard
                  key={hintKey}
                  resource={res}
                  index={i}
                  hintKey={hintKey}
                  isHintExpanded={expandedHints.has(hintKey)}
                  onToggleHint={onToggleHint}
                />
              );
            })}
          </div>
        </section>
      ) : (
        <div className="argo-clean-state">
          <span className="clean-icon">✓</span>
          <p>No Finalizer locks detected. Application sync is unblocked.</p>
        </div>
      )}
    </div>
  );
}

function TerminatingCard({ resource, index, hintKey, isHintExpanded, onToggleHint }) {
  const stuckDuration = resource.deletionTimestamp
    ? Math.round((Date.now() - new Date(resource.deletionTimestamp).getTime()) / 60_000)
    : null;

  return (
    <article className="lock-card" aria-label={`Terminating resource ${resource.kind}/${resource.name}`}>
      <div className="lock-card-header">
        <span className="lock-index">{index + 1}</span>
        <div className="lock-card-title">
          <CategoryBadge category={resource.resourceCategory} />
          <span className="lock-resource-name">
            {resource.namespace}/{resource.name}
          </span>
        </div>
        {stuckDuration !== null && (
          <span className="lock-duration" title={`Stuck since ${resource.deletionTimestamp}`}>
            ~{stuckDuration}m
          </span>
        )}
      </div>

      {/* Finalizer pills */}
      <div className="lock-finalizers">
        {resource.finalizers.map((f) => (
          <span
            key={f}
            className={`finalizer-pill ${resource.stellarFinalizers.includes(f) ? 'stellar' : ''}`}
          >
            {f}
          </span>
        ))}
      </div>

      {/* Resolution hint (expandable) */}
      <div className="lock-hint-wrap">
        <button
          type="button"
          className="hint-toggle"
          onClick={() => onToggleHint(hintKey)}
          aria-expanded={isHintExpanded}
        >
          {isHintExpanded ? '▲ Hide resolution hint' : '▼ Show resolution hint'}
        </button>
        {isHintExpanded && (
          <div className="hint-body" role="region">
            <p>{resource.resolutionHint}</p>
          </div>
        )}
      </div>
    </article>
  );
}

function SyncBadge({ status, large }) {
  const cls = {
    Synced: 'badge-synced',
    OutOfSync: 'badge-outofsync',
    Unknown: 'badge-unknown',
  }[status] ?? 'badge-unknown';
  return <span className={`sync-badge ${cls} ${large ? 'large' : ''}`}>{status}</span>;
}

function HealthBadge({ status, large }) {
  const cls = {
    Healthy: 'badge-healthy',
    Degraded: 'badge-degraded',
    Progressing: 'badge-progressing',
    Missing: 'badge-missing',
    Unknown: 'badge-unknown',
  }[status] ?? 'badge-unknown';
  return <span className={`health-badge ${cls} ${large ? 'large' : ''}`}>{status}</span>;
}

function CategoryBadge({ category }) {
  const cls = {
    Pod: 'cat-pod',
    PVC: 'cat-pvc',
    PV: 'cat-pv',
    StellarNode: 'cat-stellar',
    Unknown: 'cat-unknown',
  }[category] ?? 'cat-unknown';
  return <span className={`category-badge ${cls}`}>{category}</span>;
}

function CountChip({ label, value, tone }) {
  return (
    <div className="count-chip">
      <span className="count-label">{label}</span>
      <strong className={tone ? `tone-${tone}` : ''}>{value}</strong>
    </div>
  );
}
