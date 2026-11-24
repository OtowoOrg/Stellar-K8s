/**
 * AvailabilityZonePanel — visual drop-zone container for a single Kubernetes
 * availability zone.
 *
 * Renders the zone header, lists any Stellar nodes placed within the zone,
 * lists the worker nodes assigned to the zone, and acts as an HTML5
 * drag-and-drop target. Highlights with a green glow when a drag is active
 * over the zone and surfaces any validation errors/warnings for this zone.
 *
 * The component is named `AvailabilityZonePanel` internally to avoid a name
 * clash with the `AvailabilityZone` type imported from `./types`. It is
 * exported as `default` and also as `AvailabilityZonePanel`.
 */

import React, { useState } from 'react';
import type {
  AvailabilityZone,
  PlacedStellarNode,
  WorkerNodeConfig,
  DragPayload,
  NodeType,
  ValidationError,
  ValidationWarning,
} from './types';
import WorkerNode from './WorkerNode';

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

export interface AvailabilityZonePanelProps {
  /** The availability zone this panel represents. */
  zone: AvailabilityZone;
  /** All Stellar nodes currently placed in this zone. */
  placedNodes: PlacedStellarNode[];
  /** Worker nodes assigned to this zone (filtered from global list). */
  workerNodes: WorkerNodeConfig[];
  /**
   * Pending node configuration that will be used when a node-type tile is
   * dropped onto this zone. Passed down from TopologyBuilder.
   */
  pendingNodeConfig?: Record<string, unknown> | null;
  /** Callback invoked on a successful drop onto this zone. */
  onDrop: (zoneId: string, payload: DragPayload) => void;
  /** Callback to remove a placed node from this zone. */
  onRemoveNode: (nodeId: string) => void;
  /** Validation errors that reference this zone (shown as red badges). */
  errors?: ValidationError[];
  /** Validation warnings that reference this zone (shown as amber badges). */
  warnings?: ValidationWarning[];
}

// ---------------------------------------------------------------------------
// Node type accent colours
// ---------------------------------------------------------------------------

const TYPE_ACCENT: Record<NodeType, string> = {
  Validator: '#39d98a',
  Horizon: '#4ea8de',
  SorobanRpc: '#9d7cd8',
};

const TYPE_LABEL: Record<NodeType, string> = {
  Validator: 'Validator',
  Horizon: 'Horizon',
  SorobanRpc: 'Soroban RPC',
};

// ---------------------------------------------------------------------------
// Styles
// ---------------------------------------------------------------------------

const ZONE_CARD: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  background: '#0e1820',
  border: '1px solid #273340',
  borderRadius: '10px',
  overflow: 'hidden',
  minHeight: '280px',
  transition: 'border-color 0.15s, box-shadow 0.15s',
  fontFamily: "'Space Grotesk', sans-serif",
};

const ZONE_CARD_DRAG_OVER: React.CSSProperties = {
  borderColor: '#39d98a',
  boxShadow: '0 0 0 2px rgba(57,217,138,0.3), inset 0 0 20px rgba(57,217,138,0.04)',
};

const ZONE_HEADER: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'space-between',
  padding: '10px 14px',
  background: '#0b1822',
  borderBottom: '1px solid #1e2d3d',
};

const ZONE_NAME: React.CSSProperties = {
  fontWeight: 700,
  fontSize: '13px',
  color: '#e8edf2',
  fontFamily: "'DM Mono', monospace",
  margin: 0,
};

const REGION_BADGE: React.CSSProperties = {
  background: '#1a2637',
  border: '1px solid #273340',
  borderRadius: '3px',
  padding: '2px 7px',
  fontSize: '10px',
  color: '#7a8fa8',
  fontFamily: "'DM Mono', monospace",
  letterSpacing: '0.03em',
};

const ZONE_BODY: React.CSSProperties = {
  flex: 1,
  padding: '12px',
  display: 'flex',
  flexDirection: 'column',
  gap: '8px',
};

