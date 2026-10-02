/**
 * DR Command Center — Issue #92
 *
 * Multi-panel dashboard that visualises step-by-step disaster-recovery
 * execution progress across managed cluster nodes in real time.
 *
 * Panels
 * ──────
 *  • Snapshot Restoration  🗄️
 *  • Pod Recreation        🔄
 *  • Catch-Up Sync         ⛓️
 *  • Traffic Redirection   🔀
 *
 * Design constraints (from issue)
 * ─────────────────────────────────
 *  1. Execution history is preserved across network reconnects during
 *     simulated node failovers — history lives in React state, never in
 *     the WebSocket connection.
 *  2. "Trigger Simulated Drill" fires POST /api/dr/trigger with dry_run:true.
 *  3. Live pass/fail indicators per phase, updated via WebSocket frames.
 */

import React, {
  useCallback,
  useEffect,
  useReducer,
  useRef,
  useState,
} from 'react';

// ─── Phase catalogue ─────────────────────────────────────────────────────────

export const DR_PHASES = [
  {
    key: 'snapshot_restoration',
    label: 'Snapshot Restoration',
    icon: '🗄️',
    description: 'Restore volume snapshot to target PVC',
  },
  {
    key: 'pod_recreation',
    label: 'Pod Recreation',
    icon: '🔄',
    description: 'Recreate validator pods from restored volumes',
  },
  {
    key: 'catchup_sync',
    label: 'Catch-Up Sync',
    icon: '⛓️',
    description: 'Sync ledger state from history archives',
  },
  {
    key: 'traffic_redirection',
    label: 'Traffic Redirection',
    icon: '🔀',
    description: 'Redirect load balancer traffic to recovered nodes',
  },
];

const PHASE_KEYS = DR_PHASES.map((p) => p.key);

const STATUS = {
  PENDING:  'pending',
  RUNNING:  'running',
  PASSED:   'passed',
  FAILED:   'failed',
  SKIPPED:  'skipped',
};

// ─── Colour palette ──────────────────────────────────────────────────────────

const COLOR = {
  [STATUS.PENDING]:  '#6c757d',
  [STATUS.RUNNING]:  '#0d6efd',
  [STATUS.PASSED]:   '#198754',
  [STATUS.FAILED]:   '#dc3545',
  [STATUS.SKIPPED]:  '#adb5bd',
};

const BG = {
  [STATUS.PENDING]:  '#f8f9fa',
  [STATUS.RUNNING]:  '#cfe2ff',
  [STATUS.PASSED]:   '#d1e7dd',
  [STATUS.FAILED]:   '#f8d7da',
  [STATUS.SKIPPED]:  '#e9ecef',
};

const color = (s) => COLOR[s] ?? COLOR[STATUS.PENDING];
const bg    = (s) => BG[s]    ?? BG[STATUS.PENDING];

// ─── WebSocket reconnect constants ───────────────────────────────────────────

const WS_BASE_MS = 1_000;
const WS_MAX_MS  = 30_000;

// ─── DR REST helpers ─────────────────────────────────────────────────────────

async function triggerDrDrill({ node, dryRun = true, apiBase = '' }) {
  const res = await fetch(`${apiBase}/api/dr/trigger`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ node, dry_run: dryRun }),
  });
  if (!res.ok) throw new Error(`DR trigger failed: ${res.status} ${res.statusText}`);
  return res.json();
}

async function resetDrState({ node, apiBase = '' }) {
  const res = await fetch(`${apiBase}/api/dr/reset`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ node }),
  });
  if (!res.ok) throw new Error(`DR reset failed: ${res.status}`);
  return res.json();
}

// ─── State helpers ───────────────────────────────────────────────────────────

function makeNodeState() {
  return {
    phases: Object.fromEntries(
      PHASE_KEYS.map((k) => [
        k,
        { status: STATUS.PENDING, startedAt: null, completedAt: null, message: '' },
      ]),
    ),
    drillId:     null,
    startedAt:   null,
    completedAt: null,
  };
}

