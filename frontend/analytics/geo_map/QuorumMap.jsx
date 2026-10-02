/**
 * QuorumMap – top-level component for the 3D Geospatial Quorum & Latency Map.
 *
 * Accepts a list of raw peer descriptors (IP + latency) and:
 *   1. Resolves geographic coordinates via the GeoIP utility.
 *   2. Derives latency-coded arc data (green / yellow / red).
 *   3. Renders the WebGLGlobe with the resolved arcs.
 *
 * Designed for issue #225: visualise cross-continental quorum peer connections
 * and consensus delays for a Stellar validator operator.
 *
 * Usage:
 *   <QuorumMap peers={[...]} hostIp="1.2.3.4" />
 *
 * @module analytics/geo_map/QuorumMap
 */

import { useState, useEffect, useMemo, useRef } from 'react';
import WebGLGlobe from '../../components/webgl_globe.js';
import { resolveAllPeers, latencyColor, latencyBand } from './geoip.js';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** Raw peer descriptor supplied by the caller. */
export interface PeerInput {
  /** Unique identifier (public key, hostname, etc.) */
  id: string;
  /** IPv4 or IPv6 address used for GeoIP lookup. */
  ip: string;
  /** Round-trip latency in milliseconds. */
  latencyMs: number;
  /** Optional human-readable label. */
  label?: string;
}

export interface QuorumMapProps {
  /** List of quorum peers to display. */
  peers: PeerInput[];
  /** IP address of the local (host) validator. */
  hostIp: string;
  /** Host node's geographic coordinates if already known (skips GeoIP lookup). */
  hostCoord?: { lat: number; lng: number };
  /** Container width in CSS pixels (defaults to 100%). */
  width?: number;
  /** Container height in CSS pixels (defaults to 500px). */
  height?: number;
  /** CSS class applied to the outer wrapper div. */
  className?: string;
}

// ---------------------------------------------------------------------------
// Loading / error state UI helpers
// ---------------------------------------------------------------------------

function LoadingOverlay() {
  return (
    <div
      style={{
        position: 'absolute', inset: 0,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        background: 'rgba(11,17,25,0.7)',
        color: '#aaa',
        fontSize: 14,
        zIndex: 2,
        pointerEvents: 'none',
      }}
      aria-live="polite"
      aria-label="Resolving peer locations…"
    >
      Resolving peer locations…
    </div>
  );
}

function ErrorBanner({ message }: { message: string }) {
  return (
    <div
      role="alert"
      style={{
        position: 'absolute', top: 8, left: 8, right: 8,
        background: '#3a1010',
        color: '#f05d5e',
        padding: '6px 10px',
        borderRadius: 4,
        fontSize: 13,
        zIndex: 3,
      }}
    >
      {message}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Latency legend
// ---------------------------------------------------------------------------

function LatencyLegend() {
  const items = [
    { label: '< 50 ms', color: '#39d98a' },
    { label: '50 – 200 ms', color: '#f5b942' },
    { label: '> 200 ms', color: '#f05d5e' },
  ];
  return (
    <div
      aria-label="Latency colour legend"
      style={{
        position: 'absolute', bottom: 12, left: 12,
        background: 'rgba(11,17,25,0.8)',
        borderRadius: 6,
        padding: '6px 10px',
        display: 'flex',
        flexDirection: 'column',
        gap: 4,
        zIndex: 2,
        pointerEvents: 'none',
      }}
    >
      {items.map(({ label, color }) => (
        <div key={label} style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
          <span
            style={{
              width: 12, height: 12,
              borderRadius: '50%',
              background: color,
              flexShrink: 0,
            }}
            aria-hidden="true"
          />
          <span style={{ color: '#ccc', fontSize: 12 }}>{label}</span>
        </div>
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Peer info tooltip (appears bottom-right on hover if JS-accessible)
// ---------------------------------------------------------------------------

// No DOM-level hover events are wired to the Three.js canvas in this version;
// this component is a placeholder for future raycasting-based selection.

// ---------------------------------------------------------------------------
// QuorumMap component
// ---------------------------------------------------------------------------

// Default host coordinate used when GeoIP lookup of the host IP is pending.
const DEFAULT_HOST_COORD = { lat: 38.9072, lng: -77.0369 }; // US East fallback

export default function QuorumMap({
  peers,
  hostIp,
  hostCoord: hostCoordProp,
  width,
  height = 500,
  className,
}: QuorumMapProps) {
  const [resolvedPeers, setResolvedPeers] = useState<
    Array<{ id: string; ip: string; latencyMs: number; label?: string; coord: { lat: number; lng: number }; region: string }>
  >([]);
  const [hostCoord, setHostCoord] = useState<{ lat: number; lng: number }>(
    hostCoordProp ?? DEFAULT_HOST_COORD,
  );
  const [loading, setLoading] = useState(true);
  const [error, setError]     = useState<string | null>(null);

  // Re-resolve when peers or hostIp changes.
  const resolveGenRef = useRef(0);

  useEffect(() => {
    if (hostCoordProp) {
      setHostCoord(hostCoordProp);
    }
  }, [hostCoordProp]);

  useEffect(() => {
    let cancelled = false;
    const gen = ++resolveGenRef.current;
    setLoading(true);
    setError(null);

    const allInputs = [
      // Prepend the host as a special peer entry so its coord is resolved too.
      { id: '__host__', ip: hostIp, latencyMs: 0 },
      ...peers,
    ];

    resolveAllPeers(allInputs)
      .then((resolved) => {
        if (cancelled || gen !== resolveGenRef.current) return;

        const hostEntry = resolved.find((p) => p.id === '__host__');
        if (hostEntry && !hostCoordProp) {
          setHostCoord(hostEntry.coord);
        }

        setResolvedPeers(
          resolved
            .filter((p) => p.id !== '__host__')
            .map((p) => ({ ...p, coord: p.coord })),
        );
        setLoading(false);
      })
      .catch((err: Error) => {
        if (cancelled) return;
        setError(`GeoIP resolution failed: ${err.message}`);
        setLoading(false);
      });

    return () => { cancelled = true; };
  }, [peers, hostIp, hostCoordProp]);

  // Build arc data from resolved peers.
  const arcs = useMemo(
    () =>
      resolvedPeers.map((peer) => ({
        peerId:     peer.id,
        from:       hostCoord,
        to:         peer.coord,
        latencyMs:  peer.latencyMs,
        color:      latencyColor(peer.latencyMs),
        band:       latencyBand(peer.latencyMs),
      })),
    [resolvedPeers, hostCoord],
  );

  const containerStyle: React.CSSProperties = {
    position: 'relative',
    width:    width  ? `${width}px`  : '100%',
    height:   height ? `${height}px` : '500px',
    overflow: 'hidden',
    borderRadius: 8,
    background: '#0b1119',
  };

  return (
    <div
      className={className}
      style={containerStyle}
      data-testid="quorum-map"
    >
      {error && <ErrorBanner message={error} />}
      {loading && <LoadingOverlay />}

      <WebGLGlobe
        arcs={arcs}
        hostCoord={hostCoord}
        ariaLabel={`Quorum map – ${arcs.length} peer connections from ${hostIp}`}
      />

      <LatencyLegend />

      {/* Screen-reader summary */}
      <div className="sr-only" aria-live="polite">
        {loading
          ? 'Loading peer locations…'
          : `Displaying ${arcs.length} quorum peer connection${arcs.length !== 1 ? 's' : ''}.`}
      </div>
    </div>
  );
}
