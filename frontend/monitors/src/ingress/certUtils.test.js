/**
 * certUtils.test.js
 *
 * Unit tests for the Ingress TLS certificate utility module.
 * Covers:
 *  - daysRemaining(): boundary values, negative (expired), NaN for bad input
 *  - statusFromDays(): all four buckets including exact boundary values
 *  - colorFromStatus(): CSS token correctness for each status
 *  - expiryLabel(): human-readable output at key day values
 *  - deriveCertRow(): full CertRow derivation including sorted status color
 *  - sortCertRows(): near-expiry-first ordering, column sorting, direction toggle
 *  - filterCertRows(): status, namespace, and search text filters
 *  - summaryCounts(): bucket counting
 *
 * Run with:  node --test src/ingress/certUtils.test.js
 */

import test from 'node:test';
import assert from 'node:assert/strict';

import {
  CRITICAL_THRESHOLD_DAYS,
  WARNING_THRESHOLD_DAYS,
  colorFromStatus,
  deriveCertRow,
  daysRemaining,
  expiryLabel,
  filterCertRows,
  sortCertRows,
  statusFromDays,
  summaryCounts,
  uniqueNamespaces,
} from './certUtils.js';

// ─── Helpers ──────────────────────────────────────────────────────────────────

const ONE_DAY_MS = 24 * 60 * 60 * 1000;

/** Build an ISO expiry string `d` days from now (fractional days allowed). */
function futureISO(days) {
  return new Date(Date.now() + days * ONE_DAY_MS).toISOString();
}

/** Minimal IngressCertRecord stub. */
function makeRecord(overrides = {}) {
  return {
    id: 'test-cert',
    namespace: 'default',
    ingressName: 'test-ingress',
    host: 'test.example.com',
    secretName: 'test-tls',
    issuer: 'Let\'s Encrypt Authority X3',
    issuerOrg: 'Let\'s Encrypt',
    subject: 'CN=test.example.com',
    sans: ['test.example.com'],
    notBefore: futureISO(-90),
    expiresAt: futureISO(60),
    certManagerManaged: true,
    renewalTriggered: false,
    ...overrides,
  };
}

// ─── daysRemaining ────────────────────────────────────────────────────────────

test('daysRemaining returns a positive integer for a future date', () => {
  const days = daysRemaining(futureISO(40));
  assert.ok(days >= 39 && days <= 40, `expected ~40, got ${days}`);
});

test('daysRemaining returns 0 for a date exactly at midnight today', () => {
  // A date 12 hours from now should round down to 0 days.
  const days = daysRemaining(futureISO(0.4));
  assert.equal(days, 0);
});

test('daysRemaining returns a negative integer for a past date', () => {
  const days = daysRemaining(futureISO(-10));
  assert.ok(days >= -11 && days <= -9, `expected around -10, got ${days}`);
});

test('daysRemaining returns NaN for an invalid date string', () => {
  assert.ok(Number.isNaN(daysRemaining('not-a-date')));
});

test('daysRemaining accepts a Date object', () => {
  const days = daysRemaining(new Date(Date.now() + 5 * ONE_DAY_MS));
  assert.ok(days >= 4 && days <= 5);
});

test('daysRemaining accepts epoch milliseconds', () => {
  const days = daysRemaining(Date.now() + 7 * ONE_DAY_MS);
  assert.ok(days >= 6 && days <= 7);
});

// ─── statusFromDays ───────────────────────────────────────────────────────────

test('statusFromDays: negative days → expired', () => {
  assert.equal(statusFromDays(-1), 'expired');
  assert.equal(statusFromDays(-100), 'expired');
});

test('statusFromDays: 0 days → critical (will expire today)', () => {
  assert.equal(statusFromDays(0), 'critical');
});

test('statusFromDays: CRITICAL threshold boundary', () => {
  // one below threshold → critical
  assert.equal(statusFromDays(CRITICAL_THRESHOLD_DAYS - 1), 'critical');
  // exactly at threshold → warning
  assert.equal(statusFromDays(CRITICAL_THRESHOLD_DAYS), 'warning');
});

test('statusFromDays: WARNING threshold boundary', () => {
  // one below warning → warning
  assert.equal(statusFromDays(WARNING_THRESHOLD_DAYS - 1), 'warning');
  // exactly at warning → healthy
  assert.equal(statusFromDays(WARNING_THRESHOLD_DAYS), 'healthy');
});

test('statusFromDays: large positive value → healthy', () => {
  assert.equal(statusFromDays(365), 'healthy');
});

test('statusFromDays: NaN → expired (safe fallback)', () => {
  assert.equal(statusFromDays(NaN), 'expired');
});

// ─── colorFromStatus ─────────────────────────────────────────────────────────

