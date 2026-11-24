import { StrictMode, useMemo, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import QuorumMatrixCanvas from '../../components/webgl/QuorumMatrixCanvas.jsx';
import { buildQuorumMatrix, matrixStats } from '../matrix/quorumMatrixModel.js';
import { buildMatrixMockTopology } from '../mock/matrixTopology.js';
import './styles.css';

// Deterministic mock topology for browser profiling, sized via query params
// (?nodes=120&edges=10000) so the harness can measure small and large matrices
// in the same browser session without a Kafka bridge or operator stream.
const params = new URLSearchParams(window.location.search);
const REBUILD = params.get('rebuild') === '1';
const NODES = Number(params.get('nodes')) || 120;
const EDGES = Number(params.get('edges')) || 10000;

function App() {
  const [buildMs, setBuildMs] = useState(null);
  const [nonce, setNonce] = useState(0);
  const [hoverCell, setHoverCell] = useState(null);
  const matrixRef = useRef(null);

  const matrix = useMemo(() => {
    const start = performance.now();
    const next = buildQuorumMatrix(buildMatrixMockTopology({ nodes: NODES, edges: EDGES }));
    const elapsed = performance.now() - start;
    matrixRef.current = next;
    if (REBUILD) setBuildMs(elapsed);
    return next;
  }, [nonce]);

  const summary = useMemo(() => matrixStats(matrix), [matrix]);

  return (
    <main className="app-shell">
      <header className="topbar">
        <div className="brand-block">
          <span className="eyebrow">STELLAR / PROFILING HARNESS</span>
          <h1>Quorum matrix — {matrix.cells.length.toLocaleString()} cells</h1>
          <p>
            {buildMs === null
              ? `${matrix.size} validators, avg trust ${summary.avgTrust.toFixed(3)}`
              : `matrix built in ${buildMs.toFixed(2)} ms`}
          </p>
        </div>
        {REBUILD && (
          <div className="toolbar">
            <button className="tool-button" type="button" onClick={() => setNonce((value) => value + 1)}>
              Rebuild matrix
            </button>
          </div>
        )}
      </header>
      <section className="workspace">
        <div className="graph-panel">
          <QuorumMatrixCanvas
            matrix={matrix}
            onHoverCell={setHoverCell}
            // Exposes per-second { fps, avgRenderJsMs, frames } to the browser
            // profiler, which records the JS-side render cost separately from
            // rasterizer throughput.
            onFrameTiming={(timing) => { window.__matrixTiming = timing; }}
          />
          <div className="legend" aria-label="Matrix legend">
            <span className="muted">
              {hoverCell ? `row ${hoverCell.sourceIndex} → col ${hoverCell.targetIndex}` : 'hover the matrix to inspect a cell'}
            </span>
          </div>
        </div>
        <aside className="inspector" aria-live="polite">
          <span className="eyebrow">CELL INSPECTOR</span>
          {hoverCell ? (
            <>
              <div className="node-heading">
                <code>{hoverCell.agreement}</code>
                <h2>{matrix.nodes[hoverCell.sourceIndex]?.name} → {matrix.nodes[hoverCell.targetIndex]?.name}</h2>
              </div>
              <dl className="detail-list">
                <div className="detail-row"><dt>Trust weight</dt><dd>{hoverCell.trust.toFixed(3)}</dd></div>
                <div className="detail-row"><dt>Latency delta</dt><dd>{hoverCell.latencyMs.toFixed(2)} ms</dd></div>
              </dl>
            </>
          ) : (
            <div className="empty-inspector">
              <div className="empty-icon">%</div>
              <h2>Hover a matrix cell</h2>
              <p>Validator trust metrics will appear here.</p>
            </div>
          )}
        </aside>
      </section>
    </main>
  );
}

createRoot(document.getElementById('root')).render(<StrictMode><App /></StrictMode>);
