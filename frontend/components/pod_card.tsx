/**
 * PodCard — Issue #90
 *
 * TypeScript wrapper component that renders a single StatefulSet replica
 * card inside the Ledger Catch-Up Timeline Visualizer.  Consumes the same
 * normalized replica shape produced by `deriveRolloutView` (phases.js) and
 * is designed to compose with the existing ReplicaCard.jsx and
 * PhaseStepper.jsx without duplicating logic.
 *
 * Why a separate TypeScript component?
 * ──────────────────────────────────────
 * The rest of the timeline module is plain JSX.  PodCard.tsx provides:
 *  1. Strict prop types for the replica shape so TypeScript consumers
 *     (monitoring dashboards, embedding portals) get autocomplete and
 *     compile-time safety without touching the existing JSX files.
 *  2. A stable public API surface: callers import `PodCard` and `PodCardProps`
 *     instead of reaching into the internal ReplicaCard implementation.
 *  3. Additional "stuck-phase" visual affordances: a pulsing amber glow when
 *     the pod is stalled and a collapsible diagnostic panel showing the full
 *     diagnostic message from `diagnoseReplica`.
 *
 * Usage
 * ──────
 * ```tsx
 * import PodCard from 'frontend/components/pod_card';
 *
 * <PodCard replica={replica} onUnstick={unstick} />
 * ```
 */

import React, { useState, memo } from 'react';

// ─── Types ────────────────────────────────────────────────────────────────────

/** Ledger catch-up detail — from replica.phaseDetail when phase is history-catchup. */
export interface LedgerDetail {
  currentLedger: number;
  targetLedger:  number;
}

/** One of the four Stellar initialization phases. */
export type PhaseId =
  | 'database-schema-migration'
  | 'history-catchup'
  | 'quorum-peering'
  | 'fully-synced';

/** Normalized replica shape produced by deriveRolloutView. */
export interface ReplicaView {
  ordinal:         number;
  name:            string;
  image:           string | null;
  updated:         boolean;
  phase:           PhaseId;
  phaseIdx:        number;
  phaseProgress:   number;   // 0..1
  overallProgress: number;   // 0..1 across the full 4-phase pipeline
  stallSamples:    number;
  stalled:         boolean;
  containerStatus: string;
  containerReady:  boolean;
  restartCount:    number;
  phaseDetail:     LedgerDetail | null;
  blocked:         boolean;
  blockedBy:       { name: string } | null;
  bottleneck:      boolean;
  diagnostic:      string | null;
}

export interface PodCardProps {
  /** Normalized replica data from `deriveRolloutView`. */
  replica: ReplicaView;
  /**
   * Called when the operator clicks "Resume" on a stalled bottleneck pod.
   * Typically wires to `simulation.unstick()` or a real operator API call.
   */
  onUnstick?: (ordinal: number) => void;
  /** Optional additional CSS class applied to the root element. */
  className?: string;
}

// ─── Colour helpers ───────────────────────────────────────────────────────────

type Tone = 'green' | 'blue' | 'amber' | 'red' | 'grey';

const TONE_COLOR: Record<Tone, string> = {
  green: '#198754',
  blue:  '#0d6efd',
  amber: '#fd7e14',
  red:   '#dc3545',
  grey:  '#6c757d',
};

const TONE_BG: Record<Tone, string> = {
  green: '#d1e7dd',
  blue:  '#cfe2ff',
  amber: '#fff3cd',
  red:   '#f8d7da',
  grey:  '#f8f9fa',
};

function phaseTone(phase: PhaseId, stalled: boolean): Tone {
  if (stalled) return 'red';
  switch (phase) {
    case 'database-schema-migration': return 'blue';
    case 'history-catchup':           return 'amber';
    case 'quorum-peering':            return 'blue';
    case 'fully-synced':              return 'green';
    default:                          return 'grey';
  }
}

function overallTone(r: ReplicaView): Tone {
  if (r.bottleneck) return 'red';
  if (r.blocked)    return 'grey';
  if (r.containerReady && r.phase === 'fully-synced') return 'green';
  if (r.stalled)    return 'red';
  return 'blue';
}

