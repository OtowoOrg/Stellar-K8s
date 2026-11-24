/**
 * FilterControls.tsx
 *
 * Filter bar for the Soroban event stream inspector.
 * Supports filtering by:
 *   - Contract ID  (substring match, case-insensitive)
 *   - Event Topic  (matches against first decoded symbol topic)
 *   - Ledger range (from / to sequence numbers)
 *   - Event type   (contract | system | diagnostic | all)
 */

import React, { useCallback } from 'react';
import type { RawContractEvent } from '../../services/event_stream';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface FilterState {
  contractId: string;
  topic: string;
  ledgerFrom: string;
  ledgerTo: string;
  eventType: 'all' | 'contract' | 'system' | 'diagnostic';
  /** When true the stream is paused (no new events rendered). */
  paused: boolean;
}

export const DEFAULT_FILTER: FilterState = {
  contractId: '',
  topic: '',
  ledgerFrom: '',
  ledgerTo: '',
  eventType: 'all',
  paused: false,
};

export interface FilterControlsProps {
  filter: FilterState;
  onChange: (next: FilterState) => void;
  onClear: () => void;
  matchCount: number;
  totalCount: number;
}

// ---------------------------------------------------------------------------
// applyFilters — pure helper called in the parent render
// ---------------------------------------------------------------------------

export function applyFilters(
  events: RawContractEvent[],
  filter: FilterState,
): RawContractEvent[] {
  const cidLower = filter.contractId.trim().toLowerCase();
  const topicLower = filter.topic.trim().toLowerCase();
  const fromLedger = filter.ledgerFrom ? parseInt(filter.ledgerFrom, 10) : null;
  const toLedger = filter.ledgerTo ? parseInt(filter.ledgerTo, 10) : null;

  return events.filter((e) => {
    if (cidLower && !e.contract_id.toLowerCase().includes(cidLower)) return false;
    if (topicLower) {
      const topicMatch = e.topics.some((t) =>
        t.toLowerCase().includes(topicLower),
      );
      if (!topicMatch) return false;
    }
    if (fromLedger !== null && e.ledger < fromLedger) return false;
    if (toLedger !== null && e.ledger > toLedger) return false;
    if (filter.eventType !== 'all' && e.event_type !== filter.eventType)
      return false;
    return true;
  });
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function FilterControls({
  filter,
  onChange,
  onClear,
  matchCount,
  totalCount,
}: FilterControlsProps) {
  const set = useCallback(
    (patch: Partial<FilterState>) => onChange({ ...filter, ...patch }),
    [filter, onChange],
  );

  const hasActiveFilter =
    filter.contractId ||
    filter.topic ||
    filter.ledgerFrom ||
    filter.ledgerTo ||
    filter.eventType !== 'all';

  return (
    <div className="filter-bar" role="search" aria-label="Event filters">
      {/* Contract ID */}
      <label className="filter-field">
        <span className="filter-label">Contract ID</span>
        <input
          type="text"
          className="filter-input"
          value={filter.contractId}
          onChange={(e) => set({ contractId: e.target.value })}
          placeholder="e.g. CABC… or partial hex"
          aria-label="Filter by contract ID"
          spellCheck={false}
          autoComplete="off"
        />
      </label>

      {/* Topic */}
      <label className="filter-field">
        <span className="filter-label">Event Topic</span>
        <input
          type="text"
          className="filter-input"
          value={filter.topic}
          onChange={(e) => set({ topic: e.target.value })}
          placeholder="e.g. transfer, mint…"
          aria-label="Filter by event topic"
          spellCheck={false}
          autoComplete="off"
        />
      </label>

      {/* Ledger range */}
      <fieldset className="filter-ledger-range">
        <legend className="filter-label">Ledger range</legend>
        <input
          type="number"
          className="filter-input filter-input--ledger"
          value={filter.ledgerFrom}
          onChange={(e) => set({ ledgerFrom: e.target.value })}
          placeholder="From"
          aria-label="Ledger from"
          min="0"
        />
        <span className="filter-ledger-sep" aria-hidden>–</span>
        <input
          type="number"
          className="filter-input filter-input--ledger"
          value={filter.ledgerTo}
          onChange={(e) => set({ ledgerTo: e.target.value })}
          placeholder="To"
          aria-label="Ledger to"
          min="0"
        />
      </fieldset>

      {/* Event type */}
      <label className="filter-field">
        <span className="filter-label">Event type</span>
        <select
          className="filter-select"
          value={filter.eventType}
          onChange={(e) =>
            set({ eventType: e.target.value as FilterState['eventType'] })
          }
          aria-label="Filter by event type"
        >
          <option value="all">All types</option>
          <option value="contract">Contract</option>
          <option value="system">System</option>
          <option value="diagnostic">Diagnostic</option>
        </select>
      </label>

      {/* Actions */}
      <div className="filter-actions">
        {hasActiveFilter && (
          <button
            type="button"
            className="filter-btn filter-btn--reset"
            onClick={() =>
              onChange({ ...DEFAULT_FILTER, paused: filter.paused })
            }
            aria-label="Clear filters"
          >
            Clear filters
          </button>
        )}
        <button
          type="button"
          className={`filter-btn filter-btn--pause ${filter.paused ? 'active' : ''}`}
          onClick={() => set({ paused: !filter.paused })}
          aria-pressed={filter.paused}
          aria-label={filter.paused ? 'Resume stream' : 'Pause stream'}
        >
          {filter.paused ? '▶ Resume' : '⏸ Pause'}
        </button>
        <button
          type="button"
          className="filter-btn filter-btn--clear"
          onClick={onClear}
          aria-label="Clear all buffered events"
        >
          Clear buffer
        </button>
      </div>

      {/* Match count */}
      <div className="filter-count" aria-live="polite" aria-atomic>
        {hasActiveFilter ? (
          <>
            <strong>{matchCount.toLocaleString()}</strong>
            <span> / {totalCount.toLocaleString()} events</span>
          </>
        ) : (
          <span>{totalCount.toLocaleString()} events buffered</span>
        )}
      </div>
    </div>
  );
}
