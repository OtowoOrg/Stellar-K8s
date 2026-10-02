/**
 * GeoIP lookup utility for the Geospatial Quorum & Latency Map.
 *
 * In production, coordinates are resolved via the local ipapi.co-compatible
 * endpoint proxied at /api/geoip/:ip.  A built-in static table covers the
 * well-known Stellar testnet / mainnet validator IP ranges and is used as a
 * fast-path cache, removing network round-trips for common addresses.
 *
 * The module exposes:
 *   - resolveCoords(ip)          → Promise<GeoCoord>
 *   - resolveAllPeers(peers)     → Promise<GeoResolvedPeer[]>
 *   - latencyBand(latencyMs)     → 'good' | 'warn' | 'critical'
 *   - latencyColor(latencyMs)    → 0xRRGGBB integer
 *   - coordToVec3(coord, radius) → {x, y, z}   (Three.js-ready)
 *
 * @module geo_map/geoip
 */

import {
  LATENCY_GOOD_MS,
  LATENCY_CRITICAL_MS,
  ARC_COLOR_GOOD,
  ARC_COLOR_WARN,
  ARC_COLOR_CRITICAL,
  GLOBE_RADIUS,
} from './types.js';

// ---------------------------------------------------------------------------
// Static geo-database
// A lightweight IP-prefix → region table.  Entries are matched by exact IP
// first, then by /24 prefix, then by /16 prefix.  New entries can be added
// here to avoid external API calls during development / CI.
// ---------------------------------------------------------------------------

/** @type {Map<string, import('./types.js').GeoCoord & {region: string}>} */
const STATIC_DB = new Map([
  // --- well-known Stellar validator anchors (exact IPs) ---
  // Tokyo
  ['13.230.118.60',   { lat: 35.6762, lng: 139.6503, region: 'Tokyo, JP' }],
  ['52.69.1.1',       { lat: 35.6762, lng: 139.6503, region: 'Tokyo, JP' }],
  // Frankfurt
  ['18.185.0.1',      { lat: 50.1109, lng:   8.6821, region: 'Frankfurt, DE' }],
  ['3.64.0.1',        { lat: 50.1109, lng:   8.6821, region: 'Frankfurt, DE' }],
  // US East (Virginia / N. Virginia)
  ['3.80.0.1',        { lat: 38.9072, lng: -77.0369, region: 'Virginia, US' }],
  ['54.80.0.1',       { lat: 38.9072, lng: -77.0369, region: 'Virginia, US' }],
  ['34.194.0.1',      { lat: 38.9072, lng: -77.0369, region: 'Virginia, US' }],
  // Singapore
  ['54.179.0.1',      { lat:  1.3521, lng: 103.8198, region: 'Singapore, SG' }],
  // London
  ['35.178.0.1',      { lat: 51.5074, lng:  -0.1278, region: 'London, GB' }],
  // São Paulo
  ['18.229.0.1',      { lat: -23.5505, lng: -46.6333, region: 'São Paulo, BR' }],
  // Sydney
  ['13.236.0.1',      { lat: -33.8688, lng: 151.2093, region: 'Sydney, AU' }],
  // Localhost / loopback (used in tests)
  ['127.0.0.1',       { lat: 0, lng: 0, region: 'Localhost' }],
  ['::1',             { lat: 0, lng: 0, region: 'Localhost' }],
]);

/**
 * Look up the static database by exact IP, then /24, then /16.
 *
 * @param {string} ip
 * @returns {{lat: number, lng: number, region: string} | null}
 */
function staticLookup(ip) {
  if (STATIC_DB.has(ip)) return STATIC_DB.get(ip);

  // IPv4 prefix fallback
  const parts = ip.split('.');
  if (parts.length === 4) {
    const slash24 = `${parts[0]}.${parts[1]}.${parts[2]}.0`;
    if (STATIC_DB.has(slash24)) return STATIC_DB.get(slash24);
    const slash16 = `${parts[0]}.${parts[1]}.0.0`;
    if (STATIC_DB.has(slash16)) return STATIC_DB.get(slash16);
  }
  return null;
}

// ---------------------------------------------------------------------------
// In-memory runtime cache (resolved entries during the session)
// ---------------------------------------------------------------------------
/** @type {Map<string, {lat: number, lng: number, region: string}>} */
const _resolvedCache = new Map();

/**
 * Resolves geographic coordinates for a single IP address.
 * Resolution order: runtime cache → static DB → /api/geoip/:ip endpoint.
 *
 * @param {string} ip
 * @returns {Promise<{lat: number, lng: number, region: string}>}
 */
