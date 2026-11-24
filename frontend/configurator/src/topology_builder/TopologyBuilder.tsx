/**
 * TopologyBuilder — main orchestrator component for the Stellar-K8s topology
 * configurator.
 *
 * Wires together:
 *   - `useTopology()` for shared state + dispatch
 *   - `StellarNodePlacer` sidebar — node-type palette + inline config form
 *   - `AvailabilityZonePanel` columns — drop targets for placed nodes
 *   - `validateTopology` — real-time quorum validation
 *   - `buildManifests` — YAML manifest generation + modal display
 *
 * Drag-and-drop protocol:
 *   1. User clicks a node tile in the palette to open the config panel.
 *   2. User fills in fields and clicks "Save Configuration". This stores the
 *      config in `pendingNodeConfig` local state.
 *   3. User drags the tile onto a zone. `onDrop` on the zone fires, reads the
 *      `DragPayload` from `dataTransfer`, and (if the payload is a 'node-type'
 *      drag with a pending config) dispatches `PLACE_NODE` to the store.
 *   4. If `type === 'placed-node'` (moving an existing node), the handler
 *      moves it by removing + re-placing it in the new zone.
 */

import React, { useState, useMemo, useCallback } from 'react';
import { useTopology } from './topology_store';
import { validateTopology } from './quorum_validator';
import { buildManifests } from '../../../utils/manifest_builder';
import StellarNodePlacer, { type NodeConfigFields } from './StellarNodePlacer';
import AvailabilityZonePanel from './AvailabilityZone';
import type {
  NodeType,
  DragPayload,
  ValidationError,
  ValidationWarning,
} from './types';

