/**
 * EventTable.tsx
 *
 * High-performance virtualized table for displaying Soroban contract events.
 *
 * At 100–200 events/sec, rendering every row in the DOM would saturate the
 * main thread within seconds. Instead we:
 *   1. Keep the full event list in memory (ring-buffer, max 10 000).
 *   2. Only render the ~20 rows visible in the scroll viewport.
 *   3. Use requestAnimationFrame batching (from the service) so React
 *      reconciliation runs at most once per frame.
 *
 * The custom `useVirtualList` hook replaces react-virtual / react-window so
 * we stay dependency-free (matching the analytics app pattern).
 */

import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react';
import type { RawContractEvent } from '../../services/event_stream';
import { extractEventName, decodeScVal } from './xdr_decoder';

// ---------------------------------------------------------------------------
// useVirtualList hook
// ---------------------------------------------------------------------------

const ROW_HEIGHT = 42; // px — must match CSS .event-row height

interface VirtualWindow {
  start: number;
  end: number;
  offsetY: number; // px — top padding for the rendered slice
  totalHeight: number; // px — full scrollable height
}

function useVirtualList(
  containerRef: React.RefObject<HTMLDivElement>,
  count: number,
  rowHeight: number,
  overscan = 5,
): VirtualWindow {
  const [win, setWin] = useState<VirtualWindow>({
    start: 0,
    end: Math.min(count, 30),
    offsetY: 0,
    totalHeight: count * rowHeight,
  });

  useLayoutEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const recalc = () => {
      const scrollTop = el.scrollTop;
      const viewportH = el.clientHeight;
      const start = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan);
      const visibleCount = Math.ceil(viewportH / rowHeight);
      const end = Math.min(count, start + visibleCount + overscan * 2);
      setWin({
        start,
        end,
        offsetY: start * rowHeight,
        totalHeight: count * rowHeight,
      });
    };

    recalc();
    el.addEventListener('scroll', recalc, { passive: true });
    const ro = new ResizeObserver(recalc);
    ro.observe(el);
    return () => {
      el.removeEventListener('scroll', recalc);
      ro.disconnect();
    };
  }, [containerRef, count, rowHeight, overscan]);

  // Recalc whenever count changes (new events arrive).
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const scrollTop = el.scrollTop;
    const viewportH = el.clientHeight;
    const start = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan);
    const visibleCount = Math.ceil(viewportH / rowHeight);
    const end = Math.min(count, start + visibleCount + overscan * 2);
    setWin({
      start,
      end,
      offsetY: start * rowHeight,
      totalHeight: count * rowHeight,
    });
  }, [count, rowHeight, overscan]);

  return win;
}

// ---------------------------------------------------------------------------
// EventRow — memoised so unchanged rows don't re-render
// ---------------------------------------------------------------------------

interface EventRowProps {
  event: RawContractEvent;
  style: React.CSSProperties;
  onClick: (event: RawContractEvent) => void;
  isSelected: boolean;
}

