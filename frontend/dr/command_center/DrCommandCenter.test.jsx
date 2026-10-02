/**
 * DrCommandCenter unit tests — Issue #92
 *
 * Covers:
 *  1. Renders all four phase status cards per node (initial pending state)
 *  2. Trigger button calls POST /api/dr/trigger with dry_run=true
 *  3. Button is disabled while a drill is active
 *  4. WS phase_update → card transitions Running / Passed / Failed
 *  5. Execution history is preserved across WS reconnects
 *  6. Overall progress bar advances as phases pass
 *  7. Error banner shown on API failure, dismissable
 *  8. Reset clears node state
 *  9. Other players (other nodes) still see pending while one node drills
 */

import React from 'react';
import { render, screen, act, waitFor, fireEvent } from '@testing-library/react';
import { DrCommandCenter, DR_PHASES } from './DrCommandCenter.jsx';

// ─── Mock WebSocket ───────────────────────────────────────────────────────────

class FakeWS {
  static instances = [];
  readyState = WebSocket.CONNECTING;
  sent = [];
  onopen = null; onmessage = null; onclose = null; onerror = null;

  constructor(url) {
    this.url = url;
    FakeWS.instances.push(this);
    setTimeout(() => { this.readyState = WebSocket.OPEN; this.onopen?.(); }, 0);
  }
  send(d) { this.sent.push(JSON.parse(d)); }
  close() { this.readyState = WebSocket.CLOSED; this.onclose?.({ wasClean: true }); }
  recv(msg) { this.onmessage?.({ data: JSON.stringify(msg) }); }
}

global.WebSocket = FakeWS;
global.WebSocket.CONNECTING = 0;
global.WebSocket.OPEN = 1;
global.WebSocket.CLOSED = 3;

// ─── Mock fetch ───────────────────────────────────────────────────────────────

global.fetch = jest.fn();

function mockTriggerOk(drillId = 'drill-001') {
  global.fetch.mockResolvedValueOnce({
    ok: true,
    json: async () => ({ drill_id: drillId, node: 'validator-0', dry_run: true, queued_at: new Date().toISOString() }),
  });
}

function mockTriggerFail() {
  global.fetch.mockResolvedValueOnce({
    ok: false, status: 500, statusText: 'Server Error',
    json: async () => ({ message: 'backend down' }),
  });
}

function mockResetOk() {
  global.fetch.mockResolvedValueOnce({
    ok: true, json: async () => ({ node: 'validator-0', reset_at: new Date().toISOString() }),
  });
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

const NODES = ['validator-0', 'validator-1'];

function renderCC(props = {}) {
  return render(
    <DrCommandCenter nodes={NODES} wsUrl="ws://localhost:8080/api/dr/stream" {...props} />,
  );
}

const lastWs = () => FakeWS.instances[FakeWS.instances.length - 1];

beforeEach(() => {
  FakeWS.instances = [];
  global.fetch.mockReset();
  jest.useFakeTimers();
});
afterEach(() => jest.useRealTimers());

// ─── Tests ────────────────────────────────────────────────────────────────────

describe('1 — Phase card rendering', () => {
  it('renders all four phase labels for every node', () => {
    renderCC();
    DR_PHASES.forEach((p) => {
      expect(screen.getAllByText(p.label)).toHaveLength(NODES.length);
    });
  });

  it('starts all phases as Pending', () => {
    renderCC();
    expect(screen.getAllByText(/pending/i).length).toBeGreaterThanOrEqual(4);
  });

  it('shows all node names', () => {
    renderCC();
    NODES.forEach((n) => expect(screen.getByText(n)).toBeInTheDocument());
  });
});

describe('2 — Trigger Simulated Drill', () => {
  it('calls POST /api/dr/trigger with dry_run: true', async () => {
    mockTriggerOk();
    renderCC();
    await act(async () => { jest.runAllTimers(); });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /trigger simulated drill/i }));
    });

    expect(global.fetch).toHaveBeenCalledWith('/api/dr/trigger', expect.objectContaining({
      method: 'POST',
      body: expect.stringContaining('"dry_run":true'),
    }));
  });
});

describe('3 — Button disabled during drill', () => {
  it('disables the Trigger button once a drill is running', async () => {
    mockTriggerOk();
    renderCC();
    await act(async () => { jest.runAllTimers(); });

    const btn = screen.getByRole('button', { name: /trigger simulated drill/i });
    await act(async () => { fireEvent.click(btn); });

    await waitFor(() => expect(btn).toBeDisabled());
  });
});