function drReducer(state, action) {
  switch (action.type) {
    case 'RESET_NODE':
      return { ...state, nodes: { ...state.nodes, [action.node]: makeNodeState() } };

    case 'DRILL_STARTED':
      return {
        ...state,
        activeDrill: action.node,
        nodes: {
          ...state.nodes,
          [action.node]: { ...makeNodeState(), drillId: action.drillId, startedAt: Date.now() },
        },
      };

    case 'PHASE_UPDATE': {
      const { node, phase, status, message } = action;
      const prev      = state.nodes[node] ?? makeNodeState();
      const prevPhase = prev.phases[phase] ?? {};
      const now       = Date.now();
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
                message:     message ?? prevPhase.message,
                startedAt:   status === STATUS.RUNNING ? now : prevPhase.startedAt,
                completedAt: (status === STATUS.PASSED || status === STATUS.FAILED)
                               ? now : prevPhase.completedAt,
              },
            },
          },
        },
      };
    }

    case 'DRILL_COMPLETE':
      return {
        ...state,
        activeDrill: state.activeDrill === action.node ? null : state.activeDrill,
        nodes: {
          ...state.nodes,
          [action.node]: { ...(state.nodes[action.node] ?? makeNodeState()), completedAt: Date.now() },
        },
      };

    case 'WS_STATUS':
      return { ...state, wsStatus: action.status };

    default:
      return state;
  }
}

function initState(nodes) {
  return {
    activeDrill: null,
    wsStatus:    'connecting',
    nodes:       Object.fromEntries(nodes.map((n) => [n, makeNodeState()])),
  };
}

// ─── History hook ────────────────────────────────────────────────────────────

function useHistory() {
  const [history, setHistory] = useState([]);
  const append = useCallback((entry) => {
    setHistory((prev) => [{ ...entry, id: Date.now() + Math.random() }, ...prev].slice(0, 200));
  }, []);
  return [history, append];
}

// ─── Reconnecting WebSocket hook ─────────────────────────────────────────────

function useReconnectingWs({ url, onMessage, onStatusChange }) {
  const wsRef    = useRef(null);
  const retryRef = useRef(0);
  const mountRef = useRef(true);
  const msgRef   = useRef(onMessage);
  const stRef    = useRef(onStatusChange);
  msgRef.current = onMessage;
  stRef.current  = onStatusChange;

  useEffect(() => {
    mountRef.current = true;
    let timer;

    function connect() {
      if (!mountRef.current) return;
      stRef.current('connecting');
      try {
        const ws = new WebSocket(url);
        wsRef.current = ws;

        ws.onopen = () => {
          if (!mountRef.current) { ws.close(); return; }
          retryRef.current = 0;
          stRef.current('connected');
        };

        ws.onmessage = (e) => {
          if (!mountRef.current) return;
          try { msgRef.current(JSON.parse(e.data)); } catch { /* ignore */ }
        };

        ws.onclose = () => {
          if (!mountRef.current) return;
          stRef.current('reconnecting');
          const delay = Math.min(WS_BASE_MS * 2 ** retryRef.current, WS_MAX_MS);
          retryRef.current += 1;
          timer = setTimeout(connect, delay);
        };

        ws.onerror = () => ws.close();
      } catch { stRef.current('error'); }
    }

    connect();
    return () => {
      mountRef.current = false;
      clearTimeout(timer);
      wsRef.current?.close();
    };
  }, [url]);

  return useCallback((data) => {
    if (wsRef.current?.readyState === WebSocket.OPEN)
      wsRef.current.send(JSON.stringify(data));
  }, []);
}

// ─── Sub-components ──────────────────────────────────────────────────────────

