/**
 * event_stream.ts
 *
 * WebSocket-based event stream service for Soroban contract events.
 * Handles connection lifecycle, reconnection back-off, and batched delivery
 * via requestAnimationFrame so the UI never blocks on high-frequency streams
 * (100+ events/sec is typical for busy Soroban testnet nodes).
 */

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/** Raw Soroban contract event as received from the WebSocket feed. */
export interface RawContractEvent {
  /** Unique monotonic ID assigned by the event generator / node. */
  id: string;
  /** ISO-8601 timestamp from the ledger close. */
  timestamp: string;
  /** Ledger sequence number in which this event was emitted. */
  ledger: number;
  /** Bech32m contract address (C…) or hex contract ID. */
  contract_id: string;
  /**
   * Array of XDR-encoded topic ScVals, each base64-encoded.
   * The first element is typically the event name symbol.
   */
  topics: string[];
  /**
   * XDR-encoded value ScVal payload, base64-encoded.
   * Decode with `decodeScVal` from xdr_decoder.ts.
   */
  value_xdr: string;
  /** "contract" | "system" | "diagnostic" */
  event_type: 'contract' | 'system' | 'diagnostic';
  /** Transaction hash that produced this event (hex). */
  tx_hash: string;
}

/** Callback invoked with a batch of newly-arrived events. */
export type EventBatchCallback = (batch: RawContractEvent[]) => void;

/** Callback invoked whenever the connection state changes. */
export type ConnectionStateCallback = (state: ConnectionState) => void;

export type ConnectionState =
  | 'connecting'
  | 'live'
  | 'reconnecting'
  | 'offline'
  | 'error';

export interface EventStreamOptions {
  /** WebSocket URL to connect to (default: auto-derived from window.location). */
  url?: string;
  /**
   * Maximum events to retain in the internal ring-buffer before the
   * oldest are dropped. Default: 10_000.
   */
  maxBuffer?: number;
  /**
   * Initial reconnect back-off in ms. Doubles on each failure up to
   * `maxBackoffMs`. Default: 500.
   */
  initialBackoffMs?: number;
  /** Maximum reconnect back-off in ms. Default: 30_000. */
  maxBackoffMs?: number;
}

// ---------------------------------------------------------------------------
// EventStreamService
// ---------------------------------------------------------------------------

/**
 * Manages a single WebSocket connection to the Soroban event stream.
 *
 * Usage:
 * ```ts
 * const svc = new EventStreamService({ url: 'ws://localhost:8788' });
 * svc.onBatch((events) => { ... });
 * svc.onStateChange((state) => { ... });
 * svc.connect();
 * // later...
 * svc.disconnect();
 * ```
 */
export class EventStreamService {
  private readonly url: string;
  private readonly maxBuffer: number;
  private readonly initialBackoffMs: number;
  private readonly maxBackoffMs: number;

  private socket: WebSocket | null = null;
  private disposed = false;
  private backoffMs: number;

  private batchCallbacks: Set<EventBatchCallback> = new Set();
  private stateCallbacks: Set<ConnectionStateCallback> = new Set();

  /** Pending events waiting to be flushed via rAF. */
  private pending: RawContractEvent[] = [];
  private rafHandle: number | null = null;

  /** Current connection state (read-only externally). */
  private _state: ConnectionState = 'connecting';

  /** Timestamp of the last successfully received message. */
  public lastMessageAt: Date | null = null;

  /** Total events received since connect() was called. */
  public totalReceived = 0;

  /** Events received in the last second (rolling). */
  public eventsPerSecond = 0;
  private _epsWindow: number[] = [];

  constructor(options: EventStreamOptions = {}) {
    this.url =
      options.url ?? EventStreamService.defaultUrl();
    this.maxBuffer = options.maxBuffer ?? 10_000;
    this.initialBackoffMs = options.initialBackoffMs ?? 500;
    this.maxBackoffMs = options.maxBackoffMs ?? 30_000;
    this.backoffMs = this.initialBackoffMs;

    // Rolling EPS meter — tick every second.
    setInterval(() => this.tickEps(), 1_000);
  }

  // -------------------------------------------------------------------------
  // Public API
  // -------------------------------------------------------------------------

  get state(): ConnectionState {
    return this._state;
  }

  /** Register a callback that receives batches of new events. */
  onBatch(cb: EventBatchCallback): () => void {
    this.batchCallbacks.add(cb);
    return () => this.batchCallbacks.delete(cb);
  }

  /** Register a callback that fires on connection state changes. */
  onStateChange(cb: ConnectionStateCallback): () => void {
    this.stateCallbacks.add(cb);
    return () => this.stateCallbacks.delete(cb);
  }

  /** Open the WebSocket connection. Idempotent. */
  connect(): void {
    if (this.socket && this.socket.readyState <= WebSocket.OPEN) return;
    this.disposed = false;
    this.openSocket();
  }

  /** Permanently close the connection and stop reconnecting. */
  disconnect(): void {
    this.disposed = true;
    if (this.rafHandle !== null) {
      cancelAnimationFrame(this.rafHandle);
      this.rafHandle = null;
    }
    this.socket?.close();
    this.socket = null;
    this.setState('offline');
  }

