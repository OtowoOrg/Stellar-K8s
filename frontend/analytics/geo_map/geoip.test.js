/**
 * Unit tests for the Geospatial Quorum & Latency Map utilities.
 *
 * Tests validate:
 *  1. GeoIP resolution for mock peers in Tokyo, Frankfurt, and Virginia.
 *  2. Latency band classification (good / warn / critical).
 *  3. Latency colour coding against issue #225 spec values.
 *  4. coordToVec3 / buildArcPositions 3-D geometry helpers.
 *  5. resolveAllPeers batch resolution.
 *
 * Run with: node --test frontend/analytics/geo_map/geoip.test.js
 */

import test from 'node:test';
import assert from 'node:assert/strict';

import {
  resolveCoords,
  resolveAllPeers,
  latencyBand,
  latencyColor,
  coordToVec3,
  buildArcPositions,
  clearCache,
} from './geoip.js';

import {
  LATENCY_GOOD_MS,
  LATENCY_CRITICAL_MS,
  ARC_COLOR_GOOD,
  ARC_COLOR_WARN,
  ARC_COLOR_CRITICAL,
  GLOBE_RADIUS,
} from './types.js';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Approximation equality for floating-point comparisons. */
function near(a, b, epsilon = 0.001) {
  return Math.abs(a - b) <= epsilon;
}

// ---------------------------------------------------------------------------
// 1. GeoIP static lookup – Tokyo, Frankfurt, Virginia
// ---------------------------------------------------------------------------

test('resolves Tokyo IP to correct coordinates via static database', async () => {
  clearCache();
  // 13.230.118.60 is present in the static DB as Tokyo
  const result = await resolveCoords('13.230.118.60');
  assert.ok(near(result.lat,  35.6762, 0.1), `Expected lat ~35.68, got ${result.lat}`);
  assert.ok(near(result.lng, 139.6503, 0.1), `Expected lng ~139.65, got ${result.lng}`);
  assert.match(result.region, /Tokyo/i);
});

test('resolves Frankfurt IP to correct coordinates via static database', async () => {
  clearCache();
  // 18.185.0.1 → Frankfurt
  const result = await resolveCoords('18.185.0.1');
  assert.ok(near(result.lat, 50.1109, 0.1), `Expected lat ~50.11, got ${result.lat}`);
  assert.ok(near(result.lng,  8.6821, 0.1), `Expected lng ~8.68, got ${result.lng}`);
  assert.match(result.region, /Frankfurt/i);
});

test('resolves Virginia (US East) IP to correct coordinates via static database', async () => {
  clearCache();
  // 3.80.0.1 → Virginia
  const result = await resolveCoords('3.80.0.1');
  assert.ok(near(result.lat, 38.9072, 0.1), `Expected lat ~38.91, got ${result.lat}`);
  assert.ok(near(result.lng, -77.0369, 0.1), `Expected lng ~-77.04, got ${result.lng}`);
  assert.match(result.region, /Virginia/i);
});

test('caches repeated lookups without additional work', async () => {
  clearCache();
  const a = await resolveCoords('13.230.118.60');
  const b = await resolveCoords('13.230.118.60');
  // Exact same object returned from cache
  assert.equal(a, b);
});

test('resolves unknown IP to fallback (0, 0) coords', async () => {
  clearCache();
  // No backend available in test environment → fallback
  const result = await resolveCoords('192.0.2.1'); // TEST-NET, never in DB
  assert.equal(result.lat, 0);
  assert.equal(result.lng, 0);
});

// ---------------------------------------------------------------------------
// 2. resolveAllPeers – batch resolution
// ---------------------------------------------------------------------------