function PhaseCard({ phase, phaseState, isCurrent }) {
  const { status, message, startedAt, completedAt } = phaseState;
  const elapsed = (startedAt && completedAt)
    ? ((completedAt - startedAt) / 1000).toFixed(1)
    : null;

  const badgeLabel =
    status === STATUS.RUNNING ? '⏳ Running'  :
    status === STATUS.PASSED  ? '✅ Passed'   :
    status === STATUS.FAILED  ? '❌ Failed'   :
    status === STATUS.SKIPPED ? '⏭ Skipped'  : '⏸ Pending';

  return (
    <div style={{
      background: bg(status),
      border: `2px solid ${color(status)}`,
      borderRadius: 8,
      padding: '12px 14px',
      flex: 1,
      minWidth: 152,
      transition: 'border-color .3s, background .3s',
      boxShadow: isCurrent ? `0 0 0 3px ${color(status)}44` : 'none',
    }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 6 }}>
        <span style={{ fontSize: 18 }}>{phase.icon}</span>
        <strong style={{ fontSize: 12, color: '#333' }}>{phase.label}</strong>
      </div>
      <div style={{
        display: 'inline-block',
        padding: '2px 8px', borderRadius: 12,
        background: color(status), color: '#fff',
        fontSize: 11, fontWeight: 700,
        textTransform: 'uppercase', letterSpacing: .4,
        marginBottom: 6,
      }}>{badgeLabel}</div>
      <p style={{ margin: '4px 0 0', fontSize: 11, color: '#555', minHeight: 14 }}>
        {message || phase.description}
      </p>
      {elapsed && (
        <p style={{ margin: '4px 0 0', fontSize: 10, color: '#888' }}>
          Completed in {elapsed}s
        </p>
      )}
    </div>
  );
}

function NodePanel({ node, nodeState }) {
  const phases      = nodeState.phases;
  const currentKey  = PHASE_KEYS.find((k) => phases[k].status === STATUS.RUNNING);
  const allDone     = PHASE_KEYS.every((k) => phases[k].status === STATUS.PASSED || phases[k].status === STATUS.SKIPPED);
  const anyFailed   = PHASE_KEYS.some((k)  => phases[k].status === STATUS.FAILED);
  const overall     = anyFailed ? STATUS.FAILED : allDone ? STATUS.PASSED : currentKey ? STATUS.RUNNING : STATUS.PENDING;

  return (
    <div style={{
      background: '#fff', borderRadius: 10, padding: 18,
      boxShadow: '0 2px 8px rgba(0,0,0,.07)',
      border: `1px solid ${color(overall)}44`,
    }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 14 }}>
        <span style={{
          width: 9, height: 9, borderRadius: '50%',
          background: color(overall), display: 'inline-block',
          boxShadow: overall === STATUS.RUNNING ? `0 0 5px ${color(STATUS.RUNNING)}` : 'none',
        }} />
        <strong style={{ fontSize: 14, color: '#333' }}>{node}</strong>
        {nodeState.drillId && (
          <code style={{ fontSize: 10, color: '#888', marginLeft: 'auto' }}>
            drill:{nodeState.drillId.slice(0, 8)}
          </code>
        )}
      </div>
      <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
        {DR_PHASES.map((phase) => (
          <PhaseCard
            key={phase.key}
            phase={phase}
            phaseState={phases[phase.key]}
            isCurrent={currentKey === phase.key}
          />
        ))}
      </div>
    </div>
  );
}

function WsBadge({ status }) {
  const map = {
    connecting:   { c: '#fd7e14', l: '⟳ Connecting'  },
    connected:    { c: '#198754', l: '● Live'          },
    reconnecting: { c: '#dc3545', l: '↺ Reconnecting' },
    error:        { c: '#dc3545', l: '✕ Error'        },
  };
  const { c, l } = map[status] ?? map.connecting;
  return (
    <span style={{
      fontSize: 11, fontWeight: 700, color: c,
      padding: '2px 8px', borderRadius: 10,
      border: `1px solid ${c}`, background: `${c}18`,
    }}>{l}</span>
  );
}

