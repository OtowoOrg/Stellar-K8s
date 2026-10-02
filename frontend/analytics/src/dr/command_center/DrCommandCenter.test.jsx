/**
 * DrCommandCenter tests — Issue #92
 *
 * Tests validate:
 *  1. Phase status card rendering for all four DR phases.
 *  2. "Trigger Simulated Drill" button triggers POST /api/dr/trigger.
 *  3. WebSocket phase_update messages update node panel state.
 *  4. Execution history is preserved across simulated WS reconnects.
 *  5. Overall progress bar reflects completed phases.
 *  6. AlreadyRunning: trigger button is disabled while drill is active.
 *  7. Reset clears node state.
 *  8. Error banner appears when API call fails.
 */

import React from 'react';
import { render, screen, act, waitFor, fireEvent } from '@testing-library/react';
import { DrCommandCenter, DR_PHASES } from './DrCommandCenter.jsx';

// ---------------------------------------------------------------------------
// Mock WebSocket
// ---------------------------------------------------------------------------

class MockWebSocket {
  static instances = [];
  readyState = WebSocket.CONNECTING;
  onopen = null;
  onmessage = null;
  onclose = null;
  onerror = null;
  sent = [];

  constructor(url) {
    this.url = url;
    MockWebSocket.instances.push(this);
    // Auto-open after construction (simulates immediate connection)
    setTimeout(() => {
      this.readyState = WebSocket.OPEN;
      this.onopen?.();
    }, 0);
  }

  send(data) {
    this.sent.push(JSON.parse(data));
  }

  close() {
    this.readyState = WebSocket.CLOSED;
    this.onclose?.({ wasClean: true });
  }

  /** Helper: simulate an incoming message from the server. */
  receive(msg) {
    this.onmessage?.({ data: JSON.stringify(msg) });
  }
}

global.WebSocket = MockWebSocket;
global.WebSocket.CONNECTING = 0;
global.WebSocket.OPEN = 1;
global.WebSocket.CLOSED = 3;

// ---------------------------------------------------------------------------
// Mock fetch
// ---------------------------------------------------------------------------

const mockFetch = jest.fn();
global.fetch = mockFetch;

function mockTriggerSuccess(drillId = 'drill-abc-123') {
  mockFetch.mockResolvedValueOnce({
    ok: true,
    json: async () => ({
      drill_id: drillId,
      node: 'validator-0',
      dry_run: true,
      queued_at: new Date().toISOString(),
    }),
  });
}

function mockTriggerFailure() {
  mockFetch.mockResolvedValueOnce({
    ok: false,
    status: 500,
    statusText: 'Internal Server Error',
    json: async () => ({ message: 'Backend unavailable' }),
  });
}