test('resolveAllPeers resolves all three canonical test peers', async () => {
  clearCache();

  const peers = [
    { id: 'tokyo',     ip: '13.230.118.60',  latencyMs: 22  },
    { id: 'frankfurt', ip: '18.185.0.1',     latencyMs: 85  },
    { id: 'virginia',  ip: '3.80.0.1',       latencyMs: 210 },
  ];

  const resolved = await resolveAllPeers(peers);

  assert.equal(resolved.length, 3);

  const tokyo     = resolved.find((p) => p.id === 'tokyo');
  const frankfurt = resolved.find((p) => p.id === 'frankfurt');
  const virginia  = resolved.find((p) => p.id === 'virginia');

  assert.ok(tokyo,     'tokyo peer missing');
  assert.ok(frankfurt, 'frankfurt peer missing');
  assert.ok(virginia,  'virginia peer missing');

  assert.match(tokyo.region,     /Tokyo/i);
  assert.match(frankfurt.region, /Frankfurt/i);
  assert.match(virginia.region,  /Virginia/i);
});

test('resolveAllPeers preserves peer metadata', async () => {
  clearCache();
  const peers = [
    { id: 'test-peer', ip: '13.230.118.60', latencyMs: 42, label: 'My Validator' },
  ];
  const [resolved] = await resolveAllPeers(peers);
  assert.equal(resolved.id,        'test-peer');
  assert.equal(resolved.label,     'My Validator');
  assert.equal(resolved.latencyMs, 42);
  assert.ok(resolved.coord);
});

test('resolveAllPeers skips GeoIP when coord is pre-supplied', async () => {
  clearCache();
  const preCoord = { lat: 51.5074, lng: -0.1278 };
  const peers = [
    { id: 'london', ip: '1.2.3.4', latencyMs: 30, coord: preCoord },
  ];
  const [resolved] = await resolveAllPeers(peers);
  assert.deepEqual(resolved.coord, preCoord);
});

// ---------------------------------------------------------------------------
// 3. Latency band classification  (spec: green < 50 ms, red > 200 ms)
// ---------------------------------------------------------------------------

test('latencyBand returns "good" for values below LATENCY_GOOD_MS', () => {
  assert.equal(latencyBand(0),                    'good');
  assert.equal(latencyBand(LATENCY_GOOD_MS - 1),  'good');
  assert.equal(latencyBand(1),                    'good');
  assert.equal(latencyBand(49),                   'good');
});

test('latencyBand returns "warn" for values in [LATENCY_GOOD_MS, LATENCY_CRITICAL_MS)', () => {
  assert.equal(latencyBand(LATENCY_GOOD_MS),          'warn');
  assert.equal(latencyBand(50),                       'warn');
  assert.equal(latencyBand(100),                      'warn');
  assert.equal(latencyBand(LATENCY_CRITICAL_MS - 1),  'warn');
  assert.equal(latencyBand(199),                      'warn');
});

test('latencyBand returns "critical" for values >= LATENCY_CRITICAL_MS', () => {
  assert.equal(latencyBand(LATENCY_CRITICAL_MS),      'critical');
  assert.equal(latencyBand(200),                      'critical');
  assert.equal(latencyBand(500),                      'critical');
  assert.equal(latencyBand(9999),                     'critical');
});

// Validate the spec constants themselves
test('LATENCY_GOOD_MS is 50 ms as specified in issue #225', () => {
  assert.equal(LATENCY_GOOD_MS,     50);
});

test('LATENCY_CRITICAL_MS is 200 ms as specified in issue #225', () => {
  assert.equal(LATENCY_CRITICAL_MS, 200);
});

// ---------------------------------------------------------------------------
// 4. Latency colour coding  (green / yellow / red)
// ---------------------------------------------------------------------------

test('latencyColor returns green for Tokyo peer (22 ms)', () => {
  const color = latencyColor(22);
  assert.equal(color, ARC_COLOR_GOOD, `Expected green (0x${ARC_COLOR_GOOD.toString(16)}), got 0x${color.toString(16)}`);
});

test('latencyColor returns yellow for Frankfurt peer (85 ms)', () => {
  const color = latencyColor(85);
  assert.equal(color, ARC_COLOR_WARN, `Expected yellow (0x${ARC_COLOR_WARN.toString(16)}), got 0x${color.toString(16)}`);
});

test('latencyColor returns red for Virginia peer at 210 ms', () => {
  const color = latencyColor(210);
  assert.equal(color, ARC_COLOR_CRITICAL, `Expected red (0x${ARC_COLOR_CRITICAL.toString(16)}), got 0x${color.toString(16)}`);
});

