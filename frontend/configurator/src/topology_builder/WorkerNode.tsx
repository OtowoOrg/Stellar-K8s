/**
 * WorkerNode — a draggable tile representing a Kubernetes worker node.
 *
 * Displays the node name, its Kubernetes labels as small badges, and a zone
 * assignment indicator. The tile is draggable via the HTML5 Drag-and-Drop API
 * and sets a `DragPayload` on the `dataTransfer` object so drop targets can
 * identify which worker is being moved.
 */

import React, { useState } from 'react';
import type { WorkerNodeConfig, DragPayload } from './types';

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

export interface WorkerNodeProps {
  /** The worker node data to render. */
  workerNode: WorkerNodeConfig;
  /**
   * ID of the availability zone this worker is currently assigned to, or
   * null when unassigned.
   */
  assignedZoneId: string | null;
  /** Optional callback invoked when a drag starts on this tile. */
  onDrag?: (workerNode: WorkerNodeConfig, event: React.DragEvent<HTMLDivElement>) => void;
}

// ---------------------------------------------------------------------------
// Styles (inline, matching dark theme)
// ---------------------------------------------------------------------------

const CARD_BASE: React.CSSProperties = {
  background: '#111b27',
  border: '1px solid #273340',
  borderLeft: '3px solid #4ea8de',
  borderRadius: '6px',
  padding: '10px 12px',
  cursor: 'grab',
  userSelect: 'none',
  transition: 'border-color 0.15s, background 0.15s, box-shadow 0.15s',
  position: 'relative',
  fontFamily: "'Space Grotesk', sans-serif",
};

const CARD_DRAGGING: React.CSSProperties = {
  opacity: 0.5,
  boxShadow: '0 0 0 2px #4ea8de',
};

const HEADING: React.CSSProperties = {
  margin: 0,
  fontSize: '13px',
  fontWeight: 600,
  color: '#e8edf2',
  fontFamily: "'DM Mono', monospace",
  whiteSpace: 'nowrap',
  overflow: 'hidden',
  textOverflow: 'ellipsis',
};

const BADGE_ROW: React.CSSProperties = {
  display: 'flex',
  flexWrap: 'wrap',
  gap: '4px',
  marginTop: '6px',
};

const BADGE: React.CSSProperties = {
  background: '#1a2637',
  border: '1px solid #273340',
  borderRadius: '3px',
  padding: '1px 5px',
  fontSize: '10px',
  color: '#8899aa',
  fontFamily: "'DM Mono', monospace",
  whiteSpace: 'nowrap',
  overflow: 'hidden',
  maxWidth: '200px',
  textOverflow: 'ellipsis',
};

const ZONE_INDICATOR: React.CSSProperties = {
  marginTop: '8px',
  fontSize: '11px',
  color: '#39d98a',
  display: 'flex',
  alignItems: 'center',
  gap: '4px',
  fontFamily: "'DM Mono', monospace",
};

const UNASSIGNED_INDICATOR: React.CSSProperties = {
  marginTop: '8px',
  fontSize: '11px',
  color: '#556677',
  fontFamily: "'DM Mono', monospace",
};

const DRAG_HANDLE: React.CSSProperties = {
  position: 'absolute',
  top: '8px',
  right: '8px',
  color: '#445566',
  fontSize: '14px',
  lineHeight: 1,
  pointerEvents: 'none',
};

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

/**
 * Renders a single draggable worker-node tile.
 *
 * The component encodes a `DragPayload` into `dataTransfer` using the MIME
 * type `application/x-stellar-drag`. Drop targets should parse this payload
 * to determine the drag source and decide how to handle the drop.
 *
 * When `assignedZoneId` is provided the tile renders a green zone-assignment
 * indicator below the label badges so operators can see at a glance which
 * failure domain owns this worker.
 */
const WorkerNode: React.FC<WorkerNodeProps> = ({
  workerNode,
  assignedZoneId,
  onDrag,
}) => {
  const [isDragging, setIsDragging] = useState(false);
  const [isHovered, setIsHovered] = useState(false);

  // Build the drag payload: we treat a worker tile drag as a 'placed-node'
  // move intent. The nodeType field is populated as a placeholder; the actual
  // worker assignment logic lives in the drop handler.
  const payload: DragPayload = {
    type: 'placed-node',
    // Workers are not StellarNodes — use 'Validator' as a sentinel value.
    // The drop handler checks payload.type === 'placed-node' and the
    // workerNodeId embedded in the JSON data to distinguish worker moves.
    nodeType: 'Validator',
    placedNodeId: workerNode.id,
  };

  // Full JSON blob written to dataTransfer for the drop target
  const dragData = JSON.stringify({
    ...payload,
    _workerNodeId: workerNode.id,
    _isWorkerMove: true,
  });

  const handleDragStart = (event: React.DragEvent<HTMLDivElement>) => {
    event.dataTransfer.setData('application/x-stellar-drag', dragData);
    event.dataTransfer.effectAllowed = 'move';
    setIsDragging(true);
    onDrag?.(workerNode, event);
  };

  const handleDragEnd = () => {
    setIsDragging(false);
  };

  const cardStyle: React.CSSProperties = {
    ...CARD_BASE,
    ...(isDragging ? CARD_DRAGGING : {}),
    ...(isHovered && !isDragging
      ? { borderLeftColor: '#6bbfe8', background: '#152030', boxShadow: '0 2px 8px rgba(78,168,222,0.15)' }
      : {}),
  };

  // Build label badges — trim long values to keep UI compact
  const labelEntries = Object.entries(workerNode.labels);

  return (
    <div
      style={cardStyle}
      draggable
      onDragStart={handleDragStart}
      onDragEnd={handleDragEnd}
      onMouseEnter={() => setIsHovered(true)}
      onMouseLeave={() => setIsHovered(false)}
      role="listitem"
      aria-label={`Worker node ${workerNode.name}${assignedZoneId ? `, assigned to zone ${assignedZoneId}` : ', unassigned'}`}
      aria-grabbed={isDragging}
      tabIndex={0}
      onKeyDown={(e) => {
        // Allow keyboard users to initiate a conceptual drag (focus indication)
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
        }
      }}
    >
      {/* Drag handle icon */}
      <span style={DRAG_HANDLE} aria-hidden="true">⠿</span>

      {/* Node name */}
      <h4 style={HEADING} title={workerNode.name}>
        {workerNode.name}
      </h4>

      {/* Label badges */}
      {labelEntries.length > 0 && (
        <div style={BADGE_ROW} role="list" aria-label={`Labels for ${workerNode.name}`}>
          {labelEntries.map(([key, value]) => {
            const display = `${key}: ${value}`;
            return (
              <span
                key={key}
                style={BADGE}
                title={display}
                role="listitem"
              >
                {display}
              </span>
            );
          })}
        </div>
      )}

      {labelEntries.length === 0 && (
        <div style={{ ...BADGE_ROW }}>
          <span style={{ ...BADGE, color: '#445566', fontStyle: 'italic' }}>no labels</span>
        </div>
      )}

      {/* Zone assignment indicator */}
      {assignedZoneId ? (
        <div style={ZONE_INDICATOR} aria-label={`Assigned to zone ${assignedZoneId}`}>
          <span aria-hidden="true">◉</span>
          <span>{assignedZoneId}</span>
        </div>
      ) : (
        <div style={UNASSIGNED_INDICATOR} aria-label="Worker node is unassigned">
          <span>unassigned</span>
        </div>
      )}
    </div>
  );
};

export default WorkerNode;