  // -------------------------------------------------------------------------
  // Private helpers
  // -------------------------------------------------------------------------

  private openSocket(): void {
    this.setState('connecting');
    let ws: WebSocket;
    try {
      ws = new WebSocket(this.url);
    } catch {
      this.setState('error');
      this.scheduleReconnect();
      return;
    }
    this.socket = ws;

    ws.onopen = () => {
      this.backoffMs = this.initialBackoffMs;
      this.setState('live');
    };

    ws.onmessage = (ev: MessageEvent) => {
      if (this.disposed) return;
      try {
        const payload = JSON.parse(ev.data as string) as
          | RawContractEvent
          | RawContractEvent[];
        const events = Array.isArray(payload) ? payload : [payload];
        this.totalReceived += events.length;
        this.lastMessageAt = new Date();
        this._epsWindow.push(Date.now());
        this.enqueue(events);
      } catch {
        // Malformed frame — ignore but don't disconnect.
        console.warn('[EventStreamService] Failed to parse frame', ev.data);
      }
    };

    ws.onerror = () => {
      this.setState('error');
    };

    ws.onclose = () => {
      if (this.disposed) return;
      this.setState('reconnecting');
      this.scheduleReconnect();
    };
  }

  private scheduleReconnect(): void {
    if (this.disposed) return;
    const delay = this.backoffMs;
    this.backoffMs = Math.min(this.backoffMs * 2, this.maxBackoffMs);
    setTimeout(() => {
      if (!this.disposed) this.openSocket();
    }, delay);
  }

  private setState(state: ConnectionState): void {
    if (this._state === state) return;
    this._state = state;
    this.stateCallbacks.forEach((cb) => cb(state));
  }

  /**
   * Enqueue events and schedule a single rAF flush.
   * This coalesces rapid-fire messages into one React render.
   */
  private enqueue(events: RawContractEvent[]): void {
    for (const e of events) this.pending.push(e);
    if (this.rafHandle !== null) return;
    this.rafHandle = requestAnimationFrame(() => {
      this.rafHandle = null;
      const batch = this.pending.splice(0);
      if (batch.length === 0) return;
      this.batchCallbacks.forEach((cb) => cb(batch));
    });
  }

  private tickEps(): void {
    const now = Date.now();
    const cutoff = now - 1_000;
    // Trim timestamps older than 1 second.
    let i = 0;
    while (i < this._epsWindow.length && this._epsWindow[i] < cutoff) i++;
    this._epsWindow = this._epsWindow.slice(i);
    this.eventsPerSecond = this._epsWindow.length;
  }

  private static defaultUrl(): string {
    const proto =
      window.location.protocol === 'https:' ? 'wss' : 'ws';
    return `${proto}://${window.location.host}/ws/events`;
  }
}

// ---------------------------------------------------------------------------
// React hook — useEventStream
// ---------------------------------------------------------------------------

/**
 * React hook that connects to the event stream and returns a live-updated
 * ring-buffer of the latest `capacity` events plus connection state.
 *
 * Events are prepended (newest-first) so the table always shows the latest
 * at the top without needing to re-sort.
 */
import { useEffect, useRef, useState } from 'react';

export interface UseEventStreamResult {
  events: RawContractEvent[];
  state: ConnectionState;
  eventsPerSecond: number;
  totalReceived: number;
  lastMessageAt: Date | null;
  /** Clears the local ring-buffer without disconnecting. */
  clearEvents: () => void;
}

export function useEventStream(
  service: EventStreamService,
  capacity = 10_000,
): UseEventStreamResult {
  const [events, setEvents] = useState<RawContractEvent[]>([]);
  const [connState, setConnState] = useState<ConnectionState>(service.state);
  const [eps, setEps] = useState(0);
  const [total, setTotal] = useState(0);
  const [lastAt, setLastAt] = useState<Date | null>(null);

  // Ring-buffer stored in a ref so batch callbacks don't capture stale state.
  const bufRef = useRef<RawContractEvent[]>([]);

  // rAF handle for coalescing state flushes.
  const rafRef = useRef<number | null>(null);

  useEffect(() => {
    const unsubBatch = service.onBatch((batch) => {
      // Prepend newest events, trim to capacity.
      bufRef.current = [...batch.reverse(), ...bufRef.current].slice(
        0,
        capacity,
      );
      // Coalesce state updates into a single rAF.
      if (rafRef.current !== null) return;
      rafRef.current = requestAnimationFrame(() => {
        rafRef.current = null;
        setEvents([...bufRef.current]);
        setEps(service.eventsPerSecond);
        setTotal(service.totalReceived);
        setLastAt(service.lastMessageAt);
      });
    });

    const unsubState = service.onStateChange((s) => setConnState(s));

    return () => {
      unsubBatch();
      unsubState();
      if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
    };
  }, [service, capacity]);

  const clearEvents = () => {
    bufRef.current = [];
    setEvents([]);
  };

  return {
    events,
    state: connState,
    eventsPerSecond: eps,
    totalReceived: total,
    lastMessageAt: lastAt,
    clearEvents,
  };
}
