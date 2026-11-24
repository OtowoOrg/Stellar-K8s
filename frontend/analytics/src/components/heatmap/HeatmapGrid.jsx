/**
 * HeatmapGrid.jsx
 *
 * Real-time Resource Saturation Heatmap for Stellar-K8s worker nodes.
 *
 * Renders a GitHub-contribution-style grid where each cell represents one
 * worker node / pod.  Cells are colored on a five-band spectrum from cool
 * (idle, dark blue) to hot (critical, red).  Layout is computed by D3 and
 * written directly to an SVG element; React only owns the outer container and
 * the tooltip overlay so we never block the main thread with VDOM diffing
 * for potentially 100 cells animating every 5 seconds.
 *
 * Key performance decisions:
 *   - D3 manages SVG DOM surgically (enter/update/exit) – zero React fiber work
 *     inside the SVG on each tick.
 *   - The SVG layout algorithm runs in a single synchronous pass per update.
 *   - Cell color transitions use CSS `transition` declarations rather than
 *     D3 tweens to keep JS off the hot path.
 *   - A ResizeObserver recalculates column count on container resize.
 *   - Tooltip state is deferred via a ref + a single setState to avoid
 *     spurious re-renders during mouse moves.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import * as d3 from 'd3';
import { BAND_COLORS, POLL_INTERVAL_MS } from '../../heatmapModel.js';
import { usePrometheusPoller } from './usePrometheusPoller.js';
import HeatmapTooltip from './HeatmapTooltip.jsx';

// --- Layout constants ---
const CELL_SIZE = 28;
const CELL_GAP = 4;
const CELL_STRIDE = CELL_SIZE + CELL_GAP;
const ZONE_LABEL_HEIGHT = 22;
const ZONE_PADDING_BOTTOM = 8;
const MISSING_OPACITY = 0.35;

/**
 * Main heatmap component.  Accepts all `usePrometheusPoller` options
 * as props so the parent can configure the data source.
 *
 * @param {object} props
 * @param {string}  [props.prometheusUrl]
 * @param {string}  [props.query]
 * @param {number}  [props.intervalMs]
 * @param {boolean} [props.paused]
 * @param {string}  [props.className]
 */
