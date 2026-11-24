/**
 * StellarNodePlacer — palette sidebar for dragging Stellar node types onto
 * availability zones.
 *
 * Renders three draggable tiles (Validator, Horizon, SorobanRpc) and an
 * inline configuration panel that expands when a tile is clicked. The panel
 * collects all required fields for a PlacedStellarNode before the user drops
 * it onto a zone.
 *
 * Drag protocol:
 *   Each tile sets `application/x-stellar-drag` on `dataTransfer` with a
 *   `DragPayload` of `{ type: 'node-type', nodeType, placedNodeId: null }`.
 *   The parent (TopologyBuilder) is responsible for reading this payload on
 *   the drop event and dispatching PLACE_NODE with the configured fields.
 */

import React, { useState } from 'react';
import type { NodeType, StellarNetwork, DragPayload } from './types';

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/** All form fields collected before a node is placed on a zone. */
export interface NodeConfigFields {
  name: string;
  namespace: string;
  network: StellarNetwork;
  version: string;
  replicas: number;
  cpuRequest: string;
  memoryRequest: string;
  storageClass: string;
  storageSize: string;
  // Validator-only
  seedSecretRef: string;
  enableHistoryArchive: boolean;
  quorumSet: string;
}

export interface StellarNodePlacerProps {
  /** Callback fired when the user finishes configuring a node type. */
  onNodeConfigured: (nodeType: NodeType, config: NodeConfigFields) => void;
  /** Currently selected node type (controlled from parent). */
  selectedNodeType: NodeType | null;
}

// ---------------------------------------------------------------------------
// Node type metadata
// ---------------------------------------------------------------------------

interface NodeTypeMeta {
  type: NodeType;
  label: string;
  description: string;
  accent: string;
}

const NODE_TYPES: NodeTypeMeta[] = [
  {
    type: 'Validator',
    label: 'Validator',
    description: 'Participates in SCP consensus. Requires a signing-key secret and quorum configuration.',
    accent: '#39d98a',
  },
  {
    type: 'Horizon',
    label: 'Horizon',
    description: 'REST API server that ingests ledger data and serves transaction history.',
    accent: '#4ea8de',
  },
  {
    type: 'SorobanRpc',
    label: 'Soroban RPC',
    description: 'Smart contract execution node. Provides JSON-RPC for Soroban contract invocations.',
    accent: '#9d7cd8',
  },
];

// ---------------------------------------------------------------------------
// Default form values
// ---------------------------------------------------------------------------

const DEFAULT_FIELDS: NodeConfigFields = {
  name: '',
  namespace: 'stellar',
  network: 'testnet',
  version: 'v21.0.0',
  replicas: 1,
  cpuRequest: '500m',
  memoryRequest: '1Gi',
  storageClass: 'standard',
  storageSize: '100Gi',
  seedSecretRef: '',
  enableHistoryArchive: false,
  quorumSet: '',
};

// ---------------------------------------------------------------------------
// Styles
// ---------------------------------------------------------------------------

const SIDEBAR: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: '0',
  fontFamily: "'Space Grotesk', sans-serif",
  height: '100%',
};

const HEADER: React.CSSProperties = {
  fontSize: '11px',
  fontWeight: 700,
  letterSpacing: '0.08em',
  textTransform: 'uppercase',
  color: '#556677',
  padding: '0 0 10px 0',
  borderBottom: '1px solid #273340',
  marginBottom: '12px',
};

const TILE_BASE: React.CSSProperties = {
  borderRadius: '6px',
  padding: '11px 12px',
  cursor: 'grab',
  userSelect: 'none',
  border: '1px solid #273340',
  background: '#111b27',
  marginBottom: '8px',
  transition: 'border-color 0.15s, background 0.15s, box-shadow 0.15s',
  position: 'relative',
};

const TILE_LABEL: React.CSSProperties = {
  fontWeight: 700,
  fontSize: '13px',
  color: '#e8edf2',
  margin: 0,
};

