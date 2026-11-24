/**
 * App.tsx — Soroban Contract Event Stream Inspector
 *
 * Main application shell. Wires together:
 *   - EventStreamService (WebSocket connection + rAF batching)
 *   - useEventStream hook (ring-buffer state)
 *   - FilterControls (contract / topic / ledger / type filtering)
 *   - EventTable (virtualized, click to inspect)
 *   - JSONModal (XDR payload inspector)
 *   - Performance profiling overlay (events/sec, render budget)
 */

import React, {
  StrictMode,
  useCallback,
  useMemo,
  useRef,
  useState,
} from 'react';
import { createRoot } from 'react-dom/client';

import { EventStreamService, useEventStream } from '../../services/event_stream';
import type { RawContractEvent } from '../../services/event_stream';
import {
  FilterControls,
  applyFilters,
  DEFAULT_FILTER,
} from './events/FilterControls';
import { EventTable } from './events/EventTable';
import { JSONModal } from './events/JSONModal';
import './styles.css';

// ---------------------------------------------------------------------------
// Determine WebSocket URL from query params (same pattern as analytics app)
// ---------------------------------------------------------------------------

const query = new URLSearchParams(window.location.search);
const wsFromQuery = query.get('ws');
const mockPort = 8788;

function buildWsUrl(): string {
  if (wsFromQuery) {
    // Explicit override: ?ws=ws://localhost:8788
    return wsFromQuery;
  }
  const proto = window.location.protocol === 'https:' ? 'wss' : 'ws';
  // Default: use mock stream when running locally without a backend.
  // In production, swap to: `${proto}://${window.location.host}/ws/events`
  return `${proto}://localhost:${mockPort}`;
}

// ---------------------------------------------------------------------------
// Singleton service (lives outside React so it's never recreated on re-render)
// ---------------------------------------------------------------------------

const service = new EventStreamService({ url: buildWsUrl(), maxBuffer: 10_000 });
service.connect();

// ---------------------------------------------------------------------------
// Performance profiler overlay
// ---------------------------------------------------------------------------

interface PerfCounters {
  eps: number;
  total: number;
  frameMs: number;
}

