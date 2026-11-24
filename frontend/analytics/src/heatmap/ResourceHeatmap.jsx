import { memo, startTransition, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  DEFAULT_STALE_MS,
  DEFAULT_THRESHOLDS,
  createHeatmapState,
  ingestSamples,
  markError,
  materializeHeatmap,
  parsePrometheusResponse,
} from './heatmapModel.js';

const CELL_SIZE = 15;
const CELL_GAP = 4;
const ZONE_LABEL_HEIGHT = 20;
const ZONE_GAP = 14;
const DEFAULT_COLUMNS = 20;

const LEVELS = [
  ['idle', 'Idle'],
  ['cool', 'Cool'],
  ['warm', 'Warm'],
  ['hot', 'Hot'],
  ['critical', 'Saturated'],
];

// Pure layout for the zone-banded grid. Cells wrap to a new row every
// `columns` entries, GitHub-contribution-graph style, and zone bands stack
// vertically. Kept separate so the geometry can be unit tested directly.
export function buildHeatmapLayout(zones, columns = DEFAULT_COLUMNS) {
  const cols = Math.max(1, Math.floor(columns));
  const step = CELL_SIZE + CELL_GAP;
  let cursorY = 0;
  const laidOut = zones.map((zone) => {
    const rows = Math.max(1, Math.ceil(zone.cells.length / cols));
    const cells = zone.cells.map((cell, index) => ({
      ...cell,
      x: (index % cols) * step,
      y: ZONE_LABEL_HEIGHT + Math.floor(index / cols) * step,
    }));
    const height = ZONE_LABEL_HEIGHT + rows * step;
    const band = { zone: zone.zone, y: cursorY, height, peak: zone.peak, mean: zone.mean, cells };
    cursorY += height + ZONE_GAP;
    return band;
  });
  return {
    zones: laidOut,
    width: cols * step - CELL_GAP,
    height: Math.max(0, cursorY - ZONE_GAP),
  };
}

function samplesFromProp(initialSamples) {
  if (!initialSamples) return null;
  return Array.isArray(initialSamples) ? initialSamples : parsePrometheusResponse(initialSamples);
}

function formatPercent(ratio) {
  return `${Math.round(ratio * 100)}%`;
}

const HeatCell = memo(function HeatCell({ cell, onHover }) {
  const dim = cell.state === 'stale' ? 0.22 : cell.state === 'draining' ? 0.45 : 1;
  return (
    <rect
      className="heatmap-cell"
      x={cell.x}
      y={cell.y}
      width={CELL_SIZE}
      height={CELL_SIZE}
      rx="3"
      fill={cell.color}
      fillOpacity={dim}
      stroke={cell.state === 'draining' ? '#f05d5e' : 'rgba(255, 255, 255, 0.10)'}
      strokeDasharray={cell.state === 'draining' ? '3 2' : undefined}
      data-node={cell.id}
      data-zone={cell.zone}
      data-state={cell.state}
      data-level={cell.level}
      data-saturation={cell.saturation.toFixed(3)}
      tabIndex={0}
      role="img"
      aria-label={`${cell.id}, ${cell.zone}, CPU ${formatPercent(cell.cpu)}, memory ${formatPercent(cell.memory)}, ${cell.podCount} pods, ${cell.state}`}
      onMouseEnter={() => onHover(cell)}
      onMouseLeave={() => onHover(null)}
      onFocus={() => onHover(cell)}
      onBlur={() => onHover(null)}
    >
      <title>
        {`${cell.id} - ${cell.zone}\nCPU ${formatPercent(cell.cpu)} - MEM ${formatPercent(cell.memory)} - ${cell.podCount} pods\n${cell.state}`}
      </title>
    </rect>
  );
});