const TILE_DESC: React.CSSProperties = {
  fontSize: '11px',
  color: '#7a8fa8',
  margin: '3px 0 0 0',
  lineHeight: 1.4,
};

const CONFIG_PANEL: React.CSSProperties = {
  background: '#0e1820',
  border: '1px solid #273340',
  borderRadius: '8px',
  padding: '16px',
  marginTop: '4px',
  marginBottom: '4px',
};

const CONFIG_TITLE: React.CSSProperties = {
  fontSize: '12px',
  fontWeight: 700,
  color: '#e8edf2',
  margin: '0 0 12px 0',
  letterSpacing: '0.04em',
};

const HINT_TEXT: React.CSSProperties = {
  fontSize: '11px',
  color: '#f5b942',
  background: 'rgba(245,185,66,0.08)',
  border: '1px solid rgba(245,185,66,0.2)',
  borderRadius: '4px',
  padding: '6px 8px',
  marginBottom: '12px',
  display: 'flex',
  alignItems: 'flex-start',
  gap: '5px',
};

const FIELD_GROUP: React.CSSProperties = {
  marginBottom: '10px',
};

const LABEL_STYLE: React.CSSProperties = {
  display: 'block',
  fontSize: '11px',
  color: '#7a8fa8',
  marginBottom: '4px',
  fontWeight: 500,
};

const INPUT_STYLE: React.CSSProperties = {
  width: '100%',
  background: '#111b27',
  border: '1px solid #273340',
  borderRadius: '4px',
  color: '#e8edf2',
  padding: '6px 8px',
  fontSize: '12px',
  fontFamily: "'DM Mono', monospace",
  outline: 'none',
  boxSizing: 'border-box',
  transition: 'border-color 0.15s',
};

const SELECT_STYLE: React.CSSProperties = {
  ...INPUT_STYLE,
  cursor: 'pointer',
  appearance: 'none',
  backgroundImage: `url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='6'%3E%3Cpath d='M0 0l5 6 5-6z' fill='%23556677'/%3E%3C/svg%3E")`,
  backgroundRepeat: 'no-repeat',
  backgroundPosition: 'right 8px center',
  paddingRight: '24px',
};

const CHECKBOX_ROW: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '8px',
  cursor: 'pointer',
};

const TEXTAREA_STYLE: React.CSSProperties = {
  ...INPUT_STYLE,
  resize: 'vertical',
  minHeight: '72px',
  lineHeight: 1.4,
};

const BTN_CONFIRM: React.CSSProperties = {
  width: '100%',
  padding: '8px 12px',
  borderRadius: '5px',
  border: 'none',
  cursor: 'pointer',
  fontSize: '12px',
  fontWeight: 600,
  fontFamily: "'Space Grotesk', sans-serif",
  marginTop: '4px',
  transition: 'opacity 0.15s',
};

const SECTION_DIVIDER: React.CSSProperties = {
  fontSize: '10px',
  fontWeight: 700,
  letterSpacing: '0.08em',
  textTransform: 'uppercase',
  color: '#445566',
  margin: '10px 0 8px 0',
};

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