test('colorFromStatus maps each status to the correct colour token', () => {
  assert.equal(colorFromStatus('healthy'),  '#39d98a');
  assert.equal(colorFromStatus('warning'),  '#f5b942');
  assert.equal(colorFromStatus('critical'), '#f05d5e');
  assert.equal(colorFromStatus('expired'),  '#f05d5e');
});

test('colorFromStatus returns muted grey for unknown status', () => {
  assert.equal(colorFromStatus('unknown-bucket'), '#7f92a3');
});

// ─── expiryLabel ─────────────────────────────────────────────────────────────

test('expiryLabel: already expired shows days ago', () => {
  const label = expiryLabel(-5, futureISO(-5));
  assert.ok(label.includes('5'), `Label should mention 5: ${label}`);
  assert.ok(label.toLowerCase().includes('expired'), `Label should say expired: ${label}`);
});

test('expiryLabel: expires today shows "Expires today"', () => {
  assert.equal(expiryLabel(0, new Date().toISOString()), 'Expires today');
});

test('expiryLabel: expires in 1 day shows "Expires tomorrow"', () => {
  assert.equal(expiryLabel(1, futureISO(1)), 'Expires tomorrow');
});

test('expiryLabel: small positive days shows "Expires in N days"', () => {
  const label = expiryLabel(10, futureISO(10));
  assert.match(label, /Expires in 10 days/);
});

test('expiryLabel: healthy cert (≥30d) shows a formatted date string', () => {
  const label = expiryLabel(60, futureISO(60));
  // Should not say "Expires in", should be a date
  assert.ok(!label.startsWith('Expires in'), `Healthy label should be a date: ${label}`);
  assert.ok(label.length > 4, `Label too short: ${label}`);
});

test('expiryLabel: NaN input returns "Unknown"', () => {
  assert.equal(expiryLabel(NaN, 'bad'), 'Unknown');
});

// ─── deriveCertRow ────────────────────────────────────────────────────────────

test('deriveCertRow computes all fields from a record with a future date', () => {
  const record = makeRecord({ expiresAt: futureISO(60) });
  const row = deriveCertRow(record);
  assert.equal(row.status, 'healthy');
  assert.equal(row.statusColor, '#39d98a');
  assert.ok(row.daysRemaining >= 59 && row.daysRemaining <= 60);
  assert.ok(typeof row.expiryLabel === 'string');
  assert.equal(row.sansDisplay, 'test.example.com');
});

test('deriveCertRow marks expired certs correctly', () => {
  const row = deriveCertRow(makeRecord({ expiresAt: futureISO(-1) }));
  assert.equal(row.status, 'expired');
  assert.equal(row.statusColor, '#f05d5e');
  assert.ok(row.daysRemaining < 0);
});

test('deriveCertRow marks critical certs correctly', () => {
  const row = deriveCertRow(makeRecord({ expiresAt: futureISO(3) }));
  assert.equal(row.status, 'critical');
  assert.equal(row.statusColor, '#f05d5e');
});

test('deriveCertRow marks warning certs correctly', () => {
  const row = deriveCertRow(makeRecord({ expiresAt: futureISO(15) }));
  assert.equal(row.status, 'warning');
  assert.equal(row.statusColor, '#f5b942');
});

test('deriveCertRow joins multiple SANs with comma-space', () => {
  const record = makeRecord({ sans: ['a.com', 'b.com', 'c.com'] });
  const row = deriveCertRow(record);
  assert.equal(row.sansDisplay, 'a.com, b.com, c.com');
});

test('deriveCertRow preserves all original record fields', () => {
  const record = makeRecord();
  const row = deriveCertRow(record);
  assert.equal(row.host, record.host);
  assert.equal(row.namespace, record.namespace);
  assert.equal(row.certManagerManaged, record.certManagerManaged);
});

// ─── sortCertRows ─────────────────────────────────────────────────────────────

function makeRows() {
  return [
    deriveCertRow(makeRecord({ id: 'healthy',  expiresAt: futureISO(60),  namespace: 'prod',    host: 'z.example.com' })),
    deriveCertRow(makeRecord({ id: 'warning',  expiresAt: futureISO(20),  namespace: 'staging', host: 'a.example.com' })),
    deriveCertRow(makeRecord({ id: 'critical', expiresAt: futureISO(3),   namespace: 'prod',    host: 'm.example.com' })),
    deriveCertRow(makeRecord({ id: 'expired',  expiresAt: futureISO(-5),  namespace: 'legacy',  host: 'b.example.com' })),
  ];
}

test('sortCertRows by daysRemaining asc places expired/critical first', () => {
  const sorted = sortCertRows(makeRows(), 'daysRemaining', 'asc');
  assert.equal(sorted[0].id, 'expired');
  assert.equal(sorted[1].id, 'critical');
  assert.equal(sorted[2].id, 'warning');
  assert.equal(sorted[3].id, 'healthy');
});