const SECTION_LABEL: React.CSSProperties = {
  fontSize: '10px',
  fontWeight: 700,
  letterSpacing: '0.08em',
  textTransform: 'uppercase',
  color: '#445566',
  margin: '4px 0 6px 0',
};

// Placed stellar node card
const PLACED_CARD: React.CSSProperties = {
  background: '#111b27',
  border: '1px solid #273340',
  borderRadius: '6px',
  padding: '9px 11px',
  display: 'flex',
  alignItems: 'center',
  gap: '8px',
  position: 'relative',
};

const TYPE_BADGE: React.CSSProperties = {
  borderRadius: '4px',
  padding: '2px 7px',
  fontSize: '10px',
  fontWeight: 700,
  letterSpacing: '0.04em',
  fontFamily: "'DM Mono', monospace",
  whiteSpace: 'nowrap',
};

const NODE_NAME_TEXT: React.CSSProperties = {
  fontSize: '12px',
  color: '#c8d8e8',
  fontFamily: "'DM Mono', monospace",
  flex: 1,
  overflow: 'hidden',
  textOverflow: 'ellipsis',
  whiteSpace: 'nowrap',
};

const REPLICA_BADGE: React.CSSProperties = {
  fontSize: '10px',
  color: '#7a8fa8',
  background: '#0b1119',
  border: '1px solid #273340',
  borderRadius: '3px',
  padding: '1px 5px',
  fontFamily: "'DM Mono', monospace",
  whiteSpace: 'nowrap',
};

const REMOVE_BTN: React.CSSProperties = {
  background: 'none',
  border: '1px solid transparent',
  borderRadius: '3px',
  color: '#445566',
  cursor: 'pointer',
  fontSize: '14px',
  lineHeight: 1,
  padding: '2px 4px',
  transition: 'color 0.1s, border-color 0.1s',
  flexShrink: 0,
};

const DROP_PLACEHOLDER: React.CSSProperties = {
  flex: 1,
  display: 'flex',
  flexDirection: 'column',
  alignItems: 'center',
  justifyContent: 'center',
  border: '2px dashed #273340',
  borderRadius: '8px',
  padding: '24px 12px',
  color: '#445566',
  fontSize: '12px',
  textAlign: 'center',
  gap: '6px',
  transition: 'border-color 0.15s, color 0.15s',
};

const DROP_PLACEHOLDER_ACTIVE: React.CSSProperties = {
  borderColor: '#39d98a',
  color: '#39d98a',
  background: 'rgba(57,217,138,0.04)',
};

const VALIDATION_STRIP: React.CSSProperties = {
  padding: '6px 12px',
  borderTop: '1px solid #1e2d3d',
  display: 'flex',
  flexDirection: 'column',
  gap: '3px',
};

const VALIDATION_ITEM: React.CSSProperties = {
  display: 'flex',
  alignItems: 'flex-start',
  gap: '5px',
  fontSize: '11px',
  lineHeight: 1.4,
};

// ---------------------------------------------------------------------------
// Helper: PlacedNodeCard
// ---------------------------------------------------------------------------

interface PlacedNodeCardProps {
  node: PlacedStellarNode;
  onRemove: (id: string) => void;
}

