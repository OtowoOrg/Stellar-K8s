/**
 * DR Command Center — Issue #92
 *
 * Multi-panel dashboard that visualises step-by-step disaster-recovery
 * execution progress across managed cluster nodes.  Connects to the
 * backend DR WebSocket endpoint and preserves full execution history
 * across network reconnects so operators never lose context during a
 * simulated node failover.
 *
 * Key features
 * ─────────────
 *  • Four visual status cards: Snapshot Restoration, Pod Recreation,
 *    Catch-Up Sync, Traffic Redirection.
 *  • Live pass / fail indicator per phase per cluster node.
 *  • "Trigger Simulated Drill" button that fires a dry-run failover via
 *    the backend API (POST /api/dr/trigger) and subscribes to
 *    real-time status updates over WebSocket.
 *  • Execution history panel: every completed drill run is appended to
 *    a scrollable log that survives WebSocket reconnects.
 *  • Automatic reconnect with exponential backoff so the dashboard
 *    stays live during the simulated node failovers it is watching.
 */

import React, {
  useCallback,
  useEffect,
  useReducer,
  useRef,
  useState,
} from 'react';

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/** DR phases rendered as status cards — order matches execution sequence. */
export const DR_PHASES = [
  {
    key: 'snapshot_restoration',
    label: 'Snapshot Restoration',
    icon: '🗄️',
    description: 'Restoring volume snapshot to target PVC',
  },
  {
    key: 'pod_recreation',
    label: 'Pod Recreation',
    icon: '🔄',
    description: 'Recreating validator pods from restored volumes',
  },
  {
    key: 'catchup_sync',
    label: 'Catch-Up Sync',
    icon: '⛓️',
    description: 'Syncing ledger state from history archives',
  },
  {
    key: 'traffic_redirection',
    label: 'Traffic Redirection',
    icon: '🔀',
    description: 'Redirecting load balancer traffic to recovered nodes',
  },
];

const PHASE_KEYS = DR_PHASES.map((p) => p.key);

/** Phase execution status values. */
const STATUS = {
  PENDING: 'pending',
  RUNNING: 'running',
  PASSED: 'passed',
  FAILED: 'failed',
  SKIPPED: 'skipped',
};

/** WebSocket reconnect — base delay and max cap (ms). */
const WS_BASE_DELAY_MS = 1_000;
const WS_MAX_DELAY_MS = 30_000;

// ---------------------------------------------------------------------------
// Colour helpers
// ---------------------------------------------------------------------------

const STATUS_COLORS = {
  [STATUS.PENDING]: '#6c757d',
  [STATUS.RUNNING]: '#0d6efd',
  [STATUS.PASSED]: '#198754',
  [STATUS.FAILED]: '#dc3545',
  [STATUS.SKIPPED]: '#adb5bd',
};

const STATUS_BG = {
  [STATUS.PENDING]: '#f8f9fa',
  [STATUS.RUNNING]: '#cfe2ff',
  [STATUS.PASSED]: '#d1e7dd',
  [STATUS.FAILED]: '#f8d7da',
  [STATUS.SKIPPED]: '#e9ecef',
};

function statusColor(s) {
  return STATUS_COLORS[s] ?? STATUS_COLORS[STATUS.PENDING];
}
function statusBg(s) {
  return STATUS_BG[s] ?? STATUS_BG[STATUS.PENDING];
}

// ---------------------------------------------------------------------------
// DR API client (thin wrapper — real implementation lives in dr_client.ts)
// ---------------------------------------------------------------------------

/**
 * POST /api/dr/trigger
 * Triggers a dry-run failover drill on the given cluster node.
 * Returns a drill_id that is echoed back on the WebSocket stream.
 */
async function triggerDrDrill({ node, dryRun = true, apiBase = '' }) {
  const resp = await fetch(`${apiBase}/api/dr/trigger`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ node, dry_run: dryRun }),
  });
  if (!resp.ok) {
    throw new Error(`DR trigger failed: ${resp.status} ${resp.statusText}`);
  }
  return resp.json(); // { drill_id, node, dry_run, queued_at }
}

/**
 * POST /api/dr/reset
 * Clears the backend drill state for a given node (dev / test only).
 */