const EventRow = React.memo(function EventRow({
  event,
  style,
  onClick,
  isSelected,
}: EventRowProps) {
  const handleClick = useCallback(() => onClick(event), [onClick, event]);
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Enter' || e.key === ' ') {
        e.preventDefault();
        onClick(event);
      }
    },
    [onClick, event],
  );

  const eventName = extractEventName(event.topics) ?? '—';
  const firstTopic = event.topics[0] ?? '';
  const decodedFirst = firstTopic ? decodeScVal(firstTopic) : null;
  const topicDisplay =
    decodedFirst?.type === 'symbol' || decodedFirst?.type === 'string'
      ? decodedFirst.value
      : firstTopic.slice(0, 12) + (firstTopic.length > 12 ? '…' : '');

  const time = new Date(event.timestamp);
  const timeStr = `${time.toLocaleTimeString()}.${String(time.getMilliseconds()).padStart(3, '0')}`;

  return (
    <div
      role="row"
      tabIndex={0}
      className={`event-row event-row--${event.event_type}${isSelected ? ' event-row--selected' : ''}`}
      style={style}
      onClick={handleClick}
      onKeyDown={handleKeyDown}
      aria-selected={isSelected}
      title="Click to inspect payload"
    >
      {/* Ledger */}
      <span className="event-cell event-cell--ledger" role="cell">
        {event.ledger.toLocaleString()}
      </span>
      {/* Time */}
      <span className="event-cell event-cell--time" role="cell">
        {timeStr}
      </span>
      {/* Type badge */}
      <span
        className={`event-cell event-cell--type event-badge event-badge--${event.event_type}`}
        role="cell"
      >
        {event.event_type}
      </span>
      {/* Contract ID */}
      <span
        className="event-cell event-cell--contract"
        role="cell"
        title={event.contract_id}
      >
        {event.contract_id.slice(0, 8)}…{event.contract_id.slice(-4)}
      </span>
      {/* Event name (first symbol topic) */}
      <span
        className="event-cell event-cell--name"
        role="cell"
        title={topicDisplay}
      >
        {eventName}
      </span>
      {/* Topics count */}
      <span className="event-cell event-cell--topics" role="cell">
        {event.topics.length}
      </span>
      {/* TX hash */}
      <span
        className="event-cell event-cell--tx"
        role="cell"
        title={event.tx_hash}
      >
        {event.tx_hash.slice(0, 8)}…
      </span>
      {/* Inspect button */}
      <span className="event-cell event-cell--action" role="cell">
        <span className="event-inspect-icon" aria-hidden>⬡</span>
      </span>
    </div>
  );
});

// ---------------------------------------------------------------------------
// EventTable component
// ---------------------------------------------------------------------------

export interface EventTableProps {
  events: RawContractEvent[];
  onInspect: (event: RawContractEvent) => void;
  selectedId: string | null;
  /** When true the table scroll is locked to the top (newest events). */
  autoScroll: boolean;
}

export function EventTable({
  events,
  onInspect,
  selectedId,
  autoScroll,
}: EventTableProps) {
  const containerRef = useRef<HTMLDivElement>(null!);
  const vw = useVirtualList(containerRef, events.length, ROW_HEIGHT);

  // Auto-scroll to top when new events arrive (newest-first list).
  const prevLengthRef = useRef(0);
  useEffect(() => {
    if (!autoScroll) return;
    if (events.length !== prevLengthRef.current && containerRef.current) {
      containerRef.current.scrollTop = 0;
    }
    prevLengthRef.current = events.length;
  }, [events.length, autoScroll]);

  const slice = events.slice(vw.start, vw.end);

  return (
    <div className="event-table-wrapper">
      {/* Column headers */}
      <div className="event-header" role="row">
        <span className="event-cell event-cell--ledger" role="columnheader">Ledger</span>
        <span className="event-cell event-cell--time" role="columnheader">Time</span>
        <span className="event-cell event-cell--type" role="columnheader">Type</span>
        <span className="event-cell event-cell--contract" role="columnheader">Contract</span>
        <span className="event-cell event-cell--name" role="columnheader">Event name</span>
        <span className="event-cell event-cell--topics" role="columnheader">#</span>
        <span className="event-cell event-cell--tx" role="columnheader">TX hash</span>
        <span className="event-cell event-cell--action" role="columnheader" aria-label="Inspect" />
      </div>

      {/* Scrollable virtual list */}
      <div
        ref={containerRef}
        className="event-scroll"
        role="grid"
        aria-rowcount={events.length}
        aria-label="Soroban contract events"
        tabIndex={-1}
      >
        {events.length === 0 ? (
          <div className="event-empty">
            <span className="event-empty-icon">⬡</span>
            <p>Waiting for events…</p>
            <p className="event-empty-sub">
              Events emitted by Soroban smart contracts will appear here in
              real time.
            </p>
          </div>
        ) : (
          <div
            style={{ height: vw.totalHeight, position: 'relative' }}
            role="presentation"
          >
            {slice.map((event, i) => (
              <EventRow
                key={event.id}
                event={event}
                style={{
                  position: 'absolute',
                  top: vw.offsetY + (i * ROW_HEIGHT),
                  left: 0,
                  right: 0,
                  height: ROW_HEIGHT,
                }}
                onClick={onInspect}
                isSelected={event.id === selectedId}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