export default function HeatmapGrid({
  prometheusUrl = '/api/prometheus',
  query,
  intervalMs = POLL_INTERVAL_MS,
  paused = false,
  className = '',
}) {
  const { nodes, status, lastPollAt, error } = usePrometheusPoller({
    prometheusUrl,
    query,
    intervalMs,
    paused,
  });

  const svgRef = useRef(/** @type {SVGSVGElement|null} */ (null));
  const containerRef = useRef(/** @type {HTMLDivElement|null} */ (null));
  const columnsRef = useRef(10);

  // Tooltip state – we use a single object to avoid multiple setState calls.
  const [tooltip, setTooltip] = useState({
    visible: false,
    node: /** @type {import('../../heatmapModel.js').NodeMetric|null} */ (null),
    position: /** @type {{x:number,y:number}|null} */ (null),
  });

  // -----------------------------------------------------------------------
  // Responsive column count via ResizeObserver
  // -----------------------------------------------------------------------
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const obs = new ResizeObserver(([entry]) => {
      const width = entry.contentRect.width || 600;
      columnsRef.current = Math.max(1, Math.floor((width + CELL_GAP) / CELL_STRIDE));
    });
    obs.observe(container);
    return () => obs.disconnect();
  }, []);

  // -----------------------------------------------------------------------
  // D3 render — runs off the React render cycle via useEffect
  // -----------------------------------------------------------------------
  useEffect(() => {
    const svgEl = svgRef.current;
    if (!svgEl) return;
    if (nodes.length === 0) {
      d3.select(svgEl).attr('height', 60);
      d3.select(svgEl).selectAll('*').remove();
      return;
    }

    const cols = columnsRef.current;
    const svg = d3.select(svgEl);

    // ── Group nodes by zone ─────────────────────────────────────────────
    const zones = d3.group(nodes, (d) => d.zone || '(no zone)');

    // ── Compute row counts and total SVG height ─────────────────────────
    let yOffset = 4;
    const zoneLayouts = [];
    for (const [zoneName, zoneNodes] of zones) {
      const rows = Math.ceil(zoneNodes.length / cols);
      zoneLayouts.push({ zoneName, zoneNodes, yOffset });
      yOffset += ZONE_LABEL_HEIGHT + rows * CELL_STRIDE + ZONE_PADDING_BOTTOM;
    }
    const totalHeight = yOffset + 4;
    svg.attr('height', totalHeight);

    // ── Zone label groups ───────────────────────────────────────────────
    const zoneGroups = svg
      .selectAll('.zone-group')
      .data(zoneLayouts, (d) => d.zoneName);

    const zoneEnter = zoneGroups.enter().append('g').attr('class', 'zone-group');
    zoneEnter.append('text').attr('class', 'zone-label');

    const zoneMerge = zoneEnter.merge(zoneGroups);
    zoneMerge.attr('transform', (d) => `translate(0, ${d.yOffset})`);
    zoneMerge
      .select('.zone-label')
      .attr('x', 2)
      .attr('y', 14)
      .attr('fill', '#7f92a3')
      .attr('font-size', '11px')
      .attr('font-family', "'DM Mono', monospace")
      .attr('letter-spacing', '0.06em')
      .text((d) => d.zoneName.toUpperCase());

    zoneGroups.exit().remove();

    // ── Cell groups (one rect per node) ────────────────────────────────
    for (const { zoneName, zoneNodes, yOffset: zy } of zoneLayouts) {
      const groupSel = svg.selectAll(`.zone-group`).filter((d) => d.zoneName === zoneName);

      const cells = groupSel
        .selectAll('.hm-cell')
        .data(zoneNodes, (d) => d.id);

      // Enter
      const cellEnter = cells.enter().append('rect').attr('class', 'hm-cell');
      cellEnter
        .attr('rx', 4)
        .attr('ry', 4)
        .attr('width', CELL_SIZE)
        .attr('height', CELL_SIZE)
        .attr('tabindex', 0)
        .attr('role', 'gridcell')
        .style('transition', `fill ${POLL_INTERVAL_MS / 2}ms ease`)
        .on('mouseenter', function (event, d) {
          setTooltip({ visible: true, node: d, position: { x: event.clientX, y: event.clientY } });
          d3.select(this).attr('stroke', '#e8edf2').attr('stroke-width', 1.5);
        })
        .on('mousemove', function (event) {
          setTooltip((prev) =>
            prev.visible ? { ...prev, position: { x: event.clientX, y: event.clientY } } : prev,
          );
        })
        .on('mouseleave', function () {
          setTooltip((prev) => ({ ...prev, visible: false }));
          d3.select(this).attr('stroke', null);
        })
        .on('focus', function (event, d) {
          setTooltip({ visible: true, node: d, position: { x: event.clientX ?? 0, y: event.clientY ?? 0 } });
        })
        .on('blur', function () {
          setTooltip((prev) => ({ ...prev, visible: false }));
        })
        .on('keydown', function (event, d) {
          if (event.key === 'Enter' || event.key === ' ') {
            const rect = this.getBoundingClientRect();
            setTooltip({ visible: true, node: d, position: { x: rect.right, y: rect.top } });
          }
        });

      // Update + enter
      const cellMerge = cellEnter.merge(cells);
      cellMerge
        .attr('x', (_d, i) => (i % cols) * CELL_STRIDE)
        .attr('y', (_d, i) => ZONE_LABEL_HEIGHT + Math.floor(i / cols) * CELL_STRIDE)
        .attr('fill', (d) => BAND_COLORS[d.band] ?? BAND_COLORS.idle)
        .attr('opacity', (d) => (d.missing ? MISSING_OPACITY : 1))
        .attr('aria-label', (d) =>
          `${d.id}: CPU ${d.cpuPct.toFixed(1)}%, Memory ${d.memPct.toFixed(1)}%, saturation ${d.band}`,
        );

      // Exit
      cells.exit().remove();
    }
  }, [nodes]);

  // -----------------------------------------------------------------------
  // Resize: recalculate layout when container width changes
  // -----------------------------------------------------------------------
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const obs = new ResizeObserver(() => {
      // Trigger a re-render by updating SVG width attribute directly.
      const svgEl = svgRef.current;
      if (svgEl) svgEl.setAttribute('width', `${container.clientWidth}`);
    });
    obs.observe(container);
    return () => obs.disconnect();
  }, []);

  // -----------------------------------------------------------------------
  // Derive summary counts from nodes array (no extra state)
  // -----------------------------------------------------------------------
  const counts = deriveCount(nodes);

  return (
    <div className={`heatmap-root ${className}`.trim()} ref={containerRef}>
      {/* ── Header bar ────────────────────────────────────────────── */}
      <div className="hm-header">
        <div className="hm-title-block">
          <span className="eyebrow">WORKER NODES / RESOURCE SATURATION</span>
          <h2 className="hm-title">Real-Time Saturation Heatmap</h2>
          <p className="hm-subtitle">
            CPU &amp; Memory saturation across all Kubernetes worker nodes. Polls every 5 s.
          </p>
        </div>
        <div className="hm-controls">
          <StatusIndicator status={status} error={error} lastPollAt={lastPollAt} />
        </div>
      </div>

      {/* ── Summary strip ─────────────────────────────────────────── */}
      <div className="hm-summary-strip" role="region" aria-label="Saturation summary">
        <SummaryTile label="Total nodes" value={nodes.length} />
        <SummaryTile label="Idle" value={counts.idle} tone="" />
        <SummaryTile label="Moderate" value={counts.moderate} tone="green" />
        <SummaryTile label="Elevated" value={counts.elevated} tone="amber" />
        <SummaryTile label="High / Critical" value={counts.high + counts.critical} tone="red" />
        <SummaryTile label="Offline" value={counts.missing} tone="" muted />
      </div>

      {/* ── Color legend ──────────────────────────────────────────── */}
      <div className="hm-legend" aria-label="Saturation color legend">
        {Object.entries(BAND_COLORS).map(([band, color]) => (
          <span key={band} className="hm-legend-item">
            <span className="hm-swatch" style={{ background: color }} aria-hidden="true" />
            {band}
          </span>
        ))}
        <span className="hm-legend-item hm-legend-item--missing">
          <span className="hm-swatch hm-swatch--missing" aria-hidden="true" />
          offline
        </span>
      </div>

      {/* ── SVG grid ──────────────────────────────────────────────── */}
      <div className="hm-grid-wrap" role="grid" aria-label="Worker node saturation grid">
        {nodes.length === 0 ? (
          <div className="hm-empty">
            {status === 'polling' || status === 'idle'
              ? 'Waiting for Prometheus data…'
              : error
              ? `Error: ${error}`
              : 'No worker node metrics available.'}
          </div>
        ) : (
          <svg
            ref={svgRef}
            className="hm-svg"
            aria-hidden="true"
            width="100%"
          />
        )}
      </div>

      {/* ── Tooltip (portal) ──────────────────────────────────────── */}
      <HeatmapTooltip
        node={tooltip.node}
        position={tooltip.position}
        visible={tooltip.visible}
      />
    </div>
  );
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