async function resetDrState({ node, apiBase = '' }) {
  const resp = await fetch(`${apiBase}/api/dr/reset`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ node }),
  });
  if (!resp.ok) throw new Error(`DR reset failed: ${resp.status}`);
  return resp.json();
}

// ---------------------------------------------------------------------------
// State management
// ---------------------------------------------------------------------------

/**
 * Per-node DR state shape:
 * {
 *   phases: { [phaseKey]: { status, startedAt, completedAt, message } },
 *   drillId: string | null,
 *   startedAt: number | null,
 *   completedAt: number | null,
 * }
 */
function makeNodeState() {
  return {
    phases: Object.fromEntries(
      PHASE_KEYS.map((k) => [
        k,
        { status: STATUS.PENDING, startedAt: null, completedAt: null, message: '' },
      ]),
    ),
    drillId: null,
    startedAt: null,
    completedAt: null,
  };
}

function drReducer(state, action) {
  switch (action.type) {
    case 'RESET_NODE': {
      return {
        ...state,
        nodes: {
          ...state.nodes,
          [action.node]: makeNodeState(),
        },
      };
    }
    case 'DRILL_STARTED': {
      return {
        ...state,
        activeDrill: action.node,
        nodes: {
          ...state.nodes,
          [action.node]: {
            ...makeNodeState(),
            drillId: action.drillId,
            startedAt: Date.now(),
          },
        },
      };
    }
    case 'PHASE_UPDATE': {
      const { node, phase, status, message } = action;
      const prev = state.nodes[node] ?? makeNodeState();
      const prevPhase = prev.phases[phase] ?? {};
      const now = Date.now();
      return {
        ...state,
        nodes: {
          ...state.nodes,
          [node]: {
            ...prev,
            phases: {
              ...prev.phases,
              [phase]: {
                ...prevPhase,
                status,
                message: message ?? prevPhase.message,
                startedAt:
                  status === STATUS.RUNNING ? now : prevPhase.startedAt,
                completedAt:
                  status === STATUS.PASSED || status === STATUS.FAILED
                    ? now
                    : prevPhase.completedAt,
              },
            },
          },
        },
      };
    }
    case 'DRILL_COMPLETE': {
      const { node } = action;
      const prev = state.nodes[node] ?? makeNodeState();
      return {
        ...state,
        activeDrill: state.activeDrill === node ? null : state.activeDrill,
        nodes: {
          ...state.nodes,
          [node]: { ...prev, completedAt: Date.now() },
        },
      };
    }
    case 'WS_STATUS': {
      return { ...state, wsStatus: action.status };
    }
    default:
      return state;
  }
}

function makeInitialState(nodes) {
  return {
    activeDrill: null,
    wsStatus: 'connecting',
    nodes: Object.fromEntries(nodes.map((n) => [n, makeNodeState()])),
  };
}

// ---------------------------------------------------------------------------
// Execution history (preserved across WS reconnects)
// ---------------------------------------------------------------------------

function useHistory() {
  const [history, setHistory] = useState([]);
  const append = useCallback((entry) => {
    setHistory((prev) => [
      { ...entry, id: Date.now() + Math.random() },
      ...prev,
    ].slice(0, 200)); // cap at 200 entries
  }, []);
  return [history, append];
}

// ---------------------------------------------------------------------------
// WebSocket hook with auto-reconnect
// ---------------------------------------------------------------------------