// ---------------------------------------------------------------------------
// Generate a lightweight pseudo-UUID for new zone IDs
// ---------------------------------------------------------------------------
function genId(): string {
  return `z-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

// ---------------------------------------------------------------------------
// Styles
// ---------------------------------------------------------------------------

const ROOT: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  height: '100%',
  minHeight: '100vh',
  background: '#0b1119',
  color: '#e8edf2',
  fontFamily: "'Space Grotesk', sans-serif",
};

const TOPBAR: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'space-between',
  padding: '12px 20px',
  background: '#0d1623',
  borderBottom: '1px solid #1e2d3d',
  gap: '12px',
  flexWrap: 'wrap',
};

const TITLE_AREA: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: '3px',
  flex: '1 1 auto',
  minWidth: 0,
};

const TITLE: React.CSSProperties = {
  margin: 0,
  fontSize: '18px',
  fontWeight: 700,
  color: '#e8edf2',
  letterSpacing: '-0.01em',
};

const SUBTITLE: React.CSSProperties = {
  margin: 0,
  fontSize: '12px',
  color: '#7a8fa8',
  lineHeight: 1.4,
};

const TOOLBAR: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '8px',
  flexShrink: 0,
};

const BTN_BASE: React.CSSProperties = {
  padding: '7px 14px',
  borderRadius: '5px',
  border: '1px solid #273340',
  background: '#111b27',
  color: '#c8d8e8',
  cursor: 'pointer',
  fontSize: '12px',
  fontWeight: 600,
  fontFamily: "'Space Grotesk', sans-serif",
  transition: 'border-color 0.15s, color 0.15s, background 0.15s',
  whiteSpace: 'nowrap',
};

const BTN_PRIMARY: React.CSSProperties = {
  ...BTN_BASE,
  background: '#39d98a',
  color: '#0b1119',
  border: '1px solid #39d98a',
};

const BTN_DANGER: React.CSSProperties = {
  ...BTN_BASE,
  color: '#f05d5e',
  borderColor: '#f05d5e44',
};

const BTN_DISABLED: React.CSSProperties = {
  ...BTN_BASE,
  background: '#111b27',
  color: '#334455',
  borderColor: '#1e2d3d',
  cursor: 'not-allowed',
};

const BODY: React.CSSProperties = {
  display: 'flex',
  flex: '1 1 auto',
  overflow: 'hidden',
};

const SIDEBAR: React.CSSProperties = {
  width: '260px',
  flexShrink: 0,
  borderRight: '1px solid #1e2d3d',
  background: '#0d1623',
  padding: '16px 14px',
  overflowY: 'auto',
};

const MAIN: React.CSSProperties = {
  flex: '1 1 auto',
  display: 'flex',
  flexDirection: 'column',
  overflow: 'hidden',
};

const ZONES_GRID: React.CSSProperties = {
  flex: '1 1 auto',
  display: 'grid',
  gridTemplateColumns: 'repeat(3, 1fr)',
  gap: '14px',
  padding: '16px',
  overflowY: 'auto',
  alignContent: 'start',
};

const VALIDATION_PANEL: React.CSSProperties = {
  background: '#0d1623',
  borderTop: '1px solid #1e2d3d',
  padding: '12px 16px',
  maxHeight: '180px',
  overflowY: 'auto',
};

const VALIDATION_HEADER: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '8px',
  marginBottom: '8px',
};

const VALIDATION_TITLE: React.CSSProperties = {
  fontSize: '11px',
  fontWeight: 700,
  letterSpacing: '0.07em',
  textTransform: 'uppercase',
  color: '#556677',
  margin: 0,
};

const VALIDATION_ITEM: React.CSSProperties = {
  display: 'flex',
  alignItems: 'flex-start',
  gap: '6px',
  fontSize: '12px',
  lineHeight: 1.45,
  padding: '3px 0',
};

// Status indicator (header badge)
const STATUS_BADGE: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '5px',
  padding: '4px 10px',
  borderRadius: '12px',
  fontSize: '11px',
  fontWeight: 700,
  letterSpacing: '0.04em',
};

// Modal overlay
const MODAL_OVERLAY: React.CSSProperties = {
  position: 'fixed',
  inset: 0,
  background: 'rgba(0,0,0,0.72)',
  zIndex: 1000,
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  padding: '24px',
};

const MODAL: React.CSSProperties = {
  background: '#0d1623',
  border: '1px solid #273340',
  borderRadius: '10px',
  width: '100%',
  maxWidth: '820px',
  maxHeight: '90vh',
  display: 'flex',
  flexDirection: 'column',
  overflow: 'hidden',
  boxShadow: '0 24px 64px rgba(0,0,0,0.7)',
};

const MODAL_HEADER: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'space-between',
  padding: '14px 18px',
  borderBottom: '1px solid #1e2d3d',
  flexShrink: 0,
};

const MODAL_TITLE: React.CSSProperties = {
  margin: 0,
  fontSize: '15px',
  fontWeight: 700,
  color: '#e8edf2',
};

const MODAL_BODY: React.CSSProperties = {
  flex: '1 1 auto',
  overflow: 'auto',
  padding: '0',
};

const YAML_PRE: React.CSSProperties = {
  margin: 0,
  padding: '16px 18px',
  fontSize: '12px',
  lineHeight: 1.6,
  fontFamily: "'DM Mono', monospace",
  color: '#b8cfe8',
  background: '#080f18',
  whiteSpace: 'pre',
  overflowX: 'auto',
  minHeight: '200px',
};

const MODAL_FOOTER: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'flex-end',
  gap: '8px',
  padding: '12px 18px',
  borderTop: '1px solid #1e2d3d',
  flexShrink: 0,
};

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

const TopologyBuilder: React.FC = () => {
  const [state, dispatch] = useTopology();

  // Pending node configuration set by the StellarNodePlacer palette
  const [pendingNodeType, setPendingNodeType] = useState<NodeType | null>(null);
  const [pendingNodeConfig, setPendingNodeConfig] = useState<NodeConfigFields | null>(null);

  // Manifest modal state
  const [showManifest, setShowManifest] = useState(false);
  const [generatedYaml, setGeneratedYaml] = useState('');
  const [copySuccess, setCopySuccess] = useState(false);

  // Run real-time validation
  const validation = useMemo(() => validateTopology(state), [state]);

  // Pre-compute per-zone validation messages for the zone panels
  const zoneErrors = useMemo<Record<string, ValidationError[]>>(() => {
    const map: Record<string, ValidationError[]> = {};
    for (const err of validation.errors) {
      for (const zid of err.zoneIds) {
        (map[zid] ??= []).push(err);
      }
    }
    return map;
  }, [validation.errors]);

  const zoneWarnings = useMemo<Record<string, ValidationWarning[]>>(() => {
    const map: Record<string, ValidationWarning[]> = {};
    for (const warn of validation.warnings) {
      for (const zid of warn.zoneIds) {
        (map[zid] ??= []).push(warn);
      }
    }
    return map;
  }, [validation.warnings]);

  // ---------------------------------------------------------------------------
  // Callbacks
  // ---------------------------------------------------------------------------

  /** Called when the user saves a node config in the StellarNodePlacer. */
  const handleNodeConfigured = useCallback(
    (nodeType: NodeType, config: NodeConfigFields) => {
      setPendingNodeType(nodeType);
      setPendingNodeConfig(config);
    },
    [],
  );

  /** Called when a drag payload is dropped onto a zone panel. */
  const handleZoneDrop = useCallback(
    (zoneId: string, payload: DragPayload) => {
      if (payload.type === 'node-type') {
        // New node from palette — requires pending config
        if (!pendingNodeConfig || !pendingNodeType) {
          console.warn(
            '[TopologyBuilder] node-type drop received but no pending config. ' +
              'Configure the node in the palette before dropping.',
          );
          return;
        }
        dispatch({
          type: 'PLACE_NODE',
          payload: {
            zoneId,
            nodeType: pendingNodeType,
            name: pendingNodeConfig.name,
            namespace: pendingNodeConfig.namespace,
            network: pendingNodeConfig.network,
            version: pendingNodeConfig.version,
            replicas: pendingNodeConfig.replicas,
            resources: {
              cpu: pendingNodeConfig.cpuRequest,
              memory: pendingNodeConfig.memoryRequest,
            },
            storage: {
              storageClass: pendingNodeConfig.storageClass,
              size: pendingNodeConfig.storageSize,
              mode: 'PersistentVolume',
              retentionPolicy: 'Retain',
            },
            ...(pendingNodeType === 'Validator'
              ? {
                  validatorConfig: {
                    seedSecretRef: pendingNodeConfig.seedSecretRef,
                    enableHistoryArchive: pendingNodeConfig.enableHistoryArchive,
                    quorumSet: pendingNodeConfig.quorumSet || undefined,
                  },
                }
              : {}),
          },
        });
        // Clear pending after successful place
        setPendingNodeType(null);
        setPendingNodeConfig(null);
      } else if (payload.type === 'placed-node' && payload.placedNodeId) {
        // Moving an existing placed node to a different zone
        dispatch({
          type: 'UPDATE_PLACED_NODE',
          payload: {
            nodeId: payload.placedNodeId,
            updates: { availabilityZoneId: zoneId },
          },
        });
      }
    },
    [dispatch, pendingNodeConfig, pendingNodeType],
  );

  /** Remove a placed node from any zone. */
  const handleRemoveNode = useCallback(
    (nodeId: string) => {
      dispatch({ type: 'REMOVE_PLACED_NODE', payload: { nodeId } });
    },
    [dispatch],
  );

  /** Add a new empty availability zone. */
  const handleAddZone = useCallback(() => {
    const idx = state.zones.length + 1;
    const id = genId();
    dispatch({
      type: 'ADD_ZONE',
      payload: {
        id,
        name: `zone-${idx}`,
        region: 'custom',
      },
    });
  }, [dispatch, state.zones.length]);

  /** Reset topology to initial state. */
  const handleReset = useCallback(() => {
    if (
      state.placedNodes.length === 0 ||
      window.confirm(
        'Reset the topology? All placed nodes will be removed and zones reset to defaults.',
      )
    ) {
      dispatch({ type: 'RESET' });
      setPendingNodeType(null);
      setPendingNodeConfig(null);
    }
  }, [dispatch, state.placedNodes.length]);

  /** Generate YAML and open the manifest modal. */
  const handleGenerateManifest = useCallback(() => {
    const yaml = buildManifests(state);
    setGeneratedYaml(yaml || '# No nodes placed yet.\n');
    setShowManifest(true);
  }, [state]);

  /** Copy YAML to clipboard. */
  const handleCopy = useCallback(() => {
    navigator.clipboard
      .writeText(generatedYaml)
      .then(() => {
        setCopySuccess(true);
        setTimeout(() => setCopySuccess(false), 2000);
      })
      .catch(() => {
        // Fallback: select the pre element text
        const el = document.getElementById('manifest-yaml');
        if (el) {
          const range = document.createRange();
          range.selectNodeContents(el);
          window.getSelection()?.removeAllRanges();
          window.getSelection()?.addRange(range);
        }
      });
  }, [generatedYaml]);

  // ---------------------------------------------------------------------------
  // Status indicator
  // ---------------------------------------------------------------------------

  let statusLabel: string;
  let statusColor: string;
  let statusIcon: string;
  let statusBg: string;

  if (validation.errors.length > 0) {
    statusLabel = `${validation.errors.length} error${validation.errors.length > 1 ? 's' : ''}`;
    statusColor = '#f05d5e';
    statusIcon = '✗';
    statusBg = 'rgba(240,93,94,0.12)';
  } else if (validation.warnings.length > 0) {
    statusLabel = `${validation.warnings.length} warning${validation.warnings.length > 1 ? 's' : ''}`;
    statusColor = '#f5b942';
    statusIcon = '⚠';
    statusBg = 'rgba(245,185,66,0.1)';
  } else {
    statusLabel = 'Valid';
    statusColor = '#39d98a';
    statusIcon = '✓';
    statusBg = 'rgba(57,217,138,0.1)';
  }

  const canGenerate = state.placedNodes.length > 0;

  // ---------------------------------------------------------------------------
  // Render
  // ---------------------------------------------------------------------------

  return (
    <div style={ROOT} role="application" aria-label="Stellar-K8s Topology Configurator">
      {/* ---------------------------------------------------------------- */}
      {/* Top bar                                                           */}
      {/* ---------------------------------------------------------------- */}
      <header style={TOPBAR} role="banner">
        <div style={TITLE_AREA}>
          <h1 style={TITLE}>Topology Configurator</h1>
          <p style={SUBTITLE}>
            Drag node types from the palette onto availability zones. Configure each node
            before dropping. Validate and export manifests.
          </p>
        </div>

        <div style={TOOLBAR} role="toolbar" aria-label="Topology actions">
          {/* Validation status badge */}
          <div
            style={{
              ...STATUS_BADGE,
              background: statusBg,
              color: statusColor,
              border: `1px solid ${statusColor}33`,
            }}
            role="status"
            aria-live="polite"
            aria-label={`Validation status: ${statusLabel}`}
          >
            <span aria-hidden="true">{statusIcon}</span>
            <span>{statusLabel}</span>
          </div>

          {/* Add Zone */}
          <button
            style={BTN_BASE}
            onClick={handleAddZone}
            aria-label="Add a new availability zone"
            title="Add Zone"
          >
            + Add Zone
          </button>

          {/* Reset */}
          <button
            style={BTN_DANGER}
            onClick={handleReset}
            aria-label="Reset topology to defaults"
            title="Reset topology"
          >
            ↺ Reset
          </button>

          {/* Generate Manifest */}
          <button
            style={canGenerate ? BTN_PRIMARY : BTN_DISABLED}
            onClick={canGenerate ? handleGenerateManifest : undefined}
            disabled={!canGenerate}
            aria-label={
              canGenerate
                ? 'Generate Kubernetes manifests for all placed nodes'
                : 'Place at least one node to generate manifests'
            }
            aria-disabled={!canGenerate}
            title={canGenerate ? 'Generate Manifests' : 'No nodes placed'}
          >
            ⬇ Generate Manifest
          </button>
        </div>
      </header>

      {/* ---------------------------------------------------------------- */}
      {/* Body: sidebar + zone grid                                         */}
      {/* ---------------------------------------------------------------- */}
      <div style={BODY}>
        {/* Left sidebar: node type palette */}
        <nav style={SIDEBAR} aria-label="Node type palette">
          <StellarNodePlacer
            onNodeConfigured={handleNodeConfigured}
            selectedNodeType={pendingNodeType}
          />

          {/* Pending config indicator */}
          {pendingNodeConfig && pendingNodeType && (
            <div
              style={{
                marginTop: '12px',
                padding: '8px 10px',
                background: 'rgba(57,217,138,0.07)',
                border: '1px solid rgba(57,217,138,0.25)',
                borderRadius: '5px',
                fontSize: '11px',
                color: '#39d98a',
              }}
              role="status"
              aria-live="polite"
            >
              <strong>Ready to place:</strong> {pendingNodeConfig.name}{' '}
              ({pendingNodeType})
              <br />
              <span style={{ color: '#7a8fa8' }}>
                Drag the{' '}
                <em style={{ fontStyle: 'normal', fontWeight: 600 }}>
                  {pendingNodeType}
                </em>{' '}
                tile onto a zone below.
              </span>
            </div>
          )}
        </nav>

        {/* Right: zone canvas + validation panel */}
        <div style={MAIN}>
          {/* Zone grid */}
          <main style={ZONES_GRID} aria-label="Availability zones canvas">
            {state.zones.length === 0 && (
              <div
                style={{
                  gridColumn: '1 / -1',
                  display: 'flex',
                  flexDirection: 'column',
                  alignItems: 'center',
                  justifyContent: 'center',
                  padding: '48px 24px',
                  color: '#334455',
                  gap: '8px',
                  textAlign: 'center',
                }}
                role="status"
              >
                <span style={{ fontSize: '32px' }} aria-hidden="true">⊞</span>
                <p style={{ margin: 0, fontSize: '14px' }}>No availability zones defined.</p>
                <p style={{ margin: 0, fontSize: '12px' }}>
                  Click <strong style={{ color: '#556677' }}>+ Add Zone</strong> to create one.
                </p>
              </div>
            )}

            {state.zones.map((zone) => {
              const placedInZone = state.placedNodes.filter(
                (n) => n.availabilityZoneId === zone.id,
              );
              const workersInZone = state.workerNodes.filter((w) =>
                zone.workerNodeIds.includes(w.id),
              );

              return (
                <AvailabilityZonePanel
                  key={zone.id}
                  zone={zone}
                  placedNodes={placedInZone}
                  workerNodes={workersInZone}
                  pendingNodeConfig={pendingNodeConfig as Record<string, unknown> | null}
                  onDrop={handleZoneDrop}
                  onRemoveNode={handleRemoveNode}
                  errors={zoneErrors[zone.id] ?? []}
                  warnings={zoneWarnings[zone.id] ?? []}
                />
              );
            })}
          </main>

          {/* Validation panel */}
          <aside
            style={VALIDATION_PANEL}
            aria-label="Validation results"
            role="complementary"
          >
            <div style={VALIDATION_HEADER}>
              <h2 style={VALIDATION_TITLE}>Validation</h2>
              <span
                style={{
                  fontSize: '11px',
                  color: '#556677',
                  marginLeft: 'auto',
                }}
              >
                {state.placedNodes.length} node{state.placedNodes.length !== 1 ? 's' : ''} placed
                {state.zones.length > 0 && ` · ${state.zones.length} zone${state.zones.length !== 1 ? 's' : ''}`}
              </span>
            </div>

            {validation.errors.length === 0 && validation.warnings.length === 0 && (
              <p
                style={{ margin: 0, fontSize: '12px', color: '#39d98a' }}
                role="status"
              >
                ✓ No issues found. Topology looks good.
              </p>
            )}

            {/* Errors */}
            {validation.errors.map((err) => (
              <div
                key={err.code}
                style={{ ...VALIDATION_ITEM, color: '#f05d5e' }}
                role="alert"
              >
                <span aria-hidden="true" style={{ fontSize: '13px', flexShrink: 0, marginTop: '1px' }}>
                  ✗
                </span>
                <div>
                  <strong style={{ fontFamily: "'DM Mono', monospace", fontSize: '11px' }}>
                    {err.code}
                  </strong>
                  <span style={{ color: '#d8888a' }}> — {err.message}</span>
                </div>
              </div>
            ))}

            {/* Warnings */}
            {validation.warnings.map((warn) => (
              <div
                key={warn.code}
                style={{ ...VALIDATION_ITEM, color: '#f5b942' }}
                role="note"
              >
                <span aria-hidden="true" style={{ fontSize: '13px', flexShrink: 0, marginTop: '1px' }}>
                  ⚠
                </span>
                <div>
                  <strong style={{ fontFamily: "'DM Mono', monospace", fontSize: '11px' }}>
                    {warn.code}
                  </strong>
                  <span style={{ color: '#d8a840' }}> — {warn.message}</span>
                </div>
              </div>
            ))}
          </aside>
        </div>
      </div>

      {/* ---------------------------------------------------------------- */}
      {/* Manifest modal                                                    */}
      {/* ---------------------------------------------------------------- */}
      {showManifest && (
        <div
          style={MODAL_OVERLAY}
          role="dialog"
          aria-modal="true"
          aria-label="Generated Kubernetes Manifests"
          onClick={(e) => {
            // Close when clicking the backdrop (not the modal itself)
            if (e.target === e.currentTarget) setShowManifest(false);
          }}
          onKeyDown={(e) => {
            if (e.key === 'Escape') setShowManifest(false);
          }}
        >
          <div style={MODAL} onClick={(e) => e.stopPropagation()}>
            {/* Modal header */}
            <div style={MODAL_HEADER}>
              <h2 style={MODAL_TITLE}>Generated Kubernetes Manifests</h2>
              <button
                style={{
                  ...BTN_BASE,
                  padding: '4px 10px',
                  fontSize: '16px',
                  lineHeight: 1,
                  color: '#7a8fa8',
                }}
                onClick={() => setShowManifest(false)}
                aria-label="Close manifest modal"
                title="Close"
                autoFocus
              >
                ✕
              </button>
            </div>

            {/* YAML content */}
            <div style={MODAL_BODY}>
              <pre style={YAML_PRE} id="manifest-yaml">
                <code>{generatedYaml}</code>
              </pre>
            </div>

            {/* Modal footer */}
            <div style={MODAL_FOOTER}>
              <span
                style={{
                  fontSize: '11px',
                  color: copySuccess ? '#39d98a' : '#556677',
                  transition: 'color 0.2s',
                  marginRight: 'auto',
                }}
                role="status"
                aria-live="polite"
              >
                {copySuccess ? '✓ Copied to clipboard!' : `${state.placedNodes.length} node manifest${state.placedNodes.length !== 1 ? 's' : ''} · ${state.placedNodes.length * 2} resources`}
              </span>

              <button
                style={{
                  ...BTN_BASE,
                  borderColor: copySuccess ? '#39d98a44' : '#273340',
                  color: copySuccess ? '#39d98a' : '#c8d8e8',
                }}
                onClick={handleCopy}
                aria-label="Copy YAML to clipboard"
              >
                {copySuccess ? '✓ Copied!' : '⎘ Copy to Clipboard'}
              </button>

              <button
                style={BTN_PRIMARY}
                onClick={() => setShowManifest(false)}
                aria-label="Close manifest modal"
              >
                Done
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};

export default TopologyBuilder;