function StatusIndicator({ status, error, lastPollAt }) {
  const dotClass =
    status === 'idle' ? 'live' : status === 'error' || status === 'offline' ? 'error' : 'connecting';
  return (
    <div className="hm-status">
      <span className={`status-dot ${dotClass}`} aria-hidden="true" />
      <span>
        {status === 'error' || status === 'offline'
          ? `${status}: ${error ?? 'unknown'}`
          : lastPollAt
          ? `updated ${lastPollAt.toLocaleTimeString()}`
          : 'connecting…'}
      </span>
    </div>
  );
}

function SummaryTile({ label, value, tone, muted }) {
  return (
    <div className={`hm-tile${muted ? ' hm-tile--muted' : ''}`}>
      <span className="hm-tile-label">{label}</span>
      <strong className={tone ? `tone-${tone}` : ''}>{value}</strong>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Counts nodes per saturation band (and missing) from a flat array.
 * @param {import('../../heatmapModel.js').NodeMetric[]} nodes
 */
function deriveCount(nodes) {
  const counts = { idle: 0, moderate: 0, elevated: 0, high: 0, critical: 0, missing: 0 };
  for (const node of nodes) {
    if (node.missing) { counts.missing += 1; continue; }
    counts[node.band] = (counts[node.band] ?? 0) + 1;
  }
  return counts;
}