test('latencyColor matches band boundaries exactly', () => {
  assert.equal(latencyColor(0),   ARC_COLOR_GOOD);
  assert.equal(latencyColor(49),  ARC_COLOR_GOOD);
  assert.equal(latencyColor(50),  ARC_COLOR_WARN);
  assert.equal(latencyColor(199), ARC_COLOR_WARN);
  assert.equal(latencyColor(200), ARC_COLOR_CRITICAL);
  assert.equal(latencyColor(999), ARC_COLOR_CRITICAL);
});

test('ARC_COLOR_GOOD is the green hex value 0x39d98a', () => {
  assert.equal(ARC_COLOR_GOOD, 0x39d98a);
});

test('ARC_COLOR_WARN is the yellow hex value 0xf5b942', () => {
  assert.equal(ARC_COLOR_WARN, 0xf5b942);
});

test('ARC_COLOR_CRITICAL is the red hex value 0xf05d5e', () => {
  assert.equal(ARC_COLOR_CRITICAL, 0xf05d5e);
});

// ---------------------------------------------------------------------------
// 5. coordToVec3 – geographic → Cartesian conversion
// ---------------------------------------------------------------------------

test('coordToVec3 places the North Pole on the +Y axis', () => {
  const v = coordToVec3({ lat: 90, lng: 0 });
  assert.ok(near(v.x, 0),           `x should be ~0, got ${v.x}`);
  assert.ok(near(v.y, GLOBE_RADIUS), `y should be ~1, got ${v.y}`);
  assert.ok(near(v.z, 0),           `z should be ~0, got ${v.z}`);
});

test('coordToVec3 places the South Pole on the -Y axis', () => {
  const v = coordToVec3({ lat: -90, lng: 0 });
  assert.ok(near(v.x, 0),            `x should be ~0, got ${v.x}`);
  assert.ok(near(v.y, -GLOBE_RADIUS), `y should be ~-1, got ${v.y}`);
  assert.ok(near(v.z, 0),            `z should be ~0, got ${v.z}`);
});

test('coordToVec3 output lies on the sphere surface (‖v‖ ≈ radius)', () => {
  const cases = [
    { lat: 35.6762, lng: 139.6503 }, // Tokyo
    { lat: 50.1109, lng: 8.6821  }, // Frankfurt
    { lat: 38.9072, lng: -77.0369 }, // Virginia
    { lat: 1.3521,  lng: 103.8198 }, // Singapore
    { lat: 0,       lng: 0        }, // Origin
  ];
  for (const coord of cases) {
    const v = coordToVec3(coord);
    const len = Math.sqrt(v.x * v.x + v.y * v.y + v.z * v.z);
    assert.ok(near(len, GLOBE_RADIUS, 0.0001),
      `‖v‖ for (${coord.lat}, ${coord.lng}) expected ~${GLOBE_RADIUS}, got ${len}`);
  }
});

test('coordToVec3 respects an explicit radius argument', () => {
  const r = 2.5;
  const v = coordToVec3({ lat: 0, lng: 0 }, r);
  const len = Math.sqrt(v.x * v.x + v.y * v.y + v.z * v.z);
  assert.ok(near(len, r, 0.0001), `Expected ‖v‖ ≈ ${r}, got ${len}`);
});

// ---------------------------------------------------------------------------
// 6. buildArcPositions – Bézier arc geometry
// ---------------------------------------------------------------------------

test('buildArcPositions returns a Float32Array with (segments+1)*3 elements', () => {
  const p1 = coordToVec3({ lat: 35.6762, lng: 139.6503 });
  const p2 = coordToVec3({ lat: 50.1109, lng:   8.6821 });
  const segments = 32;
  const buf = buildArcPositions(p1, p2, segments);
  assert.ok(buf instanceof Float32Array);
  assert.equal(buf.length, (segments + 1) * 3);
});