function useReconnectingWs({ url, onMessage, onStatusChange }) {
  const wsRef = useRef(null);
  const retryRef = useRef(0);
  const mountedRef = useRef(true);
  const onMessageRef = useRef(onMessage);
  const onStatusRef = useRef(onStatusChange);
  onMessageRef.current = onMessage;
  onStatusRef.current = onStatusChange;

  useEffect(() => {
    mountedRef.current = true;
    let timeoutId;

    function connect() {
      if (!mountedRef.current) return;
      onStatusRef.current('connecting');
      try {
        const ws = new WebSocket(url);
        wsRef.current = ws;

        ws.onopen = () => {
          if (!mountedRef.current) { ws.close(); return; }
          retryRef.current = 0;
          onStatusRef.current('connected');
        };

        ws.onmessage = (evt) => {
          if (!mountedRef.current) return;
          try {
            onMessageRef.current(JSON.parse(evt.data));
          } catch {
            // malformed frame — ignore
          }
        };

        ws.onclose = () => {
          if (!mountedRef.current) return;
          onStatusRef.current('reconnecting');
          const delay = Math.min(
            WS_BASE_DELAY_MS * 2 ** retryRef.current,
            WS_MAX_DELAY_MS,
          );
          retryRef.current += 1;
          timeoutId = setTimeout(connect, delay);
        };

        ws.onerror = () => {
          ws.close(); // triggers onclose → reconnect
        };
      } catch {
        onStatusRef.current('error');
      }
    }

    connect();

    return () => {
      mountedRef.current = false;
      clearTimeout(timeoutId);
      wsRef.current?.close();
    };
  }, [url]);

  const send = useCallback((data) => {
    if (wsRef.current?.readyState === WebSocket.OPEN) {
      wsRef.current.send(JSON.stringify(data));
    }
  }, []);

  return send;
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

/** Single phase status card. */
function PhaseCard({ phase, phaseState, isCurrent }) {
  const { status, message, startedAt, completedAt } = phaseState;
  const elapsed =
    startedAt && completedAt
      ? ((completedAt - startedAt) / 1000).toFixed(1)
      : null;

  return (
    <div
      style={{
        backgroundColor: statusBg(status),
        border: `2px solid ${statusColor(status)}`,
        borderRadius: 8,
        padding: '12px 16px',
        flex: 1,
        minWidth: 160,
        transition: 'border-color 0.3s, background-color 0.3s',
        boxShadow: isCurrent ? `0 0 0 3px ${statusColor(status)}55` : 'none',
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 6 }}>
        <span style={{ fontSize: 20 }}>{phase.icon}</span>
        <strong style={{ fontSize: 13, color: '#333' }}>{phase.label}</strong>
      </div>
      <div
        style={{
          display: 'inline-block',
          padding: '2px 8px',
          borderRadius: 12,
          backgroundColor: statusColor(status),
          color: '#fff',
          fontSize: 11,
          fontWeight: 700,
          textTransform: 'uppercase',
          letterSpacing: 0.5,
          marginBottom: 6,
        }}
      >
        {status === STATUS.RUNNING ? '⏳ Running' :
         status === STATUS.PASSED  ? '✅ Passed'  :
         status === STATUS.FAILED  ? '❌ Failed'  :
         status === STATUS.SKIPPED ? '⏭ Skipped' : '⏸ Pending'}
      </div>
      <p style={{ margin: '4px 0 0', fontSize: 11, color: '#555', minHeight: 16 }}>
        {message || phase.description}
      </p>
      {elapsed && (
        <p style={{ margin: '4px 0 0', fontSize: 11, color: '#777' }}>
          Completed in {elapsed}s
        </p>
      )}
    </div>
  );
}

/** Per-node panel with its four phase cards. */
function NodePanel({ node, nodeState }) {
  const phases = nodeState.phases;
  const currentPhase = PHASE_KEYS.find(
    (k) => phases[k].status === STATUS.RUNNING,
  );
  const allDone = PHASE_KEYS.every(
    (k) => phases[k].status === STATUS.PASSED || phases[k].status === STATUS.SKIPPED,
  );
  const anyFailed = PHASE_KEYS.some((k) => phases[k].status === STATUS.FAILED);
  const overallStatus = anyFailed
    ? STATUS.FAILED
    : allDone
    ? STATUS.PASSED
    : currentPhase
    ? STATUS.RUNNING
    : STATUS.PENDING;

  return (
    <div
      style={{
        backgroundColor: '#fff',
        borderRadius: 10,
        padding: 20,
        boxShadow: '0 2px 8px rgba(0,0,0,0.08)',
        border: `1px solid ${statusColor(overallStatus)}44`,
      }}
    >
      {/* Node header */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 16 }}>
        <span
          style={{
            width: 10, height: 10, borderRadius: '50%',
            backgroundColor: statusColor(overallStatus),
            display: 'inline-block',
            boxShadow: overallStatus === STATUS.RUNNING
              ? `0 0 6px ${statusColor(STATUS.RUNNING)}` : 'none',
          }}
        />
        <strong style={{ fontSize: 15, color: '#333' }}>{node}</strong>
        {nodeState.drillId && (
          <code style={{ fontSize: 11, color: '#888', marginLeft: 'auto' }}>
            drill:{nodeState.drillId.slice(0, 8)}
          </code>
        )}
      </div>

      {/* Phase cards row */}
      <div style={{ display: 'flex', gap: 10, flexWrap: 'wrap' }}>
        {DR_PHASES.map((phase) => (
          <PhaseCard
            key={phase.key}
            phase={phase}
            phaseState={phases[phase.key]}
            isCurrent={currentPhase === phase.key}
          />
        ))}
      </div>
    </div>
  );
}

