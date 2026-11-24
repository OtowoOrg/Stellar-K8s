import React, { useEffect, useState, useRef } from 'react';

const DR_PHASES = [
  { key: 'snapshot', name: 'Snapshot Created', status: 'pending', color: 'info' },
  { key: 'terminate', name: 'Node Terminated', status: 'pending', color: 'warning' },
  { key: 'restore', name: 'Volume Restored', status: 'pending', color: 'success' },
  { key: 'sync', name: 'Sync Re-established', status: 'pending', color: 'primary' },
];

export const DrCommandCenter = ({
  clusters,
  onInitiateDrill,
  styles = {},
}) => {
  const [drState, setDrState] = useState({
    isRunning: false,
    currentPhase: null,
    phaseProgress: 0,
    clusterStates: {},
  });

  const wsRef = useRef(null);

  useEffect(() => {
    wsRef.current = new WebSocket('ws://localhost:8080/dr-status');

    wsRef.current.onmessage = (event) => {
      const message = JSON.parse(event.data);
      setDrState((prev) => ({
        ...prev,
        clusterStates: {
          ...prev.clusterStates,
          [message.cluster]: {
            ...prev.clusterStates[message.cluster],
            [message.phase]: message.status,
          },
        },
      }));
    };

    wsRef.current.onclose = () => {
      setDrState((prev) => ({ ...prev, isRunning: false }));
    };

    return () => {
      wsRef.current.close();
    };
  }, []);

  useEffect(() => {
    if (drState.isRunning && onInitiateDrill) {
      onInitiateDrill();
    }
  }, [drState.isRunning, onInitiateDrill]);

  const handleInitiateDrill = () => {
    setDrState((prev) => ({ ...prev, isRunning: true, currentPhase: DR_PHASES[0].key }));
    if (wsRef.current) {
      wsRef.current.send(JSON.stringify({ action: 'start_dr_drill' }));
    }
  };

  const getClusterStatusColor = (status) => {
    const colors = {
      pending: 'var(--bs-gray-600)',
      running: 'var(--bs-primary)',
      completed: 'var(--bs-success)',
      failed: 'var(--bs-danger)',
    };
    return colors[status] || colors.pending;
  };

  const calculateOverallProgress = () => {
    const allClusters = clusters || [];
    if (allClusters.length === 0) return 0;

    let totalSteps = 0;
    let completedSteps = 0;

    DR_PHASES.forEach(() => {
      allClusters.forEach(() => {
        totalSteps++;
      });
    });

    // Count completed steps from cluster states
    Object.values(drState.clusterStates).forEach((clusterState) => {
      DR_PHASES.forEach((phase) => {
        if (clusterState[phase.key] === 'completed') {
          completedSteps++;
        }
      });
    });

    return totalSteps > 0 ? Math.round((completedSteps / totalSteps) * 100) : 0;
  };

  return (
    <div
      style={{
        padding: styles.padding || '20px',
        fontFamily: styles.fontFamily || 'sans-serif',
        backgroundColor: styles.backgroundColor || '#f8f9fa',
        borderRadius: styles.borderRadius || '8px',
        minHeight: styles.minHeight || '400px',
      }}
    >
      <h2 style={{ marginBottom: '20px', color: '#333' }}>
        DR Command Center{' '}
        {drState.isRunning && (
          <span style={{ color: 'var(--bs-danger)', fontWeight: 'bold' }}>
            DR Drill In Progress
          </span>
        )}
      </h2>

      <div style={{ display: 'flex', gap: '10px', marginBottom: '20px' }}>
        {clusters.map((cluster) => (
          <div
            key={cluster}
            style={{
              flex: 1,
              backgroundColor: '#fff',
              borderRadius: '6px',
              padding: '16px',
              boxShadow: '0 2px 4px rgba(0,0,0,0.1)',
            }}
          >
            <h4 style={{ margin: '0 0 12px 0', color: '#555' }}>{cluster}</h4>
            <div style={{ height: '20px' }}>
              {DR_PHASES.map((phase) => {
                const phaseStatus = drState.clusterStates[cluster]?.[phase.key] || 'pending';
                const phaseColor = getClusterStatusColor(phaseStatus);
                return (
                  <div
                    key={phase.key}
                    style={{
                      height: '100%',
                      width: `${(DR_PHASES.indexOf(phase) / (DR_PHASES.length - 1)) * 100}%`,
                      backgroundColor: phaseColor,
                      transition: 'width 0.3s ease',
                    }}
                  />
                );
              })}
            </div>
            <div style={{ fontSize: '12px', marginTop: '8px' }}>
              {DR_PHASES.map((phase) => {
                const phaseStatus = drState.clusterStates[cluster]?.[phase.key] || 'pending';
                const phaseColor = getClusterStatusColor(phaseStatus);
                return (
                  <span
                    key={phase.key}
                    style={{
                      display: 'inline-block',
                      width: `${(DR_PHASES.indexOf(phase) / (DR_PHASES.length - 1)) * 100}%`,
                      color: phaseColor,
                      fontWeight: phaseStatus === 'completed' ? 'bold' : 'normal',
                    }}
                  >
                    {phase.name}: {phaseStatus}
                  </span>
                );
              })}
            </div>
          </div>
        ))}
      </div>

      <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '10px' }}>
        <button
          onClick={handleInitiateDrill}
          style={{
            padding: '8px 16px',
            backgroundColor: drState.isRunning ? 'var(--bs-gray)' : 'var(--bs-primary)',
            color: '#fff',
            border: 'none',
            borderRadius: '4px',
            cursor: drState.isRunning ? 'not-allowed' : 'pointer',
            fontSize: '14px',
          }}
          disabled={drState.isRunning}
        >
          {drState.isRunning ? 'Running...' : 'Initiate Simulated DR Drill'}
        </button>
        <button
          onClick={() => wsRef.current.send(JSON.stringify({ action: 'reset_dr_state' }))}
          style={{
            padding: '8px 16px',
            backgroundColor: 'var(--bs-secondary)',
            color: '#fff',
            border: 'none',
            borderRadius: '4px',
            cursor: 'pointer',
            fontSize: '14px',
          }}
          disabled={!drState.isRunning}
        >
          Reset
        </button>
      </div>

      <div style={{ marginTop: '20px', paddingTop: '20px', borderTop: '1px solid #dee2e6' }}>
        <div style={{ fontSize: '14px', marginBottom: '10px' }}>
          Overall Progress: {calculateOverallProgress()}%
        </div>
        <div>
          <progress
            value={calculateOverallProgress()}
            max={100}
            style={{ width: '100%', marginTop: '8px' }}
          />
        </div>
      </div>
    </div>
  );
};

export default DrCommandCenter;