test('buildArcPositions start and end points match the input surface points', () => {
  const p1 = coordToVec3({ lat: 35.6762, lng: 139.6503 });
  const p2 = coordToVec3({ lat: 50.1109, lng:   8.6821 });
  const buf = buildArcPositions(p1, p2, 64);

  // First vertex should equal p1
  assert.ok(near(buf[0], p1.x, 0.0001), `Start x mismatch: ${buf[0]} vs ${p1.x}`);
  assert.ok(near(buf[1], p1.y, 0.0001), `Start y mismatch: ${buf[1]} vs ${p1.y}`);
  assert.ok(near(buf[2], p1.z, 0.0001), `Start z mismatch: ${buf[2]} vs ${p1.z}`);

  // Last vertex should equal p2
  const last = buf.length - 3;
  assert.ok(near(buf[last],     p2.x, 0.0001), `End x mismatch: ${buf[last]} vs ${p2.x}`);
  assert.ok(near(buf[last + 1], p2.y, 0.0001), `End y mismatch: ${buf[last + 1]} vs ${p2.y}`);
  assert.ok(near(buf[last + 2], p2.z, 0.0001), `End z mismatch: ${buf[last + 2]} vs ${p2.z}`);
});

test('buildArcPositions control point lifts the midpoint above the globe surface', () => {
  const p1 = coordToVec3({ lat:  35.6762, lng:  139.6503 }); // Tokyo
  const p2 = coordToVec3({ lat: -33.8688, lng:  151.2093 }); // Sydney (far)
  const buf = buildArcPositions(p1, p2, 64);

  // Find the vertex closest to the midpoint of the arc (t ≈ 0.5, index 32)
  const midIdx = 32 * 3;
  const mx = buf[midIdx];
  const my = buf[midIdx + 1];
  const mz = buf[midIdx + 2];
  const midDist = Math.sqrt(mx * mx + my * my + mz * mz);

  // The midpoint of the arc should be above the globe surface
  assert.ok(midDist > GLOBE_RADIUS,
    `Arc midpoint (‖v‖=${midDist.toFixed(4)}) should be > ${GLOBE_RADIUS} (globe radius)`);
});

// ---------------------------------------------------------------------------
// 7. Full pipeline – mock peers through to arc colour (integration smoke test)
// ---------------------------------------------------------------------------

test('full pipeline: Tokyo 22 ms → good, Frankfurt 85 ms → warn, Virginia 210 ms → critical', async () => {
  clearCache();

  const peers = [
    { id: 'tokyo',     ip: '13.230.118.60', latencyMs: 22  },
    { id: 'frankfurt', ip: '18.185.0.1',    latencyMs: 85  },
    { id: 'virginia',  ip: '3.80.0.1',      latencyMs: 210 },
  ];

  const resolved = await resolveAllPeers(peers);
  const arcs = resolved.map((p) => ({
    peerId:    p.id,
    from:      { lat: 38.9072, lng: -77.0369 }, // host in Virginia
    to:        p.coord,
    latencyMs: p.latencyMs,
    color:     latencyColor(p.latencyMs),
    band:      latencyBand(p.latencyMs),
  }));

  const tokyoArc     = arcs.find((a) => a.peerId === 'tokyo');
  const frankfurtArc = arcs.find((a) => a.peerId === 'frankfurt');
  const virginiaArc  = arcs.find((a) => a.peerId === 'virginia');

  assert.equal(tokyoArc.band,     'good');
  assert.equal(tokyoArc.color,    ARC_COLOR_GOOD);

  assert.equal(frankfurtArc.band,  'warn');
  assert.equal(frankfurtArc.color, ARC_COLOR_WARN);

  assert.equal(virginiaArc.band,   'critical');
  assert.equal(virginiaArc.color,  ARC_COLOR_CRITICAL);

  // Verify spatial rendering: all arc destination coords lie on the sphere surface
  for (const arc of arcs) {
    const v = arc.to;
    // coordToVec3 should round-trip correctly when called later by the globe
    const vec = coordToVec3(v);
    const len = Math.sqrt(vec.x ** 2 + vec.y ** 2 + vec.z ** 2);
    assert.ok(near(len, GLOBE_RADIUS, 0.001),
      `${arc.peerId}: arc endpoint not on globe surface (‖v‖=${len})`);
  }
});