/** WebSocket connection badge. */
function WsBadge({ status }) {
  const map = {
    connecting:   { color: '#fd7e14', label: '⟳ Connecting' },
    connected:    { color: '#198754', label: '● Live' },
    reconnecting: { color: '#dc3545', label: '↺ Reconnecting' },
    error:        { color: '#dc3545', label: '✕ Error' },
  };
  const { color, label } = map[status] ?? map.connecting;
  return (
    <span
      style={{
        fontSize: 12, fontWeight: 700, color,
        padding: '2px 8px', borderRadius: 10,
        border: `1px solid ${color}`,
        backgroundColor: `${color}18`,
      }}
    >
      {label}
    </span>
  );
}

/** Execution history log panel. */
function HistoryPanel({ entries }) {
  if (entries.length === 0) {
    return (
      <div style={{ color: '#999', fontSize: 13, padding: 12 }}>
        No drill history yet. Trigger a simulated drill to begin.
      </div>
    );
  }
  return (
    <div style={{ maxHeight: 220, overflowY: 'auto' }}>
      {entries.map((e) => (
        <div
          key={e.id}
          style={{
            display: 'flex', gap: 12, alignItems: 'flex-start',
            padding: '6px 0',
            borderBottom: '1px solid #f0f0f0',
            fontSize: 12,
          }}
        >
          <span
            style={{
              color: statusColor(e.status), fontWeight: 700,
              minWidth: 60,
            }}
          >
            {e.status.toUpperCase()}
          </span>
          <span style={{ color: '#555', flex: 1 }}>
            {e.node} › {e.phase} — {e.message}
          </span>
          <span style={{ color: '#aaa', whiteSpace: 'nowrap' }}>
            {new Date(e.ts).toLocaleTimeString()}
          </span>
        </div>
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------

/**
 * DrCommandCenter
 *
 * Props
 * ─────
 * nodes        string[]   Cluster node names to monitor (required)
 * wsUrl        string     WebSocket endpoint (default: ws://localhost:8080/api/dr/stream)
 * apiBase      string     REST API base URL (default: '')
 * onDrillStart function   Callback when a drill is triggered
 * onDrillEnd   function   Callback when a drill completes
 * styles       object     Optional style overrides for the root container
 */
export function DrCommandCenter({
  nodes = [],
  wsUrl = 'ws://localhost:8080/api/dr/stream',
  apiBase = '',
  onDrillStart,
  onDrillEnd,
  styles = {},
}) {
  const [state, dispatch] = useReducer(drReducer, nodes, makeInitialState);
  const [history, appendHistory] = useHistory();
  const [error, setError] = useState(null);
  const [triggering, setTriggering] = useState(false);
  const [selectedNode, setSelectedNode] = useState(nodes[0] ?? null);

  // Handle incoming WebSocket frames
  const handleWsMessage = useCallback(
    (msg) => {
      /*
       * Expected frame shapes from the backend:
       *   { type: 'phase_update', node, phase, status, message }
       *   { type: 'drill_complete', node, drill_id }
       *   { type: 'drill_started',  node, drill_id }
       */
      if (msg.type === 'phase_update') {
        dispatch({
          type: 'PHASE_UPDATE',
          node: msg.node,
          phase: msg.phase,
          status: msg.status,
          message: msg.message,
        });
        appendHistory({
          ts: Date.now(),
          node: msg.node,
          phase: msg.phase,
          status: msg.status,
          message: msg.message ?? '',
        });
      } else if (msg.type === 'drill_started') {
        dispatch({ type: 'DRILL_STARTED', node: msg.node, drillId: msg.drill_id });
        onDrillStart?.({ node: msg.node, drillId: msg.drill_id });
      } else if (msg.type === 'drill_complete') {
        dispatch({ type: 'DRILL_COMPLETE', node: msg.node });
        appendHistory({
          ts: Date.now(),
          node: msg.node,
          phase: 'all',
          status: msg.status ?? STATUS.PASSED,
          message: 'Drill completed',
        });
        onDrillEnd?.({ node: msg.node, drillId: msg.drill_id });
      }
    },
    [appendHistory, onDrillStart, onDrillEnd],
  );

  const handleWsStatus = useCallback((status) => {
    dispatch({ type: 'WS_STATUS', status });
  }, []);

  const wsSend = useReconnectingWs({
    url: wsUrl,
    onMessage: handleWsMessage,
    onStatusChange: handleWsStatus,
  });

  // Trigger simulated drill
  const handleTriggerDrill = useCallback(async () => {
    if (!selectedNode || triggering) return;
    setError(null);
    setTriggering(true);
    try {
      const result = await triggerDrDrill({
        node: selectedNode,
        dryRun: true,
        apiBase,
      });
      // Optimistically mark the node as starting — WS will confirm
      dispatch({ type: 'DRILL_STARTED', node: selectedNode, drillId: result.drill_id });
      // Also notify backend via WS so other subscribers see it
      wsSend({ action: 'subscribe_drill', drill_id: result.drill_id });
    } catch (err) {
      setError(err.message);
    } finally {
      setTriggering(false);
    }
  }, [selectedNode, triggering, apiBase, wsSend]);

  // Reset a node's state (dev/test)
  const handleReset = useCallback(async (node) => {
    try {
      await resetDrState({ node, apiBase });
      dispatch({ type: 'RESET_NODE', node });
    } catch (err) {
      setError(err.message);
    }
  }, [apiBase]);

  const isAnyDrillRunning = state.activeDrill !== null;

  // Calculate overall progress across all nodes
  const overallProgress = (() => {
    const total = nodes.length * PHASE_KEYS.length;
    if (total === 0) return 0;
    let done = 0;
    nodes.forEach((n) => {
      PHASE_KEYS.forEach((k) => {
        const s = state.nodes[n]?.phases[k]?.status;
        if (s === STATUS.PASSED || s === STATUS.SKIPPED) done++;
      });
    });
    return Math.round((done / total) * 100);
  })();

  return (
    <div
      style={{
        fontFamily: styles.fontFamily || 'system-ui, sans-serif',
        backgroundColor: styles.backgroundColor || '#f4f6f9',
        borderRadius: styles.borderRadius || 12,
        padding: styles.padding || 24,
        minHeight: styles.minHeight || 500,
        ...styles,
      }}
    >
      {/* ── Header ── */}
      <div
        style={{
          display: 'flex', alignItems: 'center', gap: 12,
          marginBottom: 24,
          flexWrap: 'wrap',
        }}
      >
        <h2 style={{ margin: 0, fontSize: 20, color: '#1a1a2e', flex: 1 }}>
          🚨 DR Command Center
          {isAnyDrillRunning && (
            <span
              style={{
                marginLeft: 12, fontSize: 13, color: '#dc3545',
                fontWeight: 700, animation: 'pulse 1.2s infinite',
              }}
            >
              ● DRILL IN PROGRESS
            </span>
          )}
        </h2>
        <WsBadge status={state.wsStatus} />
      </div>

      {/* ── Error banner ── */}
      {error && (
        <div
          style={{
            backgroundColor: '#f8d7da', color: '#842029',
            borderRadius: 6, padding: '10px 16px',
            marginBottom: 16, fontSize: 13,
            display: 'flex', justifyContent: 'space-between',
          }}
        >
          <span>⚠️ {error}</span>
          <button
            onClick={() => setError(null)}
            style={{ background: 'none', border: 'none', cursor: 'pointer', color: '#842029' }}
          >
            ✕
          </button>
        </div>
      )}

      {/* ── Controls row ── */}
      <div
        style={{
          display: 'flex', gap: 10, alignItems: 'center',
          marginBottom: 20, flexWrap: 'wrap',
        }}
      >
        <select
          value={selectedNode ?? ''}
          onChange={(e) => setSelectedNode(e.target.value)}
          disabled={isAnyDrillRunning}
          style={{
            padding: '6px 12px', borderRadius: 6,
            border: '1px solid #ced4da', fontSize: 13,
            backgroundColor: isAnyDrillRunning ? '#e9ecef' : '#fff',
          }}
        >
          {nodes.map((n) => (
            <option key={n} value={n}>{n}</option>
          ))}
        </select>

        <button
          onClick={handleTriggerDrill}
          disabled={isAnyDrillRunning || triggering || !selectedNode}
          style={{
            padding: '7px 18px',
            backgroundColor: isAnyDrillRunning || triggering ? '#6c757d' : '#dc3545',
            color: '#fff', border: 'none', borderRadius: 6,
            cursor: isAnyDrillRunning || triggering ? 'not-allowed' : 'pointer',
            fontSize: 13, fontWeight: 600,
          }}
        >
          {triggering ? '⏳ Triggering…' : '🔥 Trigger Simulated Drill (dry-run)'}
        </button>

        {selectedNode && (
          <button
            onClick={() => handleReset(selectedNode)}
            disabled={isAnyDrillRunning}
            style={{
              padding: '7px 14px',
              backgroundColor: '#fff',
              color: '#6c757d',
              border: '1px solid #ced4da',
              borderRadius: 6,
              cursor: isAnyDrillRunning ? 'not-allowed' : 'pointer',
              fontSize: 13,
            }}
          >
            ↺ Reset
          </button>
        )}
      </div>

      {/* ── Node panels ── */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 16, marginBottom: 24 }}>
        {nodes.map((node) => (
          <NodePanel
            key={node}
            node={node}
            nodeState={state.nodes[node] ?? makeNodeState()}
          />
        ))}
        {nodes.length === 0 && (
          <div
            style={{
              backgroundColor: '#fff', borderRadius: 10, padding: 32,
              textAlign: 'center', color: '#6c757d', fontSize: 14,
            }}
          >
            No nodes configured. Pass a <code>nodes</code> prop to DrCommandCenter.
          </div>
        )}
      </div>

      {/* ── Overall progress bar ── */}
      <div
        style={{
          backgroundColor: '#fff', borderRadius: 10, padding: 16,
          boxShadow: '0 1px 4px rgba(0,0,0,0.06)', marginBottom: 16,
        }}
      >
        <div
          style={{
            display: 'flex', justifyContent: 'space-between',
            fontSize: 12, color: '#555', marginBottom: 8,
          }}
        >
          <span>Overall Recovery Progress</span>
          <strong>{overallProgress}%</strong>
        </div>
        <div
          style={{
            height: 10, backgroundColor: '#e9ecef',
            borderRadius: 5, overflow: 'hidden',
          }}
        >
          <div
            style={{
              height: '100%',
              width: `${overallProgress}%`,
              backgroundColor:
                overallProgress === 100 ? '#198754' :
                overallProgress > 0 ? '#0d6efd' : '#e9ecef',
              transition: 'width 0.5s ease',
              borderRadius: 5,
            }}
          />
        </div>
      </div>

      {/* ── Execution history ── */}
      <div
        style={{
          backgroundColor: '#fff', borderRadius: 10, padding: 16,
          boxShadow: '0 1px 4px rgba(0,0,0,0.06)',
        }}
      >
        <h3 style={{ margin: '0 0 12px', fontSize: 14, color: '#333' }}>
          📋 Execution History
          <span
            style={{
              marginLeft: 8, fontSize: 11, color: '#888',
              fontWeight: 400,
            }}
          >
            (preserved across reconnects)
          </span>
        </h3>
        <HistoryPanel entries={history} />
      </div>
    </div>
  );
}

export default DrCommandCenter;