export default function ResourceHeatmap({
  endpoint = '/api/v1/query',
  query = 'stellar_operator_resource_usage',
  pollIntervalMs = 5000,
  thresholds = DEFAULT_THRESHOLDS,
  staleAfterMs = DEFAULT_STALE_MS,
  columns = DEFAULT_COLUMNS,
  now = Date.now,
  fetchImpl,
  initialSamples = null,
}) {
  const stableThresholds = useMemo(
    () => ({ warm: thresholds.warm, hot: thresholds.hot, critical: thresholds.critical }),
    [thresholds.warm, thresholds.hot, thresholds.critical],
  );

  const stateRef = useRef(null);
  if (stateRef.current === null) {
    stateRef.current = createHeatmapState();
    const seed = samplesFromProp(initialSamples);
    if (seed) ingestSamples(stateRef.current, seed, now(), staleAfterMs);
  }

  const [view, setView] = useState(() =>
    materializeHeatmap(stateRef.current, { thresholds: stableThresholds, now: now(), staleAfterMs }),
  );
  const [status, setStatus] = useState(initialSamples ? 'live' : 'connecting');
  const [hovered, setHovered] = useState(null);
  const frameRef = useRef(null);

  // Re-materialize at most once per animation frame and hand the result to
  // React as a non-urgent update so a 100-node refresh never blocks input.
  const publish = useCallback(
    (nextStatus) => {
      if (nextStatus) setStatus(nextStatus);
      if (frameRef.current !== null) return;
      const schedule =
        typeof requestAnimationFrame === 'function' ? requestAnimationFrame : (cb) => setTimeout(cb, 16);
      frameRef.current = schedule(() => {
        frameRef.current = null;
        const next = materializeHeatmap(stateRef.current, {
          thresholds: stableThresholds,
          now: now(),
          staleAfterMs,
        });
        startTransition(() => setView(next));
      });
    },
    [stableThresholds, staleAfterMs, now],
  );

  useEffect(() => {
    return () => {
      if (frameRef.current === null) return;
      const cancel =
        typeof cancelAnimationFrame === 'function' ? cancelAnimationFrame : clearTimeout;
      cancel(frameRef.current);
      frameRef.current = null;
    };
  }, []);

  useEffect(() => {
    const doFetch = fetchImpl || (typeof fetch === 'function' ? fetch.bind(globalThis) : null);
    if (!doFetch) {
      setStatus('error');
      return undefined;
    }
    let disposed = false;
    const separator = endpoint.includes('?') ? '&' : '?';
    const url = `${endpoint}${separator}query=${encodeURIComponent(query)}`;

    const poll = async () => {
      try {
        const response = await doFetch(url, { headers: { accept: 'application/json' } });
        if (disposed) return;
        if (!response.ok) throw new Error(`prometheus responded ${response.status}`);
        const contentType = response.headers?.get?.('content-type') ?? '';
        const payload = contentType.includes('json') ? await response.json() : await response.text();
        if (disposed) return;
        ingestSamples(stateRef.current, parsePrometheusResponse(payload), now(), staleAfterMs);
        publish('live');
      } catch (error) {
        if (disposed) return;
        markError(stateRef.current, error, now());
        publish('stale');
      }
    };

    poll();
    const timer = setInterval(poll, Math.max(500, pollIntervalMs));
    return () => {
      disposed = true;
      clearInterval(timer);
    };
  }, [endpoint, query, pollIntervalMs, staleAfterMs, publish, fetchImpl, now]);

  const layout = useMemo(() => buildHeatmapLayout(view.zones, columns), [view.zones, columns]);
  const { summary } = view;

  return (
    <div className="heatmap" data-status={status}>
      <div className="heatmap-toprow">
        <div className="heatmap-stats">
          <HeatStat label="Worker nodes" value={summary.nodeCount.toLocaleString()} />
          <HeatStat label="Mean saturation" value={formatPercent(summary.meanSaturation)} />
          <HeatStat
            label="Hottest"
            value={summary.hottest ? `${summary.hottest.id} ${formatPercent(summary.hottest.saturation)}` : 'n/a'}
            tone={summary.hottest && summary.hottest.saturation >= stableThresholds.hot ? 'hot' : undefined}
          />
          <HeatStat label="At / near saturation" value={(summary.byLevel.hot + summary.byLevel.critical).toLocaleString()} />
        </div>
        <div className="heatmap-scale" aria-hidden="true">
          <span>idle</span>
          <span className="heatmap-scale-ramp" />
          <span>saturated</span>
        </div>
      </div>

      {summary.lastError ? (
        <p className="heatmap-error" role="status">
          Metrics endpoint error: {summary.lastError.message}. Showing last known state.
        </p>
      ) : null}

      {summary.nodeCount === 0 ? (
        <p className="heatmap-empty">Waiting for worker-node telemetry from {endpoint}.</p>
      ) : (
        <div className="heatmap-scroll">
          <svg
            className="heatmap-grid"
            width={layout.width}
            height={layout.height}
            viewBox={`0 0 ${layout.width} ${layout.height}`}
            role="img"
            aria-label={`Resource saturation for ${summary.nodeCount} worker nodes across ${view.zones.length} availability zones`}
          >
            {layout.zones.map((band) => (
              <g key={band.zone} transform={`translate(0 ${band.y})`}>
                <text className="heatmap-zone-label" x="0" y="12">
                  {band.zone} - peak {formatPercent(band.peak)} - avg {formatPercent(band.mean)}
                </text>
                {band.cells.map((cell) => (
                  <HeatCell key={cell.id} cell={cell} onHover={setHovered} />
                ))}
              </g>
            ))}
          </svg>
        </div>
      )}

      <div className="heatmap-legend">
        {LEVELS.map(([key, label]) => (
          <span className="heatmap-legend-item" key={key}>
            <span className={`heatmap-legend-swatch level-${key}`} />
            {label}
            <span className="muted"> {summary.byLevel[key]}</span>
          </span>
        ))}
      </div>

      {hovered ? (
        <dl className="heatmap-inspect" aria-live="polite">
          <div>
            <dt>Node</dt>
            <dd>{hovered.id}</dd>
          </div>
          <div>
            <dt>Zone</dt>
            <dd>{hovered.zone}</dd>
          </div>
          <div>
            <dt>CPU</dt>
            <dd>{formatPercent(hovered.cpu)}</dd>
          </div>
          <div>
            <dt>Memory</dt>
            <dd>{formatPercent(hovered.memory)}</dd>
          </div>
          <div>
            <dt>Pods</dt>
            <dd>{hovered.podCount}</dd>
          </div>
          <div>
            <dt>State</dt>
            <dd>{hovered.state}</dd>
          </div>
        </dl>
      ) : null}
    </div>
  );
}

function HeatStat({ label, value, tone }) {
  return (
    <div className="heatmap-stat">
      <span className="heatmap-stat-label">{label}</span>
      <strong className={tone ? `tone-${tone}` : ''}>{value}</strong>
    </div>
  );
}