// ─── Internal sub-components ─────────────────────────────────────────────────

interface ProgressBarProps {
  value:   number;   // 0..1
  label:   string;
  detail?: string | null;
  tone:    Tone;
}

function InlineProgressBar({ value, label, detail, tone }: ProgressBarProps) {
  const pct = Math.round(Math.max(0, Math.min(1, value)) * 100);
  return (
    <div style={{ marginBottom: 6 }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 11, color: '#555', marginBottom: 3 }}>
        <span>{label}</span>
        <span style={{ color: '#888' }}>{detail ?? `${pct}%`}</span>
      </div>
      <div
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={pct}
        aria-label={label}
        style={{ height: 6, background: '#e9ecef', borderRadius: 3, overflow: 'hidden' }}
      >
        <div style={{
          height: '100%',
          width: `${pct}%`,
          background: TONE_COLOR[tone],
          transition: 'width .4s ease',
          borderRadius: 3,
        }} />
      </div>
    </div>
  );
}

interface PhaseBadgeProps {
  phase: PhaseId;
  stalled: boolean;
}

function PhaseBadge({ phase, stalled }: PhaseBadgeProps) {
  const label: Record<PhaseId, string> = {
    'database-schema-migration': 'Schema Migration',
    'history-catchup':           'Ledger Catch-Up',
    'quorum-peering':            'Quorum Peering',
    'fully-synced':              'Fully Synced',
  };
  const tone = phaseTone(phase, stalled);
  return (
    <span style={{
      display: 'inline-block',
      padding: '2px 8px', borderRadius: 10,
      background: TONE_BG[tone],
      color: TONE_COLOR[tone],
      fontSize: 11, fontWeight: 700,
      border: `1px solid ${TONE_COLOR[tone]}55`,
    }}>
      {stalled ? '⚠ STALLED — ' : ''}{label[phase] ?? phase}
    </span>
  );
}

// ─── Main component ───────────────────────────────────────────────────────────

/**
 * PodCard renders one StatefulSet replica in the Ledger Catch-Up Timeline.
 *
 * Visual states
 * ─────────────
 *  ready      — green border, "✓ READY" badge
 *  rolling    — blue border, active phase badge + progress bars
 *  stalled    — red/amber border + pulsing glow, expandable diagnostic
 *  bottleneck — red border, "BLOCKED" banner + optional Resume button
 *  gated      — grey border, "WAITING" banner (held by a higher-ordinal pod)
 */