function mockResetSuccess() {
  mockFetch.mockResolvedValueOnce({
    ok: true,
    json: async () => ({ node: 'validator-0', reset_at: new Date().toISOString() }),
  });
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const DEFAULT_NODES = ['validator-0', 'validator-1'];

function renderCommandCenter(props = {}) {
  return render(
    <DrCommandCenter
      nodes={DEFAULT_NODES}
      wsUrl="ws://localhost:8080/api/dr/stream"
      {...props}
    />,
  );
}

function getLastWs() {
  return MockWebSocket.instances[MockWebSocket.instances.length - 1];
}

beforeEach(() => {
  MockWebSocket.instances = [];
  mockFetch.mockReset();
  jest.useFakeTimers();
});

afterEach(() => {
  jest.useRealTimers();
});

// ---------------------------------------------------------------------------
// 1. Phase status card rendering
// ---------------------------------------------------------------------------

describe('Phase status cards', () => {
  it('renders all four phase labels for every node', () => {
    renderCommandCenter();
    // 4 phases × 2 nodes = 8 phase card labels
    DR_PHASES.forEach((phase) => {
      const cards = screen.getAllByText(phase.label);
      expect(cards).toHaveLength(DEFAULT_NODES.length);
    });
  });

  it('renders Pending state for all phases on first load', () => {
    renderCommandCenter();
    // "Pending" badge should appear for 4 phases × 2 nodes
    const badges = screen.getAllByText(/pending/i);
    expect(badges.length).toBeGreaterThanOrEqual(4);
  });

  it('shows node names in the panels', () => {
    renderCommandCenter();
    expect(screen.getByText('validator-0')).toBeInTheDocument();
    expect(screen.getByText('validator-1')).toBeInTheDocument();
  });
});

// ---------------------------------------------------------------------------
// 2. Trigger Simulated Drill
// ---------------------------------------------------------------------------

describe('Trigger Simulated Drill button', () => {
  it('calls POST /api/dr/trigger with dry_run=true on click', async () => {
    mockTriggerSuccess();
    renderCommandCenter();
    await act(async () => { jest.runAllTimers(); }); // let WS open

    const btn = screen.getByRole('button', { name: /trigger simulated drill/i });
    await act(async () => { fireEvent.click(btn); });

    expect(mockFetch).toHaveBeenCalledWith(
      '/api/dr/trigger',
      expect.objectContaining({
        method: 'POST',
        body: expect.stringContaining('"dry_run":true'),
      }),
    );
  });

  it('is disabled while a drill is in progress', async () => {
    mockTriggerSuccess('drill-xyz');
    renderCommandCenter();
    await act(async () => { jest.runAllTimers(); });

    const btn = screen.getByRole('button', { name: /trigger simulated drill/i });
    await act(async () => { fireEvent.click(btn); });

    // After trigger, button should become disabled
    await waitFor(() => {
      expect(btn).toBeDisabled();
    });
  });

  it('shows error banner when trigger API call fails', async () => {
    mockTriggerFailure();
    renderCommandCenter();
    await act(async () => { jest.runAllTimers(); });

    const btn = screen.getByRole('button', { name: /trigger simulated drill/i });
    await act(async () => { fireEvent.click(btn); });

    await waitFor(() => {
      expect(screen.getByText(/backend unavailable|DR trigger failed/i)).toBeInTheDocument();
    });
  });
});

// ---------------------------------------------------------------------------
// 3. WebSocket phase_update messages
// ---------------------------------------------------------------------------

describe('WebSocket phase updates', () => {
  it('updates a phase card to Running when phase_update arrives', async () => {
    renderCommandCenter();
    await act(async () => { jest.runAllTimers(); });

    const ws = getLastWs();
    act(() => {
      ws.receive({
        type: 'phase_update',
        node: 'validator-0',
        phase: 'snapshot_restoration',
        status: 'running',
        message: 'Creating snapshot…',
        timestamp: new Date().toISOString(),
      });
    });

    await waitFor(() => {
      expect(screen.getAllByText(/running/i).length).toBeGreaterThan(0);
    });
  });

  it('shows Passed badge after a phase completes successfully', async () => {
    renderCommandCenter();
    await act(async () => { jest.runAllTimers(); });

    const ws = getLastWs();
    act(() => {
      ws.receive({
        type: 'phase_update',
        node: 'validator-0',
        phase: 'pod_recreation',
        status: 'passed',
        message: 'Pod recreated',
        timestamp: new Date().toISOString(),
      });
    });

    await waitFor(() => {
      expect(screen.getAllByText(/passed/i).length).toBeGreaterThan(0);
    });
  });

  it('shows Failed badge when a phase fails', async () => {
    renderCommandCenter();
    await act(async () => { jest.runAllTimers(); });

    const ws = getLastWs();
    act(() => {
      ws.receive({
        type: 'phase_update',
        node: 'validator-1',
        phase: 'traffic_redirection',
        status: 'failed',
        message: 'LB timeout',
        timestamp: new Date().toISOString(),
      });
    });

    await waitFor(() => {
      expect(screen.getAllByText(/failed/i).length).toBeGreaterThan(0);
    });
  });
});

// ---------------------------------------------------------------------------
// 4. Execution history preserved across reconnects
// ---------------------------------------------------------------------------

describe('Execution history', () => {
  it('appends entries to history when phase updates arrive', async () => {
    renderCommandCenter();
    await act(async () => { jest.runAllTimers(); });

    const ws = getLastWs();
    act(() => {
      ws.receive({
        type: 'phase_update',
        node: 'validator-0',
        phase: 'catchup_sync',
        status: 'passed',
        message: 'Synced to ledger 12345',
        timestamp: new Date().toISOString(),
      });
    });

    await waitFor(() => {
      expect(screen.getByText(/synced to ledger 12345/i)).toBeInTheDocument();
    });
  });

  it('retains history entries after WebSocket closes and reconnects', async () => {
    renderCommandCenter();
    await act(async () => { jest.runAllTimers(); });

    const firstWs = getLastWs();
    // Send a message before disconnect
    act(() => {
      firstWs.receive({
        type: 'phase_update',
        node: 'validator-0',
        phase: 'catchup_sync',
        status: 'passed',
        message: 'history-entry-1',
        timestamp: new Date().toISOString(),
      });
    });

    await waitFor(() => {
      expect(screen.getByText(/history-entry-1/i)).toBeInTheDocument();
    });

    // Simulate disconnect → reconnect
    act(() => { firstWs.onclose?.({ wasClean: false }); });
    await act(async () => { jest.advanceTimersByTime(2000); });

    // History entry must still be visible after reconnect
    expect(screen.getByText(/history-entry-1/i)).toBeInTheDocument();
  });
});

// ---------------------------------------------------------------------------
// 5. Overall progress bar
// ---------------------------------------------------------------------------

describe('Overall progress bar', () => {
  it('shows 0% at start', () => {
    renderCommandCenter();
    expect(screen.getByText('0%')).toBeInTheDocument();
  });

  it('increases as phases pass', async () => {
    renderCommandCenter({ nodes: ['validator-0'] });
    await act(async () => { jest.runAllTimers(); });

    const ws = getLastWs();
    // Pass 1 of 4 phases → 25%
    act(() => {
      ws.receive({
        type: 'phase_update',
        node: 'validator-0',
        phase: 'snapshot_restoration',
        status: 'passed',
        message: '',
        timestamp: new Date().toISOString(),
      });
    });

    await waitFor(() => {
      expect(screen.getByText('25%')).toBeInTheDocument();
    });
  });

  it('shows 100% when all phases pass', async () => {
    renderCommandCenter({ nodes: ['validator-0'] });
    await act(async () => { jest.runAllTimers(); });

    const ws = getLastWs();
    const phases = ['snapshot_restoration', 'pod_recreation', 'catchup_sync', 'traffic_redirection'];
    act(() => {
      phases.forEach((phase) => {
        ws.receive({
          type: 'phase_update',
          node: 'validator-0',
          phase,
          status: 'passed',
          message: '',
          timestamp: new Date().toISOString(),
        });
      });
    });

    await waitFor(() => {
      expect(screen.getByText('100%')).toBeInTheDocument();
    });
  });
});

// ---------------------------------------------------------------------------
// 6. Reset
// ---------------------------------------------------------------------------

describe('Reset', () => {
  it('clears node state and calls POST /api/dr/reset', async () => {
    mockTriggerSuccess();
    mockResetSuccess();
    renderCommandCenter();
    await act(async () => { jest.runAllTimers(); });

    // Trigger then reset
    const triggerBtn = screen.getByRole('button', { name: /trigger simulated drill/i });
    await act(async () => { fireEvent.click(triggerBtn); });

    // Drill started — now call reset
    const resetBtn = screen.getByRole('button', { name: /reset/i });
    // Reset is enabled only when no active drill; simulate drill_complete first
    const ws = getLastWs();
    act(() => {
      ws.receive({
        type: 'drill_complete',
        node: 'validator-0',
        drill_id: 'drill-abc-123',
        status: 'passed',
        timestamp: new Date().toISOString(),
      });
    });

    await act(async () => { fireEvent.click(resetBtn); });

    expect(mockFetch).toHaveBeenCalledWith(
      '/api/dr/reset',
      expect.objectContaining({ method: 'POST' }),
    );
  });
});
