/**
 * HeatmapTooltip.jsx
 *
 * Accessible floating tooltip for the heatmap cells.
 * Positioned at a pixel offset from the triggering cell via inline style.
 * Rendered into a portal so it is never clipped by overflow:hidden parents.
 */
import { createPortal } from 'react-dom';
import { BAND_COLORS } from '../../heatmapModel.js';

/**
 * @param {object} props
 * @param {import('../../heatmapModel.js').NodeMetric | null} props.node
 * @param {{ x: number; y: number } | null} props.position  Screen-space coordinates
 * @param {boolean} props.visible
 */
export default function HeatmapTooltip({ node, position, visible }) {
  if (!visible || !node || !position) return null;

  // Keep the tooltip within the viewport (simple right-edge guard).
  const left = Math.min(position.x + 14, window.innerWidth - 230);
  const top = position.y + 14;

  const bandColor = BAND_COLORS[node.band] ?? BAND_COLORS.idle;

  return createPortal(
    <div
      role="tooltip"
      className="heatmap-tooltip"
      style={{ left, top }}
      aria-label={`Resource usage for ${node.id}`}
    >
      <div className="ht-header" style={{ borderLeftColor: bandColor }}>
        <span className="ht-id">{node.id}</span>
        {node.missing && <span className="ht-badge ht-badge--missing">Offline</span>}
      </div>
      <dl className="ht-metrics">
        <Row label="Node" value={node.node} />
        {node.namespace && <Row label="Namespace" value={node.namespace} />}
        {node.zone && <Row label="Zone" value={node.zone} />}
        <Row
          label="CPU"
          value={`${node.cpuPct.toFixed(1)} %`}
          tone={toneCss(node.cpuPct)}
        />
        <Row
          label="Memory"
          value={`${node.memPct.toFixed(1)} %`}
          tone={toneCss(node.memPct)}
        />
        <Row
          label="Saturation"
          value={`${node.saturationPct.toFixed(1)} % — ${node.band}`}
          tone={toneCss(node.saturationPct)}
        />
      </dl>
    </div>,
    document.body,
  );
}

function Row({ label, value, tone }) {
  return (
    <div className="ht-row">
      <dt>{label}</dt>
      <dd className={tone ? `tone-${tone}` : ''}>{value}</dd>
    </div>
  );
}

/**
 * Maps a saturation percentage to a CSS tone name that matches
 * the project's existing .tone-* utility classes.
 * @param {number} pct
 * @returns {'green'|'amber'|'red'|''}
 */
function toneCss(pct) {
  if (pct >= 85) return 'red';
  if (pct >= 70) return 'amber';
  if (pct >= 40) return 'green';
  return '';
}