test('sortCertRows by daysRemaining desc places healthy first', () => {
  const sorted = sortCertRows(makeRows(), 'daysRemaining', 'desc');
  assert.equal(sorted[0].id, 'healthy');
  assert.equal(sorted[3].id, 'expired');
});

test('sortCertRows by host asc sorts alphabetically', () => {
  const sorted = sortCertRows(makeRows(), 'host', 'asc');
  assert.equal(sorted[0].host, 'a.example.com');
  assert.equal(sorted[3].host, 'z.example.com');
});

test('sortCertRows by host desc reverses alphabetical order', () => {
  const sorted = sortCertRows(makeRows(), 'host', 'desc');
  assert.equal(sorted[0].host, 'z.example.com');
});

test('sortCertRows by namespace asc groups namespaces alphabetically', () => {
  const sorted = sortCertRows(makeRows(), 'namespace', 'asc');
  assert.equal(sorted[0].namespace, 'legacy');
});

test('sortCertRows does not mutate the original array', () => {
  const rows = makeRows();
  const original = rows.map((r) => r.id);
  sortCertRows(rows, 'daysRemaining', 'asc');
  assert.deepEqual(rows.map((r) => r.id), original);
});

// ─── filterCertRows ───────────────────────────────────────────────────────────

test('filterCertRows status=all returns every row', () => {
  const rows = makeRows();
  assert.equal(filterCertRows(rows, { status: 'all' }).length, 4);
});

test('filterCertRows status=expired returns only expired rows', () => {
  const result = filterCertRows(makeRows(), { status: 'expired' });
  assert.equal(result.length, 1);
  assert.equal(result[0].id, 'expired');
});

test('filterCertRows status=critical returns only critical rows', () => {
  const result = filterCertRows(makeRows(), { status: 'critical' });
  assert.equal(result.length, 1);
  assert.equal(result[0].id, 'critical');
});

test('filterCertRows namespace filter narrows to matching namespace', () => {
  const result = filterCertRows(makeRows(), { status: 'all', namespace: 'prod' });
  assert.equal(result.length, 2);
  result.forEach((r) => assert.equal(r.namespace, 'prod'));
});

test('filterCertRows search matches host substring case-insensitively', () => {
  const result = filterCertRows(makeRows(), { status: 'all', search: 'A.EXAMPLE' });
  assert.equal(result.length, 1);
  assert.equal(result[0].host, 'a.example.com');
});

test('filterCertRows combined status + namespace returns empty when no match', () => {
  const result = filterCertRows(makeRows(), { status: 'expired', namespace: 'prod' });
  assert.equal(result.length, 0);
});

// ─── summaryCounts ────────────────────────────────────────────────────────────

test('summaryCounts returns correct totals for all buckets', () => {
  const counts = summaryCounts(makeRows());
  assert.equal(counts.total,    4);
  assert.equal(counts.expired,  1);
  assert.equal(counts.critical, 1);
  assert.equal(counts.warning,  1);
  assert.equal(counts.healthy,  1);
});

test('summaryCounts returns zeros for empty array', () => {
  const counts = summaryCounts([]);
  assert.equal(counts.total, 0);
  assert.equal(counts.expired, 0);
});

// ─── uniqueNamespaces ─────────────────────────────────────────────────────────

test('uniqueNamespaces returns sorted unique namespace list', () => {
  const ns = uniqueNamespaces(makeRows());
  assert.deepEqual(ns, ['legacy', 'prod', 'staging']);
});

test('uniqueNamespaces returns empty array for empty input', () => {
  assert.deepEqual(uniqueNamespaces([]), []);
});

// ─── Integration: near-expiry rows sorted to top ──────────────────────────────

test('near-expiry rows always appear before healthy rows after sort', () => {
  const mixed = [
    deriveCertRow(makeRecord({ id: 'h1', expiresAt: futureISO(200) })),
    deriveCertRow(makeRecord({ id: 'c1', expiresAt: futureISO(2) })),
    deriveCertRow(makeRecord({ id: 'h2', expiresAt: futureISO(100) })),
    deriveCertRow(makeRecord({ id: 'w1', expiresAt: futureISO(25) })),
    deriveCertRow(makeRecord({ id: 'e1', expiresAt: futureISO(-1) })),
  ];
  const sorted = sortCertRows(mixed, 'daysRemaining', 'asc');
  const ids = sorted.map((r) => r.id);
  // expired before critical before warning before healthy
  assert.ok(ids.indexOf('e1') < ids.indexOf('c1'), 'expired should precede critical');
  assert.ok(ids.indexOf('c1') < ids.indexOf('w1'), 'critical should precede warning');
  assert.ok(ids.indexOf('w1') < ids.indexOf('h1'), 'warning should precede healthy');
  assert.ok(ids.indexOf('w1') < ids.indexOf('h2'), 'warning should precede healthy');
});
