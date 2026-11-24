import React, { useState, useMemo } from 'react';
import { validateTopologyQuorum } from './quorum_validator.js';
import { generateTopologyManifestYaml } from '../../utils/manifest_builder.js';
import './TopologyBuilder.css';

const DEFAULT_ZONES = [
  { id: 'us-east-1a', name: 'us-east-1a' },
  { id: 'us-east-1b', name: 'us-east-1b' },
  { id: 'us-east-1c', name: 'us-east-1c' },
];

const DEFAULT_NODES = [
  { id: '1', name: 'validator-east-1a', nodeType: 'Validator', zone: 'us-east-1a', network: 'mainnet' },
  { id: '2', name: 'validator-east-1b', nodeType: 'Validator', zone: 'us-east-1b', network: 'mainnet' },
  { id: '3', name: 'validator-east-1c', nodeType: 'Validator', zone: 'us-east-1c', network: 'mainnet' },
  { id: '4', name: 'horizon-rpc-1', nodeType: 'Horizon', zone: 'us-east-1a', network: 'mainnet' },
  { id: '5', name: 'soroban-rpc-1', nodeType: 'SorobanRpc', zone: 'us-east-1b', network: 'mainnet' },
];

export function TopologyBuilder() {
  const [zones, setZones] = useState(DEFAULT_ZONES);
  const [nodes, setNodes] = useState(DEFAULT_NODES);
  const [spreadSettings, setSpreadSettings] = useState({
    maxSkew: 1,
    topologyKey: 'topology.kubernetes.io/zone',
    whenUnsatisfiable: 'DoNotSchedule',
  });
  const [draggedNodeId, setDraggedNodeId] = useState(null);
  const [dragOverZoneId, setDragOverZoneId] = useState(null);
  const [copiedStatus, setCopiedStatus] = useState(false);

  // New Node Form State
  const [newNodeName, setNewNodeName] = useState('');
  const [newNodeType, setNewNodeType] = useState('Validator');
  const [newNodeZone, setNewNodeZone] = useState('unassigned');

  // Real-time quorum validation
  const validationResult = useMemo(() => {
    return validateTopologyQuorum(zones, nodes, spreadSettings);
  }, [zones, nodes, spreadSettings]);

  // Generated YAML Manifest
  const manifestYaml = useMemo(() => {
    return generateTopologyManifestYaml({
      zones,
      nodes: nodes.filter(n => n.zone && n.zone !== 'unassigned'),
      spreadSettings,
      namespace: 'stellar',
    });
  }, [zones, nodes, spreadSettings]);

  // Drag and Drop Handlers
  const handleDragStart = (e, nodeId) => {
    setDraggedNodeId(nodeId);
    e.dataTransfer.setData('text/plain', nodeId);
    e.dataTransfer.effectAllowed = 'move';
  };

  const handleDragOver = (e, zoneId) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = 'move';
    if (dragOverZoneId !== zoneId) {
      setDragOverZoneId(zoneId);
    }
  };

  const handleDragLeave = (e, zoneId) => {
    if (dragOverZoneId === zoneId) {
      setDragOverZoneId(null);
    }
  };

  const handleDrop = (e, targetZoneId) => {
    e.preventDefault();
    setDragOverZoneId(null);
    const nodeId = e.dataTransfer.getData('text/plain') || draggedNodeId;
    if (!nodeId) return;

    setNodes(prev =>
      prev.map(node =>
        node.id === nodeId ? { ...node, zone: targetZoneId } : node
      )
    );
    setDraggedNodeId(null);
  };

  // Add Node Handler
  const handleAddNode = e => {
    e.preventDefault();
    const name = newNodeName.trim() || `${newNodeType.toLowerCase()}-${Date.now().toString().slice(-4)}`;
    const newNode = {
      id: String(Date.now()),
      name,
      nodeType: newNodeType,
      zone: newNodeZone === 'unassigned' ? '' : newNodeZone,
      network: 'mainnet',
    };
    setNodes(prev => [...prev, newNode]);
    setNewNodeName('');
  };

  // Remove Node Handler
  const handleRemoveNode = nodeId => {
    setNodes(prev => prev.filter(n => n.id !== nodeId));
  };

  // Add Zone Handler
  const handleAddZone = () => {
    const nextChar = String.fromCharCode(97 + zones.length); // a, b, c, d...
    const zoneId = `us-east-1${nextChar}`;
    setZones(prev => [...prev, { id: zoneId, name: zoneId }]);
  };

  // Copy YAML Handler
  const handleCopyYaml = () => {
    navigator.clipboard.writeText(manifestYaml);
    setCopiedStatus(true);
    setTimeout(() => setCopiedStatus(false), 2000);
  };

  return (
    <div className="topology-builder-container">
      {/* Header */}
      <header className="tb-header">
        <div className="tb-title-group">
          <h1>Dynamic Node Topology Spread Configurator</h1>
          <p>Visually place nodes across availability zones with real-time quorum validation & K8s manifest generation</p>
        </div>
        <button className="tb-btn tb-btn-primary" onClick={handleAddZone}>
          + Add Availability Zone
        </button>
      </header>

      {/* Metrics & Validation Status Bar */}
      <section className="tb-metrics-bar">
        <div className="tb-metric-card">
          <span className="tb-metric-label">Quorum Redundancy</span>
          <div className="tb-metric-value">
            {validationResult.isValid ? (
              <span className="status-badge success">✓ Quorum Safe</span>
            ) : (
              <span className="status-badge danger">⚠ Quorum Risk</span>
            )}
          </div>
        </div>

        <div className="tb-metric-card">
          <span className="tb-metric-label">Active Zones</span>
          <div className="tb-metric-value">
            {validationResult.activeZonesCount} / {zones.length}
          </div>
        </div>

        <div className="tb-metric-card">
          <span className="tb-metric-label">Validator Nodes</span>
          <div className="tb-metric-value">{validationResult.totalValidators}</div>
        </div>

        <div className="tb-metric-card">
          <span className="tb-metric-label">Zone Skew</span>
          <div className="tb-metric-value">
            {validationResult.skew}
            <span style={{ fontSize: '12px', color: 'var(--tb-text-secondary)', marginLeft: '6px' }}>
              (max: {spreadSettings.maxSkew})
            </span>
          </div>
        </div>
      </section>

      {/* Validation Alerts */}
      {(validationResult.errors.length > 0 || validationResult.warnings.length > 0) && (
        <section className="tb-alerts-panel">
          {validationResult.errors.map((err, idx) => (
            <div key={`err-${idx}`} className="tb-alert error">
              <span>🛑</span>
              <span>{err}</span>
            </div>
          ))}
          {validationResult.warnings.map((warn, idx) => (
            <div key={`warn-${idx}`} className="tb-alert warning">
              <span>⚠️</span>
              <span>{warn}</span>
            </div>
          ))}
        </section>
      )}

      {/* Workspace Grid */}
      <main className="tb-main-grid">
        {/* Drag and Drop Workspace */}
        <section className="tb-workspace">
          <div className="tb-workspace-toolbar">
            <h2 style={{ margin: 0, fontSize: '16px' }}>Availability Zone Placement Workspace</h2>
            <span style={{ fontSize: '12px', color: 'var(--tb-text-secondary)' }}>
              Drag node cards between zone boxes to rebalance topology
            </span>
          </div>

          {/* Zones Grid */}
          <div className="tb-zones-container">
            {zones.map(zone => {
              const zoneNodes = nodes.filter(n => n.zone === zone.id);
              const validatorCount = zoneNodes.filter(n => n.nodeType === 'Validator').length;
              const isOver = dragOverZoneId === zone.id;

              return (
                <div
                  key={zone.id}
                  className={`tb-zone-column ${isOver ? 'drag-over' : ''}`}
                  onDragOver={e => handleDragOver(e, zone.id)}
                  onDragLeave={e => handleDragLeave(e, zone.id)}
                  onDrop={e => handleDrop(e, zone.id)}
                >
                  <div className="tb-zone-header">
                    <div className="tb-zone-title">
                      <span>☁️ {zone.name}</span>
                      <span className="tb-zone-badge">{zoneNodes.length} nodes</span>
                    </div>
                    <span style={{ fontSize: '11px', color: 'var(--tb-text-secondary)' }}>
                      {validatorCount} validator(s)
                    </span>
                  </div>

                  <div className="tb-nodes-list">
                    {zoneNodes.map(node => (
                      <div
                        key={node.id}
                        className="tb-node-card"
                        draggable
                        onDragStart={e => handleDragStart(e, node.id)}
                      >
                        <div className="tb-node-card-header">
                          <span className="tb-node-name">{node.name}</span>
                          <button
                            onClick={() => handleRemoveNode(node.id)}
                            style={{ background: 'none', border: 'none', color: '#ef4444', cursor: 'pointer' }}
                            title="Remove node"
                          >
                            ×
                          </button>
                        </div>
                        <div className="tb-node-meta">
                          <span className={`tb-node-type ${node.nodeType.toLowerCase()}`}>
                            {node.nodeType}
                          </span>
                          <span>Network: {node.network || 'mainnet'}</span>
                        </div>
                      </div>
                    ))}

                    {zoneNodes.length === 0 && (
                      <div style={{ textAlign: 'center', color: '#64748b', fontSize: '12px', marginTop: '30px' }}>
                        Drop worker node here
                      </div>
                    )}
                  </div>
                </div>
              );
            })}
          </div>

          {/* Unassigned Pool */}
          <div
            className="tb-pool-container"
            onDragOver={e => handleDragOver(e, 'unassigned')}
            onDragLeave={e => handleDragLeave(e, 'unassigned')}
            onDrop={e => handleDrop(e, '')}
          >
            <div className="tb-pool-header">Unassigned Nodes Pool (Drag to place in a zone)</div>
            <div className="tb-pool-list">
              {nodes
                .filter(n => !n.zone || n.zone === 'unassigned')
                .map(node => (
                  <div
                    key={node.id}
                    className="tb-node-card"
                    draggable
                    onDragStart={e => handleDragStart(e, node.id)}
                    style={{ minWidth: '180px' }}
                  >
                    <div className="tb-node-card-header">
                      <span className="tb-node-name">{node.name}</span>
                      <button
                        onClick={() => handleRemoveNode(node.id)}
                        style={{ background: 'none', border: 'none', color: '#ef4444', cursor: 'pointer' }}
                      >
                        ×
                      </button>
                    </div>
                    <div className="tb-node-meta">
                      <span className={`tb-node-type ${node.nodeType.toLowerCase()}`}>
                        {node.nodeType}
                      </span>
                    </div>
                  </div>
                ))}

              {nodes.filter(n => !n.zone || n.zone === 'unassigned').length === 0 && (
                <div style={{ fontSize: '12px', color: '#64748b' }}>All nodes are assigned to availability zones.</div>
              )}
            </div>
          </div>
        </section>

        {/* Sidebar: Controls & Manifest Output */}
        <aside className="tb-sidebar">
          {/* Add Node Controls */}
          <div className="tb-panel">
            <h3 className="tb-panel-title">Add Node to Topology</h3>
            <form onSubmit={handleAddNode}>
              <div className="tb-form-group">
                <label>Node Name</label>
                <input
                  type="text"
                  className="tb-input"
                  placeholder="e.g. validator-east-1a"
                  value={newNodeName}
                  onChange={e => setNewNodeName(e.target.value)}
                />
              </div>

              <div className="tb-form-group">
                <label>Node Type</label>
                <select
                  className="tb-select"
                  value={newNodeType}
                  onChange={e => setNewNodeType(e.target.value)}
                >
                  <option value="Validator">Validator (Stellar Core)</option>
                  <option value="Horizon">Horizon API</option>
                  <option value="SorobanRpc">Soroban RPC</option>
                </select>
              </div>

              <div className="tb-form-group">
                <label>Target Availability Zone</label>
                <select
                  className="tb-select"
                  value={newNodeZone}
                  onChange={e => setNewNodeZone(e.target.value)}
                >
                  <option value="unassigned">Unassigned Pool</option>
                  {zones.map(z => (
                    <option key={z.id} value={z.id}>
                      {z.name}
                    </option>
                  ))}
                </select>
              </div>

              <button type="submit" className="tb-btn tb-btn-primary" style={{ width: '100%' }}>
                + Add Node
              </button>
            </form>
          </div>

          {/* Topology Spread Constraints Configuration */}
          <div className="tb-panel">
            <h3 className="tb-panel-title">Topology Spread Rules</h3>
            <div className="tb-form-group">
              <label>maxSkew</label>
              <input
                type="number"
                min="1"
                max="5"
                className="tb-input"
                value={spreadSettings.maxSkew}
                onChange={e =>
                  setSpreadSettings(prev => ({ ...prev, maxSkew: parseInt(e.target.value) || 1 }))
                }
              />
            </div>

            <div className="tb-form-group">
              <label>topologyKey</label>
              <input
                type="text"
                className="tb-input"
                value={spreadSettings.topologyKey}
                onChange={e => setSpreadSettings(prev => ({ ...prev, topologyKey: e.target.value }))}
              />
            </div>

            <div className="tb-form-group">
              <label>whenUnsatisfiable</label>
              <select
                className="tb-select"
                value={spreadSettings.whenUnsatisfiable}
                onChange={e =>
                  setSpreadSettings(prev => ({ ...prev, whenUnsatisfiable: e.target.value }))
                }
              >
                <option value="DoNotSchedule">DoNotSchedule (Hard enforcement)</option>
                <option value="ScheduleAnyway">ScheduleAnyway (Soft effort)</option>
              </select>
            </div>
          </div>

          {/* Generated Manifest Panel */}
          <div className="tb-panel">
            <div className="tb-panel-title">
              <span>Kubernetes Manifest Snippet</span>
              <button className="tb-btn" onClick={handleCopyYaml}>
                {copiedStatus ? '✓ Copied' : '📋 Copy YAML'}
              </button>
            </div>

            <pre className="tb-manifest-preview">{manifestYaml}</pre>
          </div>
        </aside>
      </main>
    </div>
  );
}

export default TopologyBuilder;