const PodCard = memo(function PodCard({ replica, onUnstick, className = '' }: PodCardProps) {
  const [diagOpen, setDiagOpen] = useState(replica.bottleneck || replica.stalled);
  const tone     = overallTone(replica);
  const phaseTn  = phaseTone(replica.phase, replica.stalled);
  const isReady  = replica.containerReady && replica.phase === 'fully-synced';

  const ledgerDetail: string | null =
    replica.phaseDetail
      ? `${replica.phaseDetail.currentLedger.toLocaleString()} / ${replica.phaseDetail.targetLedger.toLocaleString()}`
      : null;

  const overallPct = Math.round(replica.overallProgress * 100);

  return (
    <article
      className={`pod-card ${className}`}
      data-testid={`pod-card-${replica.ordinal}`}
      style={{
        background: '#fff',
        borderRadius: 10,
        padding: '14px 16px',
        border: `2px solid ${TONE_COLOR[tone]}`,
        boxShadow: replica.stalled
          ? `0 0 0 3px ${TONE_COLOR.amber}44, 0 2px 8px rgba(0,0,0,.06)`
          : '0 2px 8px rgba(0,0,0,.06)',
        transition: 'border-color .3s, box-shadow .3s',
      }}
    >
      {/* ── Header ── */}
      <header style={{ display: 'flex', alignItems: 'flex-start', gap: 10, marginBottom: 12 }}>
        <div style={{ flex: 1 }}>
          <code style={{ fontSize: 13, fontWeight: 700, color: '#1a1a2e', display: 'block' }}>
            {replica.name}
          </code>
          <span style={{ fontSize: 11, color: '#888' }}>
            {replica.updated ? 'updated image' : 'previous image'}
            {replica.restartCount > 0 && ` · ${replica.restartCount} restart${replica.restartCount > 1 ? 's' : ''}`}
          </span>
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'flex-end', gap: 4 }}>
          {/* Overall status chip */}
          <span style={{
            padding: '2px 8px', borderRadius: 10,
            background: isReady ? TONE_BG.green : TONE_BG[tone],
            color:  isReady ? TONE_COLOR.green : TONE_COLOR[tone],
            fontSize: 11, fontWeight: 700,
            border: `1px solid ${isReady ? TONE_COLOR.green : TONE_COLOR[tone]}55`,
          }}>
            {replica.bottleneck ? '⛔ BLOCKED'
              : replica.blocked   ? '⏸ WAITING'
              : isReady           ? '✓ READY'
              : '↻ ROLLING'}
          </span>

          {/* Raw K8s container status */}
          <span style={{ fontSize: 10, color: '#999', fontFamily: 'monospace' }}>
            {replica.containerStatus}
          </span>
        </div>
      </header>

      {/* ── Current phase badge ── */}
      <div style={{ marginBottom: 10 }}>
        <PhaseBadge phase={replica.phase} stalled={replica.stalled} />
      </div>

      {/* ── Progress bars ── */}
      <InlineProgressBar
        value={replica.overallProgress}
        label="Stellar init"
        detail={`Step ${replica.phaseIdx + 1}/4 — ${overallPct}%`}
        tone={replica.stalled ? 'red' : overallTone(replica) === 'green' ? 'green' : 'blue'}
      />

      {(replica.phase === 'history-catchup' || (replica.phase === 'fully-synced' && ledgerDetail)) && (
        <InlineProgressBar
          value={replica.phaseProgress}
          label="Ledger catch-up"
          detail={ledgerDetail}
          tone={replica.stalled ? 'red' : 'amber'}
        />
      )}

      {replica.phase !== 'history-catchup' && replica.phase !== 'fully-synced' && (
        <InlineProgressBar
          value={replica.phaseProgress}
          label={replica.phase.replace(/-/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase())}
          detail={null}
          tone={phaseTn}
        />
      )}

      {/* ── Diagnostic panel ── */}
      {replica.diagnostic && (
        <div style={{ marginTop: 10 }}>
          <button
            onClick={() => setDiagOpen((o) => !o)}
            style={{
              background: 'none', border: 'none', cursor: 'pointer',
              fontSize: 11, color: TONE_COLOR[replica.bottleneck ? 'red' : replica.blocked ? 'grey' : 'amber'],
              padding: 0, display: 'flex', alignItems: 'center', gap: 4,
            }}
            aria-expanded={diagOpen}
          >
            <span>{replica.bottleneck ? '⛔' : replica.blocked ? '⏸' : '⏳'}</span>
            <span>{diagOpen ? '▲ Hide diagnostic' : '▼ Show diagnostic'}</span>
          </button>

          {diagOpen && (
            <p
              role="status"
              style={{
                margin: '6px 0 0',
                padding: '8px 10px',
                borderRadius: 6,
                background: replica.bottleneck ? TONE_BG.red : replica.blocked ? TONE_BG.grey : TONE_BG.amber,
                color: '#333',
                fontSize: 12,
                lineHeight: 1.5,
              }}
            >
              {replica.diagnostic}
            </p>
          )}
        </div>
      )}

      {/* ── Resume button (bottleneck only) ── */}
      {replica.bottleneck && onUnstick && (
        <div style={{ marginTop: 10 }}>
          <button
            onClick={() => onUnstick(replica.ordinal)}
            style={{
              padding: '5px 12px',
              background: TONE_COLOR.amber, color: '#fff',
              border: 'none', borderRadius: 6,
              cursor: 'pointer', fontSize: 12, fontWeight: 600,
            }}
          >
            ▶ Resume stuck replica
          </button>
        </div>
      )}
    </article>
  );
});

export default PodCard;