describe('4 — WebSocket phase updates', () => {
  it('transitions a card to Running on phase_update', async () => {
    renderCC();
    await act(async () => { jest.runAllTimers(); });

    act(() => {
      lastWs().recv({ type: 'phase_update', node: 'validator-0',
        phase: 'snapshot_restoration', status: 'running', message: 'Restoring…', timestamp: '' });
    });

    await waitFor(() => expect(screen.getAllByText(/running/i).length).toBeGreaterThan(0));
  });

  it('shows Passed badge after phase completes', async () => {
    renderCC();
    await act(async () => { jest.runAllTimers(); });

    act(() => {
      lastWs().recv({ type: 'phase_update', node: 'validator-0',
        phase: 'pod_recreation', status: 'passed', message: 'Done', timestamp: '' });
    });

    await waitFor(() => expect(screen.getAllByText(/passed/i).length).toBeGreaterThan(0));
  });

  it('shows Failed badge on failure', async () => {
    renderCC();
    await act(async () => { jest.runAllTimers(); });

    act(() => {
      lastWs().recv({ type: 'phase_update', node: 'validator-1',
        phase: 'traffic_redirection', status: 'failed', message: 'LB timeout', timestamp: '' });
    });

    await waitFor(() => expect(screen.getAllByText(/failed/i).length).toBeGreaterThan(0));
  });
});

describe('5 — Execution history preserved across reconnects', () => {
  it('appends history entries on phase_update', async () => {
    renderCC();
    await act(async () => { jest.runAllTimers(); });

    act(() => {
      lastWs().recv({ type: 'phase_update', node: 'validator-0',
        phase: 'catchup_sync', status: 'passed', message: 'ledger-12345', timestamp: '' });
    });

    await waitFor(() => expect(screen.getByText(/ledger-12345/)).toBeInTheDocument());
  });

  it('retains history after WS disconnect + reconnect', async () => {
    renderCC();
    await act(async () => { jest.runAllTimers(); });

    const ws1 = lastWs();
    act(() => {
      ws1.recv({ type: 'phase_update', node: 'validator-0',
        phase: 'catchup_sync', status: 'passed', message: 'history-preserved', timestamp: '' });
    });
    await waitFor(() => expect(screen.getByText(/history-preserved/)).toBeInTheDocument());

    // Simulate disconnect → reconnect
    act(() => { ws1.onclose?.({ wasClean: false }); });
    await act(async () => { jest.advanceTimersByTime(2000); });

    // History still present
    expect(screen.getByText(/history-preserved/)).toBeInTheDocument();
  });
});

describe('6 — Overall progress bar', () => {
  it('shows 0% initially', () => {
    renderCC();
    expect(screen.getByText('0%')).toBeInTheDocument();
  });

  it('advances to 25% after one of four phases passes on a single-node setup', async () => {
    renderCC({ nodes: ['validator-0'] });
    await act(async () => { jest.runAllTimers(); });

    act(() => {
      lastWs().recv({ type: 'phase_update', node: 'validator-0',
        phase: 'snapshot_restoration', status: 'passed', message: '', timestamp: '' });
    });

    await waitFor(() => expect(screen.getByText('25%')).toBeInTheDocument());
  });

  it('reaches 100% when all phases pass on a single node', async () => {
    renderCC({ nodes: ['validator-0'] });
    await act(async () => { jest.runAllTimers(); });

    const phases = ['snapshot_restoration', 'pod_recreation', 'catchup_sync', 'traffic_redirection'];
    act(() => {
      phases.forEach((phase) => {
        lastWs().recv({ type: 'phase_update', node: 'validator-0', phase, status: 'passed', message: '', timestamp: '' });
      });
    });

    await waitFor(() => expect(screen.getByText('100%')).toBeInTheDocument());
  });
});

describe('7 — Error banner', () => {
  it('shows error message when trigger API fails', async () => {
    mockTriggerFail();
    renderCC();
    await act(async () => { jest.runAllTimers(); });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /trigger simulated drill/i }));
    });

    await waitFor(() =>
      expect(screen.getByText(/backend down|DR trigger failed/i)).toBeInTheDocument()
    );
  });

  it('dismisses error banner on ✕ click', async () => {
    mockTriggerFail();
    renderCC();
    await act(async () => { jest.runAllTimers(); });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /trigger simulated drill/i }));
    });

    await waitFor(() => expect(screen.getByText(/backend down|DR trigger failed/i)).toBeInTheDocument());

    fireEvent.click(screen.getByText('✕'));
    await waitFor(() =>
      expect(screen.queryByText(/backend down|DR trigger failed/i)).not.toBeInTheDocument()
    );
  });
});

describe('8 — Reset', () => {
  it('calls POST /api/dr/reset', async () => {
    mockResetOk();
    renderCC();
    await act(async () => { jest.runAllTimers(); });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /reset/i }));
    });

    await waitFor(() =>
      expect(global.fetch).toHaveBeenCalledWith('/api/dr/reset', expect.objectContaining({ method: 'POST' }))
    );
  });
});

describe('9 — Node isolation', () => {
  it('validator-1 phases remain Pending while validator-0 drills', async () => {
    renderCC();
    await act(async () => { jest.runAllTimers(); });

    act(() => {
      lastWs().recv({ type: 'phase_update', node: 'validator-0',
        phase: 'snapshot_restoration', status: 'running', message: '', timestamp: '' });
    });

    await waitFor(() => expect(screen.getAllByText(/running/i).length).toBeGreaterThan(0));

    // validator-1 cards should still show pending
    expect(screen.getAllByText(/pending/i).length).toBeGreaterThan(0);
  });
});