const PlacedNodeCard: React.FC<PlacedNodeCardProps> = ({ node, onRemove }) => {
  const [removeHovered, setRemoveHovered] = useState(false);
  const accent = TYPE_ACCENT[node.nodeType];

  return (
    <div
      style={{ ...PLACED_CARD, borderLeftWidth: '3px', borderLeftColor: accent }}
      role="listitem"
      aria-label={`${TYPE_LABEL[node.nodeType]} node ${node.name}, ${node.replicas} replica${node.replicas !== 1 ? 's' : ''}`}
    >
      {/* Type badge */}
      <span
        style={{
          ...TYPE_BADGE,
          background: `${accent}22`,
          color: accent,
          border: `1px solid ${accent}44`,
        }}
        aria-hidden="true"
      >
        {TYPE_LABEL[node.nodeType]}
      </span>

      {/* Node name */}
      <span style={NODE_NAME_TEXT} title={`${node.name} (${node.namespace})`}>
        {node.name}
      </span>

      {/* Replica count */}
      <span style={REPLICA_BADGE} aria-label={`${node.replicas} replica${node.replicas !== 1 ? 's' : ''}`}>
        ×{node.replicas}
      </span>

      {/* Remove button */}
      <button
        style={{
          ...REMOVE_BTN,
          color: removeHovered ? '#f05d5e' : '#445566',
          borderColor: removeHovered ? '#f05d5e55' : 'transparent',
        }}
        onClick={() => onRemove(node.id)}
        onMouseEnter={() => setRemoveHovered(true)}
        onMouseLeave={() => setRemoveHovered(false)}
        aria-label={`Remove ${node.name} from zone`}
        title="Remove node"
      >
        ✕
      </button>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------

const AvailabilityZonePanel: React.FC<AvailabilityZonePanelProps> = ({
  zone,
  placedNodes,
  workerNodes,
  onDrop,
  onRemoveNode,
  errors = [],
  warnings = [],
}) => {
  const [isDragOver, setIsDragOver] = useState(false);

  const handleDragOver = (event: React.DragEvent<HTMLDivElement>) => {
    event.preventDefault();
    event.dataTransfer.dropEffect = 'copy';
    setIsDragOver(true);
  };

  const handleDragLeave = (event: React.DragEvent<HTMLDivElement>) => {
    // Only clear if leaving the zone panel entirely (not entering a child)
    const rect = event.currentTarget.getBoundingClientRect();
    const { clientX, clientY } = event;
    if (
      clientX < rect.left ||
      clientX > rect.right ||
      clientY < rect.top ||
      clientY > rect.bottom
    ) {
      setIsDragOver(false);
    }
  };

  const handleDrop = (event: React.DragEvent<HTMLDivElement>) => {
    event.preventDefault();
    setIsDragOver(false);

    const raw = event.dataTransfer.getData('application/x-stellar-drag');
    if (!raw) return;

    try {
      const payload = JSON.parse(raw) as DragPayload;
      onDrop(zone.id, payload);
    } catch {
      console.warn('[AvailabilityZonePanel] Failed to parse drag payload', raw);
    }
  };

  const hasContent = placedNodes.length > 0;
  const zoneHasErrors = errors.length > 0;
  const zoneHasWarnings = warnings.length > 0;

  const cardStyle: React.CSSProperties = {
    ...ZONE_CARD,
    ...(isDragOver ? ZONE_CARD_DRAG_OVER : {}),
    ...(zoneHasErrors
      ? { borderColor: 'rgba(240,93,94,0.4)' }
      : zoneHasWarnings
      ? { borderColor: 'rgba(245,185,66,0.35)' }
      : {}),
  };

  return (
    <div
      style={cardStyle}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
      role="region"
      aria-label={`Availability zone ${zone.name}, region ${zone.region}${hasContent ? `, ${placedNodes.length} node${placedNodes.length !== 1 ? 's' : ''} placed` : ', empty'}`}
      tabIndex={0}
    >
      {/* Zone header */}
      <header style={ZONE_HEADER}>
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          {/* Status dot */}
          <span
            style={{
              display: 'inline-block',
              width: '7px',
              height: '7px',
              borderRadius: '50%',
              background: zoneHasErrors ? '#f05d5e' : zoneHasWarnings ? '#f5b942' : '#39d98a',
              flexShrink: 0,
            }}
            aria-hidden="true"
          />
          <h3 style={ZONE_NAME}>{zone.name}</h3>
        </div>
        <span style={REGION_BADGE} aria-label={`Region: ${zone.region}`}>
          {zone.region}
        </span>
      </header>

      {/* Zone body */}
      <div style={ZONE_BODY}>

        {/* Placed Stellar nodes section */}
        {hasContent && (
          <section aria-label={`Stellar nodes in ${zone.name}`}>
            <p style={SECTION_LABEL}>Stellar Nodes</p>
            <div role="list" aria-label={`${placedNodes.length} Stellar node${placedNodes.length !== 1 ? 's' : ''} placed`}>
              {placedNodes.map((node) => (
                <PlacedNodeCard
                  key={node.id}
                  node={node}
                  onRemove={onRemoveNode}
                />
              ))}
            </div>
          </section>
        )}

        {/* Drop placeholder */}
        {!hasContent ? (
          <div
            style={{
              ...DROP_PLACEHOLDER,
              ...(isDragOver ? DROP_PLACEHOLDER_ACTIVE : {}),
            }}
            aria-live="polite"
            aria-atomic="true"
          >
            <span style={{ fontSize: '22px' }} aria-hidden="true">
              {isDragOver ? '⊕' : '⊞'}
            </span>
            <span>
              {isDragOver
                ? 'Release to place node here'
                : 'Drag a node type here'}
            </span>
            {!isDragOver && (
              <span style={{ fontSize: '10px', color: '#334455' }}>
                configure a node in the palette first
              </span>
            )}
          </div>
        ) : (
          /* Secondary drop area when nodes already exist */
          isDragOver && (
            <div
              style={{
                ...DROP_PLACEHOLDER,
                ...DROP_PLACEHOLDER_ACTIVE,
                flex: 'none',
                minHeight: '56px',
                padding: '12px',
              }}
              aria-live="polite"
            >
              <span style={{ fontSize: '14px' }} aria-hidden="true">⊕</span>
              <span>Release to add another node</span>
            </div>
          )
        )}

        {/* Worker nodes section */}
        {workerNodes.length > 0 && (
          <section aria-label={`Worker nodes in ${zone.name}`} style={{ marginTop: hasContent ? '4px' : 0 }}>
            <p style={SECTION_DIVIDER_STYLE}>
              Workers ({workerNodes.length})
            </p>
            <div
              role="list"
              aria-label={`${workerNodes.length} worker node${workerNodes.length !== 1 ? 's' : ''}`}
              style={{ display: 'flex', flexDirection: 'column', gap: '6px' }}
            >
              {workerNodes.map((wn) => (
                <WorkerNode
                  key={wn.id}
                  workerNode={wn}
                  assignedZoneId={zone.id}
                />
              ))}
            </div>
          </section>
        )}

        {workerNodes.length === 0 && !hasContent && !isDragOver && (
          <p style={{ fontSize: '11px', color: '#334455', margin: '0', textAlign: 'center' }}>
            No worker nodes assigned to this zone.
          </p>
        )}
      </div>

      {/* Validation strip */}
      {(zoneHasErrors || zoneHasWarnings) && (
        <div
          style={VALIDATION_STRIP}
          role="alert"
          aria-label={`Validation issues for zone ${zone.name}`}
        >
          {errors.map((err) => (
            <div key={err.code} style={{ ...VALIDATION_ITEM, color: '#f05d5e' }}>
              <span aria-hidden="true" style={{ fontSize: '12px', flexShrink: 0, marginTop: '1px' }}>✗</span>
              <span>{err.message}</span>
            </div>
          ))}
          {warnings.map((warn) => (
            <div key={warn.code} style={{ ...VALIDATION_ITEM, color: '#f5b942' }}>
              <span aria-hidden="true" style={{ fontSize: '12px', flexShrink: 0, marginTop: '1px' }}>⚠</span>
              <span>{warn.message}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
};

// Extracted to avoid duplication
const SECTION_DIVIDER_STYLE: React.CSSProperties = {
  fontSize: '10px',
  fontWeight: 700,
  letterSpacing: '0.08em',
  textTransform: 'uppercase',
  color: '#445566',
  margin: '6px 0 6px 0',
};

export { AvailabilityZonePanel };
export default AvailabilityZonePanel;
