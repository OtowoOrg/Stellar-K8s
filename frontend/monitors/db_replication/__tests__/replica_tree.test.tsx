import React from 'react';
import { render, screen } from '@testing-library/react';
import { ReplicaTree } from '../../../components/replica_tree';
import { ReplicationSnapshot } from '../types';

function snapshot(overrides: Partial<ReplicationSnapshot> = {}): ReplicationSnapshot {
  return {
    primary: { host: 'primary.db.internal', replicaCount: 2, lastUpdated: 1700000000000 },
    replicas: [
      {
        name: 'replica-1',
        host: '10.0.0.1',
        state: 'streaming',
        byteLag: 1024,
        replayLatencySeconds: 1.25,
        lastUpdated: 1700000000000,
      },
      {
        name: 'replica-2',
        host: '10.0.0.2',
        state: 'disconnected',
        byteLag: 5242880,
        replayLatencySeconds: 7.5,
        lastUpdated: 1700000000000,
      },
    ],
    ...overrides,
  };
}

describe('ReplicaTree', () => {
  it('renders the primary host and replica count', () => {
    render(<ReplicaTree snapshot={snapshot()} />);
    expect(screen.getByTestId('primary-host')).toHaveTextContent('primary.db.internal');
    expect(screen.getByTestId('replica-count')).toHaveTextContent('2 replicas');
  });

  it('flags replicas whose replay delay exceeds the threshold', () => {
    render(<ReplicaTree snapshot={snapshot()} />);
    const lagging = screen.getByTestId(`replica-replica-2`);
    expect(lagging).toHaveAttribute('data-lagging', 'true');
    expect(lagging.className).toContain('replica-node--lagging');
    expect(lagging.className).toContain('replica-node--degraded');

    const healthy = screen.getByTestId(`replica-replica-1`);
    expect(healthy).toHaveAttribute('data-lagging', 'false');
    expect(healthy.className).not.toContain('replica-node--lagging');
  });

  it('renders byte lag and replay delay for each replica', () => {
    render(<ReplicaTree snapshot={snapshot()} />);
    expect(screen.getByTestId('replica-replica-1-byte-lag')).toHaveTextContent('1 KB');
    expect(screen.getByTestId('replica-replica-1-replay-latency')).toHaveTextContent('1.25s');
    expect(screen.getByTestId('replica-replica-2-byte-lag')).toHaveTextContent('5 MB');
    expect(screen.getByTestId('replica-replica-2-replay-latency')).toHaveTextContent('7.50s');
  });

  it('shows a healthy summary when no replica is lagging', () => {
    const healthy = snapshot({
      replicas: [
        {
          name: 'replica-1',
          host: '10.0.0.1',
          state: 'streaming',
          byteLag: 0,
          replayLatencySeconds: 0.5,
          lastUpdated: 1700000000000,
        },
      ],
    });
    render(<ReplicaTree snapshot={healthy} />);
    expect(screen.getByTestId('healthy')).toBeITheTruthy();
    expect(screen.queryByTestId('lag-alert')).not.toBeInTheDocument();
  });

  it('renders an error message when the fetch failed', () => {
    render(<ReplicaTree snapshot={null} error="Prometheus query failed" />);
    expect(screen.getByRole('alert')).toHaveTextContentContaining('Prometheus query failed');
  });

  it('shows a loading state when no snapshot is available yet', () => {
    render(<ReplicaTree snapshot={null} />);
    expect(screen.getByRole('status')).toHaveTextContentContaining('Loading');
  });
});
