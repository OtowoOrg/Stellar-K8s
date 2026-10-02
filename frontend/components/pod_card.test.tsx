/**
 * PodCard unit tests — Issue #90
 *
 * Validates:
 *  1. Ready replica: green "✓ READY" badge, no diagnostic
 *  2. Bottleneck replica: red "⛔ BLOCKED" badge + diagnostic shown by default
 *  3. Gated replica: "⏸ WAITING" badge + diagnostic
 *  4. Stalled replica: pulsing amber, stalled phase badge, diagnostic
 *  5. History-catchup: ledger progress bar rendered with current/target
 *  6. Resume button visible only for bottleneck replicas
 *  7. onUnstick called with correct ordinal when Resume clicked
 *  8. Diagnostic panel toggle (click to expand/collapse)
 *  9. Previous image label when replica.updated is false
 * 10. Restart count shown when > 0
 */

import React from 'react';
import { render, screen, fireEvent } from '@testing-library/react';
import PodCard, { ReplicaView } from './pod_card';

// ─── Fixture factory ──────────────────────────────────────────────────────────

function makeReplica(overrides: Partial<ReplicaView> = {}): ReplicaView {
  return {
    ordinal:         0,
    name:            'my-validator-0',
    image:           'stellar/core:20.5.0',
    updated:         true,
    phase:           'fully-synced',
    phaseIdx:        3,
    phaseProgress:   1,
    overallProgress: 1,
    stallSamples:    0,
    stalled:         false,
    containerStatus: 'Ready',
    containerReady:  true,
    restartCount:    0,
    phaseDetail:     null,
    blocked:         false,
    blockedBy:       null,
    bottleneck:      false,
    diagnostic:      null,
    ...overrides,
  };
}

function makeStuckReplica(): ReplicaView {
  return makeReplica({
    ordinal:         1,
    name:            'my-validator-1',
    phase:           'history-catchup',
    phaseIdx:        1,
    phaseProgress:   0.53,
    overallProgress: 0.38,
    stalled:         true,
    stallSamples:    6,
    containerReady:  false,
    containerStatus: 'Running',
    bottleneck:      true,
    blocked:         false,
    phaseDetail:     { currentLedger: 5_123_000, targetLedger: 5_142_300 },
    diagnostic:      'History Catchup is stalled — stuck at ledger 5,123,000. Check history archive reachability.',
  });
}

function makeGatedReplica(): ReplicaView {
  return makeReplica({
    ordinal:        0,
    name:           'my-validator-0',
    phase:          'fully-synced',
    updated:        false,
    containerReady: false,
    blocked:        true,
    blockedBy:      { name: 'my-validator-1' },
    diagnostic:     'Waiting for my-validator-1 to become Ready.',
  });
}

// ─── Tests ────────────────────────────────────────────────────────────────────

describe('1 — Ready replica', () => {
  it('shows ✓ READY badge', () => {
    render(<PodCard replica={makeReplica()} />);
    expect(screen.getByText('✓ READY')).toBeInTheDocument();
  });

  it('renders no diagnostic button for a healthy pod', () => {
    render(<PodCard replica={makeReplica()} />);
    expect(screen.queryByText(/show diagnostic/i)).not.toBeInTheDocument();
  });

  it('renders pod name', () => {
    render(<PodCard replica={makeReplica()} />);
    expect(screen.getByText('my-validator-0')).toBeInTheDocument();
  });
});

describe('2 — Bottleneck replica', () => {
  it('shows ⛔ BLOCKED badge', () => {
    render(<PodCard replica={makeStuckReplica()} />);
    expect(screen.getByText('⛔ BLOCKED')).toBeInTheDocument();
  });

  it('shows diagnostic panel open by default', () => {
    render(<PodCard replica={makeStuckReplica()} />);
    expect(screen.getByText(/History Catchup is stalled/)).toBeInTheDocument();
  });
});