function HistoryPanel({ entries }) {
  if (entries.length === 0) {
    return (
      <p style={{ color: '#999', fontSize: 12, padding: '8px 0', margin: 0 }}>
        No drill history yet. Trigger a simulated drill to begin.
      </p>
    );
  }
  return (
    <div style={{ maxHeight: 200, overflowY: 'auto' }}>
      {entries.map((e) => (
        <div key={e.id} style={{
          display: 'flex', gap: 10, padding: '5px 0',
          borderBottom: '1px solid #f0f0f0', fontSize: 11,
        }}>
          <span style={{ color: color(e.status), fontWeight: 700, minWidth: 58 }}>
            {e.status.toUpperCase()}
          </span>
          <span style={{ color: '#555', flex: 1 }}>
            {e.node} › {e.phase} {e.message ? `— ${e.message}` : ''}
          </span>
          <span style={{ color: '#bbb', whiteSpace: 'nowrap' }}>
            {new Date(e.ts).toLocaleTimeString()}
          </span>
        </div>
      ))}
    </div>
  );
}

// ─── Main component ───────────────────────────────────────────────────────────

/**
 * DrCommandCenter
 *
 * Props
 * ─────
 * nodes        string[]   Cluster node names to monitor (required)
 * wsUrl        string     WebSocket endpoint (default ws://localhost:8080/api/dr/stream)
 * apiBase      string     REST base URL (default '')
 * onDrillStart function   Callback when a drill triggers
 * onDrillEnd   function   Callback when a drill completes
 * styles       object     Root container style overrides
 */
