/**
 * Type definitions and constants for the Geospatial Quorum & Latency Map.
 * Uses JSDoc for type safety without requiring a separate TypeScript compilation.
 *
 * @module geo_map/types
 */

/**
 * @typedef {Object} GeoCoord
 * @property {number} lat  - Latitude in degrees  (-90  … +90)
 * @property {number} lng  - Longitude in degrees (-180 … +180)
 */

/**
 * @typedef {Object} PeerInfo
 * @property {string}   id        - Unique validator / peer identifier (public key or hostname)
 * @property {string}   ip        - IPv4 or IPv6 address (used for GeoIP lookup)
 * @property {number}   latencyMs - Round-trip ping latency in milliseconds
 * @property {string}   [label]   - Optional human-readable label / name
 * @property {GeoCoord} [coord]   - Pre-resolved coordinates (skips GeoIP lookup when set)
 */

/**
 * @typedef {Object} GeoResolvedPeer
 * @property {string}   id        - Same as PeerInfo.id
 * @property {string}   ip        - Same as PeerInfo.ip
 * @property {number}   latencyMs - Same as PeerInfo.latencyMs
 * @property {string}   [label]
 * @property {GeoCoord} coord     - Resolved geographic coordinates
 * @property {string}   region    - Human-readable region name (e.g. "Tokyo, JP")
 */

/**
 * @typedef {Object} ArcDatum
 * @property {string}   peerId      - Peer identifier
 * @property {GeoCoord} from        - Host node coordinates
 * @property {GeoCoord} to          - Peer node coordinates
 * @property {number}   latencyMs   - Latency used to derive color
 * @property {number}   color       - 0xRRGGBB hex color derived from latency
 * @property {string}   latencyBand - 'good' | 'warn' | 'critical'
 */

/**
 * Latency thresholds (milliseconds) that drive arc color coding.
 *   < LATENCY_GOOD_MS         → green  (good)
 *   < LATENCY_CRITICAL_MS     → yellow (warn)
 *   ≥ LATENCY_CRITICAL_MS     → red    (critical)
 */
export const LATENCY_GOOD_MS = 50;
export const LATENCY_CRITICAL_MS = 200;

/** Arc colors as 0xRRGGBB integers. */
export const ARC_COLOR_GOOD = 0x39d98a;     // green
export const ARC_COLOR_WARN = 0xf5b942;     // yellow
export const ARC_COLOR_CRITICAL = 0xf05d5e; // red

/** Globe visual constants. */
export const GLOBE_RADIUS = 1.0;
export const GLOBE_SEGMENTS = 64;
export const GLOBE_COLOR = 0x0d1f2d;
export const GLOBE_WIREFRAME_COLOR = 0x1a3a5c;
export const GLOBE_GRATICULE_COLOR = 0x1e3a5f;
export const HOST_NODE_COLOR = 0xffd700;    // gold – the local validator
export const PEER_NODE_COLOR = 0xaaaaaa;    // grey – remote quorum peers