describe('3 — Gated replica', () => {
  it('shows ⏸ WAITING badge', () => {
    render(<PodCard replica={makeGatedReplica()} />);
    expect(screen.getByText('⏸ WAITING')).toBeInTheDocument();
  });

  it('renders gated diagnostic after expanding', () => {
    render(<PodCard replica={makeGatedReplica()} />);
    const btn = screen.getByText(/show diagnostic/i);
    fireEvent.click(btn);
    expect(screen.getByText(/Waiting for my-validator-1/)).toBeInTheDocument();
  });
});

describe('4 — Stalled replica badge', () => {
  it('shows STALLED in the phase badge label', () => {
    render(<PodCard replica={makeStuckReplica()} />);
    expect(screen.getByText(/stalled/i)).toBeInTheDocument();
  });
});

describe('5 — Ledger catch-up progress bar', () => {
  it('renders current and target ledger numbers', () => {
    render(<PodCard replica={makeStuckReplica()} />);
    expect(screen.getByText(/5,123,000.*5,142,300|5,123,000 \/ 5,142,300/)).toBeInTheDocument();
  });

  it('renders a progressbar with correct aria-valuenow', () => {
    render(<PodCard replica={makeStuckReplica()} />);
    const bars = screen.getAllByRole('progressbar');
    const catchupBar = bars.find((b) => b.getAttribute('aria-label') === 'Ledger catch-up');
    expect(catchupBar).toBeDefined();
    // 53% progress
    expect(Number(catchupBar!.getAttribute('aria-valuenow'))).toBe(53);
  });
});

describe('6 — Resume button visibility', () => {
  it('shows Resume button on bottleneck replica', () => {
    render(<PodCard replica={makeStuckReplica()} onUnstick={jest.fn()} />);
    expect(screen.getByRole('button', { name: /resume stuck replica/i })).toBeInTheDocument();
  });

  it('does NOT show Resume button on gated replica', () => {
    render(<PodCard replica={makeGatedReplica()} onUnstick={jest.fn()} />);
    expect(screen.queryByRole('button', { name: /resume/i })).not.toBeInTheDocument();
  });

  it('does NOT show Resume button when onUnstick is not provided', () => {
    render(<PodCard replica={makeStuckReplica()} />);
    expect(screen.queryByRole('button', { name: /resume/i })).not.toBeInTheDocument();
  });
});

describe('7 — onUnstick callback', () => {
  it('calls onUnstick with the correct ordinal', () => {
    const onUnstick = jest.fn();
    render(<PodCard replica={makeStuckReplica()} onUnstick={onUnstick} />);
    fireEvent.click(screen.getByRole('button', { name: /resume stuck replica/i }));
    expect(onUnstick).toHaveBeenCalledWith(1);
  });
});

describe('8 — Diagnostic panel toggle', () => {
  it('toggles diagnostic panel open/closed', () => {
    render(<PodCard replica={makeGatedReplica()} />);
    // Default: closed (gated but not bottleneck)
    expect(screen.queryByText(/Waiting for/)).not.toBeInTheDocument();

    fireEvent.click(screen.getByText(/show diagnostic/i));
    expect(screen.getByText(/Waiting for my-validator-1/)).toBeInTheDocument();

    fireEvent.click(screen.getByText(/hide diagnostic/i));
    expect(screen.queryByText(/Waiting for/)).not.toBeInTheDocument();
  });
});

describe('9 — Previous image label', () => {
  it('shows "previous image" when updated is false', () => {
    render(<PodCard replica={makeReplica({ updated: false })} />);
    expect(screen.getByText(/previous image/i)).toBeInTheDocument();
  });

  it('shows "updated image" when updated is true', () => {
    render(<PodCard replica={makeReplica({ updated: true })} />);
    expect(screen.getByText(/updated image/i)).toBeInTheDocument();
  });
});

describe('10 — Restart count', () => {
  it('shows restart count when > 0', () => {
    render(<PodCard replica={makeReplica({ restartCount: 3 })} />);
    expect(screen.getByText(/3 restart/i)).toBeInTheDocument();
  });

  it('does not show restart count when 0', () => {
    render(<PodCard replica={makeReplica({ restartCount: 0 })} />);
    expect(screen.queryByText(/restart/i)).not.toBeInTheDocument();
  });
});