export function DrCommandCenter({
  nodes = [],
  wsUrl = 'ws://localhost:8080/api/dr/stream',
  apiBase = '',
  onDrillStart,
  onDrillEnd,
  styles = {},
}) {
  const [state, dispatch]   = useReducer(drReducer, nodes, initState);
  const [history, append]   = useHistory();
  const [error, setError]   = useState(null);
  const [busy, setBusy]     = useState(false);
  const [target, setTarget] = useState(nodes[0] ?? null);

  const handleMsg = useCallback((msg) => {
    if (msg.type === 'phase_update') {
      dispatch({ type: 'PHASE_UPDATE', node: msg.node, phase: msg.phase, status: msg.status, message: msg.message });
      append({ ts: Date.now(), node: msg.node, phase: msg.phase, status: msg.status, message: msg.message ?? '' });
    } else if (msg.type === 'drill_started') {
      dispatch({ type: 'DRILL_STARTED', node: msg.node, drillId: msg.drill_id });
      onDrillStart?.({ node: msg.node, drillId: msg.drill_id });
    } else if (msg.type === 'drill_complete') {
      dispatch({ type: 'DRILL_COMPLETE', node: msg.node });
      append({ ts: Date.now(), node: msg.node, phase: 'all', status: msg.status ?? STATUS.PASSED, message: 'Drill completed' });
      onDrillEnd?.({ node: msg.node, drillId: msg.drill_id });
    }
  }, [append, onDrillStart, onDrillEnd]);

  const wsSend = useReconnectingWs({
    url: wsUrl,
    onMessage: handleMsg,
    onStatusChange: useCallback((s) => dispatch({ type: 'WS_STATUS', status: s }), []),
  });

  const triggerDrill = useCallback(async () => {
    if (!target || busy) return;
    setError(null); setBusy(true);
    try {
      const result = await triggerDrDrill({ node: target, dryRun: true, apiBase });
      dispatch({ type: 'DRILL_STARTED', node: target, drillId: result.drill_id });
      wsSend({ action: 'subscribe_drill', drill_id: result.drill_id });
    } catch (err) {
      setError(err.message);
    } finally {
      setBusy(false);
    }
  }, [target, busy, apiBase, wsSend]);

  const resetNode = useCallback(async (node) => {
    try {
      await resetDrState({ node, apiBase });
      dispatch({ type: 'RESET_NODE', node });
    } catch (err) {
      setError(err.message);
    }
  }, [apiBase]);

  const drilling   = state.activeDrill !== null;

  const progress = (() => {
    const total = nodes.length * PHASE_KEYS.length;
    if (!total) return 0;
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
    <div style={{
      fontFamily: styles.fontFamily || 'system-ui,sans-serif',
      background:  styles.backgroundColor || '#f4f6f9',
      borderRadius: styles.borderRadius || 12,
      padding:  styles.padding  || 22,
      minHeight: styles.minHeight || 480,
      ...styles,
    }}>

      {/* Header */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 20, flexWrap: 'wrap' }}>
        <h2 style={{ margin: 0, fontSize: 19, color: '#1a1a2e', flex: 1 }}>
          🚨 DR Command Center
          {drilling && (
            <span style={{ marginLeft: 12, fontSize: 12, color: '#dc3545', fontWeight: 700 }}>
              ● DRILL IN PROGRESS
            </span>
          )}
        </h2>
        <WsBadge status={state.wsStatus} />
      </div>

      {/* Error banner */}
      {error && (
        <div style={{
          background: '#f8d7da', color: '#842029',
          borderRadius: 6, padding: '9px 14px',
          marginBottom: 14, fontSize: 12,
          display: 'flex', justifyContent: 'space-between',
        }}>
          <span>⚠️ {error}</span>
          <button onClick={() => setError(null)}
            style={{ background: 'none', border: 'none', cursor: 'pointer', color: '#842029' }}>✕</button>
        </div>
      )}

      {/* Controls */}
      <div style={{ display: 'flex', gap: 8, alignItems: 'center', marginBottom: 18, flexWrap: 'wrap' }}>
        <select
          value={target ?? ''}
          onChange={(e) => setTarget(e.target.value)}
          disabled={drilling}
          style={{ padding: '6px 10px', borderRadius: 6, border: '1px solid #ced4da', fontSize: 12 }}
        >
          {nodes.map((n) => <option key={n} value={n}>{n}</option>)}
        </select>

        <button
          onClick={triggerDrill}
          disabled={drilling || busy || !target}
          style={{
            padding: '7px 16px',
            background: (drilling || busy) ? '#6c757d' : '#dc3545',
            color: '#fff', border: 'none', borderRadius: 6,
            cursor: (drilling || busy) ? 'not-allowed' : 'pointer',
            fontSize: 12, fontWeight: 600,
          }}
        >
          {busy ? '⏳ Triggering…' : '🔥 Trigger Simulated Drill (dry-run)'}
        </button>

        {target && (
          <button
            onClick={() => resetNode(target)}
            disabled={drilling}
            style={{
              padding: '7px 12px', background: '#fff', color: '#6c757d',
              border: '1px solid #ced4da', borderRadius: 6,
              cursor: drilling ? 'not-allowed' : 'pointer', fontSize: 12,
            }}
          >
            ↺ Reset
          </button>
        )}
      </div>

      {/* Node panels */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 14, marginBottom: 20 }}>
        {nodes.length === 0
          ? <div style={{ background: '#fff', borderRadius: 10, padding: 28, textAlign: 'center', color: '#6c757d', fontSize: 13 }}>
              No nodes configured. Pass a <code>nodes</code> prop.
            </div>
          : nodes.map((n) => (
              <NodePanel key={n} node={n} nodeState={state.nodes[n] ?? makeNodeState()} />
            ))
        }
      </div>

      {/* Progress bar */}
      <div style={{ background: '#fff', borderRadius: 10, padding: 14, boxShadow: '0 1px 4px rgba(0,0,0,.05)', marginBottom: 14 }}>
        <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 11, color: '#555', marginBottom: 6 }}>
          <span>Overall Recovery Progress</span>
          <strong>{progress}%</strong>
        </div>
        <div style={{ height: 8, background: '#e9ecef', borderRadius: 4, overflow: 'hidden' }}>
          <div style={{
            height: '100%',
            width: `${progress}%`,
            background: progress === 100 ? '#198754' : progress > 0 ? '#0d6efd' : '#e9ecef',
            transition: 'width .5s ease',
            borderRadius: 4,
          }} />
        </div>
      </div>

      {/* Execution history */}
      <div style={{ background: '#fff', borderRadius: 10, padding: 14, boxShadow: '0 1px 4px rgba(0,0,0,.05)' }}>
        <h3 style={{ margin: '0 0 10px', fontSize: 13, color: '#333' }}>
          📋 Execution History
          <span style={{ marginLeft: 8, fontSize: 10, color: '#aaa', fontWeight: 400 }}>
            (preserved across reconnects)
          </span>
        </h3>
        <HistoryPanel entries={history} />
      </div>
    </div>
  );
}

export default DrCommandCenter;