function usePerfProfiler(): PerfCounters & { frameRef: (ms: number) => void } {
  const [counters, setCounters] = useState<PerfCounters>({
    eps: 0,
    total: 0,
    frameMs: 0,
  });
  const frameMsRef = useRef(0);

  const frameRef = useCallback((ms: number) => {
    frameMsRef.current = ms;
  }, []);

  // Refresh display 4× per second — no need to pollute every rAF.
  React.useEffect(() => {
    const id = setInterval(() => {
      setCounters({
        eps: service.eventsPerSecond,
        total: service.totalReceived,
        frameMs: frameMsRef.current,
      });
    }, 250);
    return () => clearInterval(id);
  }, []);

  return { ...counters, frameRef };
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

function App() {
  const { events, state, clearEvents } = useEventStream(service, 10_000);
  const [filter, setFilter] = useState(DEFAULT_FILTER);
  const [selectedEvent, setSelectedEvent] = useState<RawContractEvent | null>(
    null,
  );
  const [source, setSource] = useState(wsFromQuery ?? `ws://localhost:${mockPort}`);
  const { eps, total, frameMs, frameRef } = usePerfProfiler();

  // Measure rAF render budget.
  const rafStart = useRef(0);
  React.useLayoutEffect(() => {
    rafStart.current = performance.now();
    return () => {
      const elapsed = performance.now() - rafStart.current;
      frameRef(elapsed);
    };
  });

  // Apply filters (memoised — runs only when events or filter changes).
  const filtered = useMemo(
    () => (filter.paused ? events : applyFilters(events, filter)),
    [events, filter],
  );

  const handleInspect = useCallback((event: RawContractEvent) => {
    setSelectedEvent(event);
  }, []);

  const handleModalClose = useCallback(() => setSelectedEvent(null), []);

  const handleSourceChange = useCallback(
    (e: React.ChangeEvent<HTMLSelectElement>) => {
      const val = e.target.value;
      setSource(val);
      // Reconnect with new URL.
      service.disconnect();
      // Give EventStreamService a new URL. We recreate it instead of mutating.
      // (In production you'd support a proper "setUrl" method; for demo purposes
      //  we redirect to the new source via query param.)
      const url = new URL(window.location.href);
      url.searchParams.set('ws', val);
      window.location.href = url.toString();
    },
    [],
  );

  return (
    <main className="app-shell">
      {/* Topbar */}
      <header className="topbar">
        <div className="brand-block">
          <span className="eyebrow">STELLAR / SOROBAN</span>
          <h1>Contract Event Stream</h1>
          <p>Real-time Soroban contract event inspector with XDR decoding.</p>
        </div>

        <div className="toolbar" role="toolbar" aria-label="Inspector controls">
          {/* Source selector */}
          <label className="select-wrap">
            <span>Data source</span>
            <select value={source} onChange={handleSourceChange}>
              <option value={`ws://localhost:${mockPort}`}>
                Mock stream (local)
              </option>
              <option value="ws://localhost:8080/ws/events">
                Soroban RPC (local)
              </option>
              <option value="wss://horizon-testnet.stellar.org/soroban/events">
                Testnet RPC
              </option>
            </select>
          </label>

          {/* Connection badge */}
          <div
            className={`conn-badge conn-badge--${state}`}
            role="status"
            aria-label={`Connection: ${state}`}
          >
            <span className={`status-dot ${state === 'live' ? 'live' : state === 'connecting' || state === 'reconnecting' ? 'connecting' : 'error'}`} />
            {state}
          </div>
        </div>
      </header>

      {/* Metric strip */}
      <section className="metric-strip" aria-label="Stream statistics">
        <Metric
          label="Events/sec"
          value={eps.toLocaleString()}
          detail="rolling 1 s"
          tone={eps > 50 ? 'amber' : 'green'}
        />
        <Metric
          label="Total received"
          value={total.toLocaleString()}
          detail="since connect"
        />
        <Metric
          label="Buffered"
          value={events.length.toLocaleString()}
          detail={`of ${(10_000).toLocaleString()} max`}
        />
        <Metric
          label="Filtered"
          value={filtered.length.toLocaleString()}
          detail="matching events"
          tone={filtered.length < events.length ? 'amber' : undefined}
        />
        <Metric
          label="Frame budget"
          value={`${frameMs.toFixed(1)} ms`}
          detail="last render"
          tone={frameMs > 16 ? 'red' : 'green'}
        />
      </section>

      {/* Filter bar */}
      <FilterControls
        filter={filter}
        onChange={setFilter}
        onClear={clearEvents}
        matchCount={filtered.length}
        totalCount={events.length}
      />

      {/* Main event table */}
      <section className="event-panel" aria-label="Event stream">
        <EventTable
          events={filtered}
          onInspect={handleInspect}
          selectedId={selectedEvent?.id ?? null}
          autoScroll={!filter.paused}
        />
      </section>

      {/* Performance log — visible only in dev or when query param ?perf=1 */}
      {(query.get('perf') === '1' ||
        import.meta.env.DEV) && (
        <PerfOverlay eps={eps} total={total} frameMs={frameMs} />
      )}

      {/* JSON inspector modal */}
      <JSONModal event={selectedEvent} onClose={handleModalClose} />
    </main>
  );
}

// ---------------------------------------------------------------------------
// Helper components
// ---------------------------------------------------------------------------

function Metric({
  label,
  value,
  detail,
  tone,
}: {
  label: string;
  value: string;
  detail: string;
  tone?: 'green' | 'amber' | 'red';
}) {
  return (
    <div className="metric">
      <span className="metric-label">{label}</span>
      <strong className={tone ? `tone-${tone}` : ''}>{value}</strong>
      <span className="muted">{detail}</span>
    </div>
  );
}

function PerfOverlay({
  eps,
  total,
  frameMs,
}: {
  eps: number;
  total: number;
  frameMs: number;
}) {
  return (
    <div className="perf-overlay" role="log" aria-label="Performance log">
      <span className="eyebrow">PERF</span>
      <pre>
        {`EPS       : ${eps.toString().padStart(6)}
Total     : ${total.toString().padStart(6)}
Frame     : ${frameMs.toFixed(2).padStart(6)} ms
Budget OK : ${frameMs <= 16 ? '✓' : '✗ OVER BUDGET'}`}
      </pre>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Bootstrap
// ---------------------------------------------------------------------------

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
