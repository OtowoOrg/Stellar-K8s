import { StrictMode, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import TopologyScene from './SceneRenderer.jsx';
import { createStreamState, ingest, materialize, statusForNode } from './graphModel.js';
import './styles.css';

const EMPTY_GRAPH = materialize(createStreamState());
const query = new URLSearchParams(window.location.search);
const sourceFromQuery = query.get('source');
const bridgeUrl = query.get('ws') || 'localhost:8787';
const initialSource = sourceFromQuery === 'mock' || sourceFromQuery === 'kafka' ? sourceFromQuery : 'live';
const prefersReducedMotion = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false;

function streamUrl(source) {
  const protocol = window.location.protocol === 'https:' ? 'wss' : 'ws';
  if (source === 'mock' || source === 'kafka') return `${protocol}://${bridgeUrl}`;
  return `${protocol}://${window.location.host}/api/v1/quorum/topology/stream`;
}

function App() {
  const [source, setSource] = useState(initialSource);
  const [graph, setGraph] = useState(EMPTY_GRAPH);
  const [connection, setConnection] = useState('connecting');
  const [selected, setSelected] = useState(null);
  const [paused, setPaused] = useState(prefersReducedMotion);
  const [lastUpdate, setLastUpdate] = useState(null);
  const [frameStats, setFrameStats] = useState({ fps: 0, memory: null, nodes: 0, edges: 0 });
  const streamStateRef = useRef(createStreamState());
  const renderFrameRef = useRef(null);

  useEffect(() => {
    streamStateRef.current = createStreamState();
    setGraph(EMPTY_GRAPH);
    setSelected(null);
    setConnection('connecting');
    let socket;
    let disposed = false;
    const publishGraph = () => {
      if (renderFrameRef.current !== null) return;
      renderFrameRef.current = requestAnimationFrame(() => {
        renderFrameRef.current = null;
        if (disposed) return;
        setGraph(materialize(streamStateRef.current));
        setLastUpdate(new Date());
      });
    };

    try {
      socket = new WebSocket(streamUrl(source));
      socket.onopen = () => setConnection('live');
      socket.onmessage = (event) => {
        if (disposed) return;
        try {
          const payload = JSON.parse(event.data);
          streamStateRef.current = ingest(streamStateRef.current, payload);
          publishGraph();
        } catch {
          setConnection('error');
        }
      };
      socket.onerror = () => setConnection('error');
      socket.onclose = () => {
        if (!disposed) setConnection('offline');
      };
    } catch {
      setConnection('error');
    }

    return () => {
      disposed = true;
      socket?.close();
      if (renderFrameRef.current !== null) {
        cancelAnimationFrame(renderFrameRef.current);
        renderFrameRef.current = null;
      }
    };
  }, [source]);

  useEffect(() => {
    const update = (event) => setFrameStats(event.detail);
    window.addEventListener('topology-frame', update);
    return () => window.removeEventListener('topology-frame', update);
  }, []);

  const counts = useMemo(() => {
    const totals = { synced: 0, degraded: 0, falling: 0 };
    for (const node of graph.nodes) {
      const status = statusForNode(node);
      if (status === 'synced') totals.synced += 1;
      else if (status === 'degraded') totals.degraded += 1;
      else totals.falling += 1;
    }
    return totals;
  }, [graph.nodes]);

  const selectNode = useCallback((node) => setSelected(node), []);
  const sourceLabel = source === 'mock' ? 'Mock Kafka stream' : source === 'kafka' ? 'Kafka WebSocket bridge' : 'Operator WebSocket';

  return (
    <main className="app-shell">
      <header className="topbar">
        <div className="brand-block">
          <span className="eyebrow">Stellar-K8s analytics</span>
          <h1>Quorum topology</h1>
          <p>Multi-cluster SCP health, partitions, and validator latency.</p>
        </div>
        <div className="toolbar" role="toolbar" aria-label="Topology controls">
          <label className="select-wrap">
            <span>Source</span>
            <select value={source} onChange={(event) => setSource(event.target.value)}>
              <option value="live">Operator stream</option>
              <option value="kafka">Kafka bridge</option>
              <option value="mock">Mock stream</option>
            </select>
          </label>
          <button className="tool-button" type="button" onClick={() => setPaused((value) => !value)}>
            {paused ? 'Resume' : 'Pause'}
          </button>
        </div>
      </header>

      <section className="metric-strip" aria-label="Network summary">
        <Metric label="Validators" value={graph.nodes.length.toLocaleString()} detail={`${graph.edges.length.toLocaleString()} quorum links`} />
        <Metric label="Synced" value={counts.synced.toLocaleString()} detail="Externalize phase" tone="green" />
        <Metric label="Degraded" value={counts.degraded.toLocaleString()} detail="Prepare or confirm" tone="amber" />
        <Metric label="Behind" value={counts.falling.toLocaleString()} detail="Stalled or unknown" tone="red" />
        <Metric label="FPS" value={frameStats.fps || '-'} detail={frameStats.memory ? `${frameStats.memory} MB heap` : 'heap unavailable'} />
      </section>

      <section className="workspace">
        <div className="graph-panel">
          <div className="panel-heading">
            <div>
              <span className={`status-dot ${connection}`} aria-hidden="true" />
              <strong>{sourceLabel}</strong>
              <span className="muted">{lastUpdate ? `updated ${lastUpdate.toLocaleTimeString()}` : 'waiting for telemetry'}</span>
            </div>
            <span className="muted">{frameStats.nodes.toLocaleString()} nodes / {frameStats.edges.toLocaleString()} edges</span>
          </div>
          <TopologyScene graph={graph} onSelect={selectNode} selectedId={selected?.id} paused={paused} onFrame={setFrameStats} />
          <div className="legend" aria-label="Node health legend">
            <Legend color="green" label="Synced" />
            <Legend color="amber" label="Degraded" />
            <Legend color="red" label="Falling behind" />
          </div>
        </div>

        <aside className="inspector" aria-live="polite">
          <span className="eyebrow">Node inspector</span>
          {selected ? <NodeInspector node={selected} /> : <EmptyInspector />}
        </aside>
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

function Legend({ color, label }) {
  return (
    <span className="legend-item">
      <span className={`legend-swatch ${color}`} aria-hidden="true" />
      {label}
    </span>
  );
}

function EmptyInspector() {
  return (
    <div className="empty-inspector">
      <div className="empty-icon" aria-hidden="true">+</div>
      <h2>Select a validator</h2>
      <p>Validator metrics will appear here.</p>
    </div>
  );
}

function NodeInspector({ node }) {
  const status = statusForNode(node);
  return (
    <>
      <div className="node-heading">
        <span className={`node-status ${status}`}>{status.replace('-', ' ')}</span>
        <h2>{node.name}</h2>
        <code>{node.fullId}</code>
      </div>
      <dl className="detail-list">
        <Detail label="Cluster" value={node.cluster} />
        <Detail label="SCP phase" value={node.phase} />
        <Detail label="Ballot" value={node.ballotCounter.toLocaleString()} />
        <Detail label="TPS" value={node.tps ? node.tps.toFixed(1) : 'No sample'} />
        <Detail label="Ledger time" value={node.ledgerTimeMs ? `${node.ledgerTimeMs.toFixed(2)} ms` : 'No sample'} />
        <Detail label="Quorum threshold" value={node.threshold || 'Not reported'} />
      </dl>
      <div className="inspector-note">
        {node.critical ? 'Critical quorum member.' : 'No criticality alert reported.'}
        {node.stalled ? ' Consensus progress is stalled.' : ''}
      </div>
    </>
  );
}

function Detail({ label, value }) {
  return (
    <div className="detail-row">
      <dt>{label}</dt>
      <dd>{value}</dd>
    </div>
  );
}

createRoot(document.getElementById('root')).render(<StrictMode><App /></StrictMode>);