const StellarNodePlacer: React.FC<StellarNodePlacerProps> = ({
  onNodeConfigured,
  selectedNodeType,
}) => {
  const [activeType, setActiveType] = useState<NodeType | null>(null);
  const [fields, setFields] = useState<NodeConfigFields>({ ...DEFAULT_FIELDS });
  const [hoveredType, setHoveredType] = useState<NodeType | null>(null);
  const [draggingType, setDraggingType] = useState<NodeType | null>(null);

  // Merge external selectedNodeType selection (controlled from parent)
  const effectiveActive = activeType ?? selectedNodeType;

  const setField = <K extends keyof NodeConfigFields>(key: K, value: NodeConfigFields[K]) => {
    setFields((prev) => ({ ...prev, [key]: value }));
  };

  const handleTileClick = (nodeType: NodeType) => {
    if (activeType === nodeType) {
      setActiveType(null);
    } else {
      setActiveType(nodeType);
      setFields({ ...DEFAULT_FIELDS });
    }
  };

  const handleDragStart = (nodeType: NodeType, event: React.DragEvent<HTMLDivElement>) => {
    const payload: DragPayload = {
      type: 'node-type',
      nodeType,
      placedNodeId: null,
    };
    event.dataTransfer.setData('application/x-stellar-drag', JSON.stringify(payload));
    event.dataTransfer.effectAllowed = 'copy';
    setDraggingType(nodeType);
  };

  const handleDragEnd = () => {
    setDraggingType(null);
  };

  const handleConfirm = () => {
    if (!effectiveActive) return;
    if (!fields.name.trim()) {
      // Require name before confirming
      return;
    }
    onNodeConfigured(effectiveActive, { ...fields });
    setActiveType(null);
    setFields({ ...DEFAULT_FIELDS });
  };

  return (
    <aside style={SIDEBAR} role="complementary" aria-label="Node type palette">
      <p style={HEADER}>Node Types</p>

      {NODE_TYPES.map((meta) => {
        const isActive = effectiveActive === meta.type;
        const isDragging = draggingType === meta.type;
        const isHovered = hoveredType === meta.type;

        const tileStyle: React.CSSProperties = {
          ...TILE_BASE,
          borderLeftWidth: '3px',
          borderLeftColor: meta.accent,
          ...(isDragging ? { opacity: 0.5 } : {}),
          ...(isActive
            ? {
                background: '#162030',
                borderColor: meta.accent,
                boxShadow: `0 0 0 1px ${meta.accent}33`,
              }
            : isHovered
            ? {
                background: '#152030',
                borderLeftColor: meta.accent,
                boxShadow: `0 2px 8px ${meta.accent}22`,
              }
            : {}),
        };

        return (
          <div key={meta.type}>
            {/* Draggable tile */}
            <div
              style={tileStyle}
              draggable
              onDragStart={(e) => handleDragStart(meta.type, e)}
              onDragEnd={handleDragEnd}
              onMouseEnter={() => setHoveredType(meta.type)}
              onMouseLeave={() => setHoveredType(null)}
              onClick={() => handleTileClick(meta.type)}
              role="button"
              tabIndex={0}
              aria-label={`${meta.label} node type. Click to configure, drag to place on a zone.`}
              aria-pressed={isActive}
              aria-grabbed={isDragging}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  handleTileClick(meta.type);
                }
              }}
            >
              {/* Left-side color dot */}
              <span
                style={{
                  display: 'inline-block',
                  width: '7px',
                  height: '7px',
                  borderRadius: '50%',
                  background: meta.accent,
                  marginRight: '7px',
                  verticalAlign: 'middle',
                  marginBottom: '1px',
                }}
                aria-hidden="true"
              />
              <span style={TILE_LABEL}>{meta.label}</span>
              <p style={TILE_DESC}>{meta.description}</p>
            </div>

            {/* Inline config panel — expands when this tile is active */}
            {isActive && (
              <div
                style={{
                  ...CONFIG_PANEL,
                  borderColor: `${meta.accent}44`,
                  boxShadow: `0 0 0 1px ${meta.accent}22`,
                }}
                role="form"
                aria-label={`Configure ${meta.label} node`}
              >
                <h3 style={{ ...CONFIG_TITLE, color: meta.accent }}>
                  Configure {meta.label}
                </h3>

                {/* Hint */}
                <div style={HINT_TEXT} role="note" aria-live="polite">
                  <span aria-hidden="true">⚠</span>
                  <span>Configure before placing — drag this tile onto a zone after saving.</span>
                </div>

                {/* Basic fields */}
                <p style={SECTION_DIVIDER}>Basic</p>

                <div style={FIELD_GROUP}>
                  <label style={LABEL_STYLE} htmlFor={`node-name-${meta.type}`}>
                    Name <span style={{ color: '#f05d5e' }}>*</span>
                  </label>
                  <input
                    id={`node-name-${meta.type}`}
                    style={{
                      ...INPUT_STYLE,
                      borderColor: !fields.name.trim() ? '#f05d5e55' : '#273340',
                    }}
                    type="text"
                    value={fields.name}
                    onChange={(e) => setField('name', e.target.value)}
                    placeholder="my-validator-1"
                    aria-required="true"
                    aria-invalid={!fields.name.trim()}
                    aria-describedby={!fields.name.trim() ? `name-error-${meta.type}` : undefined}
                  />
                  {!fields.name.trim() && (
                    <span
                      id={`name-error-${meta.type}`}
                      style={{ fontSize: '10px', color: '#f05d5e', marginTop: '2px', display: 'block' }}
                      role="alert"
                    >
                      Name is required
                    </span>
                  )}
                </div>

                <div style={FIELD_GROUP}>
                  <label style={LABEL_STYLE} htmlFor={`node-ns-${meta.type}`}>Namespace</label>
                  <input
                    id={`node-ns-${meta.type}`}
                    style={INPUT_STYLE}
                    type="text"
                    value={fields.namespace}
                    onChange={(e) => setField('namespace', e.target.value)}
                    placeholder="stellar"
                  />
                </div>

                <div style={FIELD_GROUP}>
                  <label style={LABEL_STYLE} htmlFor={`node-network-${meta.type}`}>Network</label>
                  <select
                    id={`node-network-${meta.type}`}
                    style={SELECT_STYLE}
                    value={fields.network}
                    onChange={(e) => setField('network', e.target.value as StellarNetwork)}
                    aria-label="Stellar network"
                  >
                    <option value="mainnet">mainnet</option>
                    <option value="testnet">testnet</option>
                    <option value="futurenet">futurenet</option>
                  </select>
                </div>

                <div style={FIELD_GROUP}>
                  <label style={LABEL_STYLE} htmlFor={`node-version-${meta.type}`}>Version</label>
                  <input
                    id={`node-version-${meta.type}`}
                    style={INPUT_STYLE}
                    type="text"
                    value={fields.version}
                    onChange={(e) => setField('version', e.target.value)}
                    placeholder="v21.0.0"
                  />
                </div>

                <div style={FIELD_GROUP}>
                  <label style={LABEL_STYLE} htmlFor={`node-replicas-${meta.type}`}>Replicas</label>
                  <input
                    id={`node-replicas-${meta.type}`}
                    style={INPUT_STYLE}
                    type="number"
                    min={1}
                    max={10}
                    value={fields.replicas}
                    onChange={(e) => setField('replicas', Math.max(1, parseInt(e.target.value, 10) || 1))}
                    aria-label="Replica count"
                  />
                </div>

                {/* Resources */}
                <p style={SECTION_DIVIDER}>Resources</p>

                <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '8px' }}>
                  <div style={FIELD_GROUP}>
                    <label style={LABEL_STYLE} htmlFor={`node-cpu-${meta.type}`}>CPU Request</label>
                    <input
                      id={`node-cpu-${meta.type}`}
                      style={INPUT_STYLE}
                      type="text"
                      value={fields.cpuRequest}
                      onChange={(e) => setField('cpuRequest', e.target.value)}
                      placeholder="500m"
                    />
                  </div>
                  <div style={FIELD_GROUP}>
                    <label style={LABEL_STYLE} htmlFor={`node-mem-${meta.type}`}>Memory Request</label>
                    <input
                      id={`node-mem-${meta.type}`}
                      style={INPUT_STYLE}
                      type="text"
                      value={fields.memoryRequest}
                      onChange={(e) => setField('memoryRequest', e.target.value)}
                      placeholder="1Gi"
                    />
                  </div>
                </div>

                {/* Storage */}
                <p style={SECTION_DIVIDER}>Storage</p>

                <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '8px' }}>
                  <div style={FIELD_GROUP}>
                    <label style={LABEL_STYLE} htmlFor={`node-sc-${meta.type}`}>Storage Class</label>
                    <input
                      id={`node-sc-${meta.type}`}
                      style={INPUT_STYLE}
                      type="text"
                      value={fields.storageClass}
                      onChange={(e) => setField('storageClass', e.target.value)}
                      placeholder="standard"
                    />
                  </div>
                  <div style={FIELD_GROUP}>
                    <label style={LABEL_STYLE} htmlFor={`node-sz-${meta.type}`}>Storage Size</label>
                    <input
                      id={`node-sz-${meta.type}`}
                      style={INPUT_STYLE}
                      type="text"
                      value={fields.storageSize}
                      onChange={(e) => setField('storageSize', e.target.value)}
                      placeholder="100Gi"
                    />
                  </div>
                </div>

                {/* Validator-only fields */}
                {meta.type === 'Validator' && (
                  <>
                    <p style={SECTION_DIVIDER}>Validator Config</p>

                    <div style={FIELD_GROUP}>
                      <label style={LABEL_STYLE} htmlFor={`node-seed-${meta.type}`}>
                        Seed Secret Ref
                      </label>
                      <input
                        id={`node-seed-${meta.type}`}
                        style={INPUT_STYLE}
                        type="text"
                        value={fields.seedSecretRef}
                        onChange={(e) => setField('seedSecretRef', e.target.value)}
                        placeholder="my-validator-seed"
                        aria-describedby={`seed-hint-${meta.type}`}
                      />
                      <span
                        id={`seed-hint-${meta.type}`}
                        style={{ fontSize: '10px', color: '#556677', marginTop: '2px', display: 'block' }}
                      >
                        Pre-created Kubernetes Secret name in the same namespace.
                      </span>
                    </div>

                    <div style={FIELD_GROUP}>
                      <label style={CHECKBOX_ROW} htmlFor={`node-archive-${meta.type}`}>
                        <input
                          id={`node-archive-${meta.type}`}
                          type="checkbox"
                          checked={fields.enableHistoryArchive}
                          onChange={(e) => setField('enableHistoryArchive', e.target.checked)}
                          style={{ accentColor: meta.accent, width: '14px', height: '14px' }}
                          aria-label="Enable history archive publishing"
                        />
                        <span style={{ fontSize: '12px', color: '#c8d8e8' }}>
                          Enable History Archive
                        </span>
                      </label>
                    </div>

                    <div style={FIELD_GROUP}>
                      <label style={LABEL_STYLE} htmlFor={`node-quorum-${meta.type}`}>
                        Quorum Set (TOML/JSON)
                      </label>
                      <textarea
                        id={`node-quorum-${meta.type}`}
                        style={TEXTAREA_STYLE}
                        value={fields.quorumSet}
                        onChange={(e) => setField('quorumSet', e.target.value)}
                        placeholder={'[[QUORUM_SET]]\nTHRESHOLD_PERCENT=67\nVALIDATORS=["$public_key_1","$public_key_2"]'}
                        aria-label="Quorum set configuration in TOML or JSON format"
                        spellCheck={false}
                      />
                    </div>
                  </>
                )}

                {/* Confirm button */}
                <button
                  style={{
                    ...BTN_CONFIRM,
                    background: fields.name.trim() ? meta.accent : '#273340',
                    color: fields.name.trim() ? '#0b1119' : '#445566',
                    cursor: fields.name.trim() ? 'pointer' : 'not-allowed',
                  }}
                  onClick={handleConfirm}
                  disabled={!fields.name.trim()}
                  aria-label={`Save ${meta.label} configuration`}
                  aria-disabled={!fields.name.trim()}
                >
                  ✓ Save Configuration
                </button>
              </div>
            )}
          </div>
        );
      })}

      {/* Instructional footer */}
      <p
        style={{
          fontSize: '11px',
          color: '#445566',
          marginTop: 'auto',
          paddingTop: '16px',
          lineHeight: 1.5,
        }}
      >
        Click a tile to configure, then drag it onto an availability zone to place the node.
      </p>
    </aside>
  );
};

export default StellarNodePlacer;