export async function resolveCoords(ip) {
  if (_resolvedCache.has(ip)) return _resolvedCache.get(ip);

  const staticHit = staticLookup(ip);
  if (staticHit) {
    _resolvedCache.set(ip, staticHit);
    return staticHit;
  }

  // Attempt live lookup via the backend proxy.
  try {
    const response = await fetch(`/api/geoip/${encodeURIComponent(ip)}`, {
      signal: AbortSignal.timeout(3000),
    });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const data = await response.json();
    if (typeof data.latitude !== 'number' || typeof data.longitude !== 'number') {
      throw new Error('Invalid GeoIP response schema');
    }
    const result = {
      lat: data.latitude,
      lng: data.longitude,
      region: data.city ? `${data.city}, ${data.country_code ?? ''}`.trim().replace(/,\s*$/, '') : (data.country_name ?? 'Unknown'),
    };
    _resolvedCache.set(ip, result);
    return result;
  } catch (_err) {
    // Fallback: place at (0,0) with a warning label so the globe still renders.
    const fallback = { lat: 0, lng: 0, region: 'Unknown' };
    _resolvedCache.set(ip, fallback);
    return fallback;
  }
}

/**
 * Resolves all peers concurrently, skipping peers that already have coords.
 *
 * @param {import('./types.js').PeerInfo[]} peers
 * @returns {Promise<import('./types.js').GeoResolvedPeer[]>}
 */
export async function resolveAllPeers(peers) {
  return Promise.all(
    peers.map(async (peer) => {
      if (peer.coord) {
        return {
          ...peer,
          coord: peer.coord,
          region: peer.region ?? 'Unknown',
        };
      }
      const { lat, lng, region } = await resolveCoords(peer.ip);
      return { ...peer, coord: { lat, lng }, region };
    }),
  );
}

// ---------------------------------------------------------------------------
// Latency helpers
// ---------------------------------------------------------------------------

/**
 * Classifies a latency value into a named band.
 *
 * @param {number} latencyMs
 * @returns {'good' | 'warn' | 'critical'}
 */
export function latencyBand(latencyMs) {
  if (latencyMs < LATENCY_GOOD_MS) return 'good';
  if (latencyMs < LATENCY_CRITICAL_MS) return 'warn';
  return 'critical';
}

/**
 * Returns a 0xRRGGBB integer for a given latency value.
 *
 * @param {number} latencyMs
 * @returns {number}
 */
export function latencyColor(latencyMs) {
  const band = latencyBand(latencyMs);
  if (band === 'good') return ARC_COLOR_GOOD;
  if (band === 'warn') return ARC_COLOR_WARN;
  return ARC_COLOR_CRITICAL;
}

// ---------------------------------------------------------------------------
// 3-D coordinate helpers
// ---------------------------------------------------------------------------

/**
 * Converts geographic coordinates (lat/lng in degrees) to a Cartesian
 * position on the surface of a sphere with the given radius.
 *
 * The coordinate system matches Three.js conventions:
 *   +Y  = North Pole
 *   +X  = prime meridian / equator intersection
 *   +Z  = 90°W / 90°E
 *
 * @param {{lat: number, lng: number}} coord
 * @param {number} [radius=GLOBE_RADIUS]
 * @returns {{x: number, y: number, z: number}}
 */
export function coordToVec3(coord, radius = GLOBE_RADIUS) {
  const phi   = (90 - coord.lat)  * (Math.PI / 180);
  const theta = (coord.lng + 180) * (Math.PI / 180);
  return {
    x:  Math.sin(phi) * Math.cos(theta) * radius,
    y:  Math.cos(phi) * radius,
    z: -Math.sin(phi) * Math.sin(theta) * radius,
  };
}

/**
 * Generates a quadratic Bézier arc between two surface points.
 * The midpoint control is lifted above the globe surface proportionally
 * to the angular distance between the two points.
 *
 * @param {{x:number,y:number,z:number}} p1
 * @param {{x:number,y:number,z:number}} p2
 * @param {number} [segments=64]   Number of line segments
 * @param {number} [radius=GLOBE_RADIUS]
 * @returns {Float32Array}  Flat [x,y,z, x,y,z, …] buffer for LineGeometry
 */
export function buildArcPositions(p1, p2, segments = 64, radius = GLOBE_RADIUS) {
  // Mid-point lifted above the surface
  const mx = (p1.x + p2.x) / 2;
  const my = (p1.y + p2.y) / 2;
  const mz = (p1.z + p2.z) / 2;
  const midLen = Math.sqrt(mx * mx + my * my + mz * mz) || 1;
  // Arc height proportional to chord length (max 40 % above surface)
  const chord = Math.sqrt(
    (p2.x - p1.x) ** 2 + (p2.y - p1.y) ** 2 + (p2.z - p1.z) ** 2,
  );
  const liftFactor = radius + chord * 0.4;
  const ctrl = {
    x: (mx / midLen) * liftFactor,
    y: (my / midLen) * liftFactor,
    z: (mz / midLen) * liftFactor,
  };

  const positions = new Float32Array((segments + 1) * 3);
  for (let i = 0; i <= segments; i++) {
    const t = i / segments;
    const u = 1 - t;
    positions[i * 3]     = u * u * p1.x + 2 * u * t * ctrl.x + t * t * p2.x;
    positions[i * 3 + 1] = u * u * p1.y + 2 * u * t * ctrl.y + t * t * p2.y;
    positions[i * 3 + 2] = u * u * p1.z + 2 * u * t * ctrl.z + t * t * p2.z;
  }
  return positions;
}

/**
 * Clears the runtime resolution cache.
 * Useful in tests or when the peer set changes dramatically.
 */
export function clearCache() {
  _resolvedCache.clear();
}
