/**
 * certUtils.js – Core data utilities for the Ingress TLS Certificate dashboard.
 *
 * Provides:
 *  - JSDoc type definitions for IngressCertRecord and derived CertRow
 *  - daysRemaining(expiresAt)      → signed integer days until expiry
 *  - statusFromDays(days)          → 'expired' | 'critical' | 'warning' | 'healthy'
 *  - colorFromStatus(status)       → CSS colour token
 *  - deriveCertRow(record)         → full CertRow with computed fields
 *  - sortCertRows(rows, key, dir)  → sorted copy (near-expiry first by default)
 *  - filterCertRows(rows, filter)  → filtered copy by status bucket or namespace
 */

const ONE_DAY_MS = 24 * 60 * 60 * 1000;

// ─── JSDoc types ─────────────────────────────────────────────────────────────

/**
 * Raw record as returned by the backend (or mock fixtures).
 *
 * @typedef {object} IngressCertRecord
 * @property {string}   id                 Unique identifier (e.g. "<namespace>/<secret>")
 * @property {string}   namespace          Kubernetes namespace
 * @property {string}   ingressName        Ingress object name
 * @property {string}   host               Primary hostname / SNI
 * @property {string}   secretName         TLS secret name
 * @property {string}   issuer             Certificate Issuer DN
 * @property {string}   issuerOrg          Issuer organisation (display label)
 * @property {string}   subject            Certificate Subject DN
 * @property {string[]} sans               Subject Alternative Names
 * @property {string}   notBefore          ISO-8601 not-before timestamp
 * @property {string}   expiresAt          ISO-8601 expiry timestamp
 * @property {boolean}  certManagerManaged Whether cert-manager owns this cert
 * @property {boolean}  renewalTriggered   Whether a force-renewal is in progress
 */

/**
 * Derived view-model row ready for rendering.
 *
 * @typedef {IngressCertRecord & {
 *   daysRemaining: number,
 *   status: 'expired'|'critical'|'warning'|'healthy',
 *   statusColor: string,
 *   expiryLabel: string,
 *   sansDisplay: string,
 * }} CertRow
 */

/**
 * Column sort key used by sortCertRows.
 *
 * @typedef {'daysRemaining'|'host'|'namespace'|'issuerOrg'|'expiresAt'|'status'} SortKey
 */

/**
 * Active filter state for the table.
 *
 * @typedef {object} FilterState
 * @property {'all'|'expired'|'critical'|'warning'|'healthy'} status
 * @property {string} namespace  Empty string means "all namespaces"
 * @property {string} search     Free-text search against host / ingressName
 */

// ─── Status thresholds ───────────────────────────────────────────────────────

/** Days threshold below which a cert enters "warning" status. */
export const WARNING_THRESHOLD_DAYS = 30;
/** Days threshold below which a cert enters "critical" status. */
export const CRITICAL_THRESHOLD_DAYS = 7;

// ─── Core calculations ───────────────────────────────────────────────────────

/**
 * Compute the number of whole days remaining before `expiresAt`.
 * Returns a negative number for already-expired certs.
 *
 * @param {string|number|Date} expiresAt  ISO-8601 string, epoch ms, or Date
 * @returns {number}
 */
export function daysRemaining(expiresAt) {
  const expiry = new Date(expiresAt).getTime();
  if (!Number.isFinite(expiry)) return NaN;
  return Math.floor((expiry - Date.now()) / ONE_DAY_MS);
}

/**
 * Derive a status bucket from a signed days-remaining value.
 *
 * | Range                       | Status     | Default colour |
 * |-----------------------------|------------|----------------|
 * | days < 0                    | 'expired'  | red            |
 * | 0 ≤ days < CRITICAL (7)     | 'critical' | red            |
 * | CRITICAL ≤ days < WARNING   | 'warning'  | amber          |
 * | days ≥ WARNING (30)         | 'healthy'  | green          |
 *
 * @param {number} days  Value from daysRemaining()
 * @returns {'expired'|'critical'|'warning'|'healthy'}
 */
export function statusFromDays(days) {
  if (!Number.isFinite(days) || days < 0) return 'expired';
  if (days < CRITICAL_THRESHOLD_DAYS) return 'critical';
  if (days < WARNING_THRESHOLD_DAYS) return 'warning';
  return 'healthy';
}

/**
 * Map a status bucket to the project's canonical CSS colour token.
 *
 * @param {'expired'|'critical'|'warning'|'healthy'} status
 * @returns {string}
 */
export function colorFromStatus(status) {
  switch (status) {
    case 'healthy':  return '#39d98a'; // green
    case 'warning':  return '#f5b942'; // amber
    case 'critical': return '#f05d5e'; // red
    case 'expired':  return '#f05d5e'; // red
    default:         return '#7f92a3'; // muted fallback
  }
}

/**
 * Human-readable expiry label: "Expired N days ago", "Expires today",
 * "Expires in N day(s)", or the formatted date for healthy certs.
 *
 * @param {number} days
 * @param {string} isoDate
 * @returns {string}
 */
export function expiryLabel(days, isoDate) {
  if (!Number.isFinite(days)) return 'Unknown';
  if (days < 0)  return `Expired ${Math.abs(days)} day${Math.abs(days) === 1 ? '' : 's'} ago`;
  if (days === 0) return 'Expires today';
  if (days === 1) return 'Expires tomorrow';
  if (days < WARNING_THRESHOLD_DAYS) return `Expires in ${days} days`;
  // Healthy: show the absolute date
  return new Date(isoDate).toLocaleDateString('en-GB', {
    day: '2-digit',
    month: 'short',
    year: 'numeric',
  });
}

// ─── Derived row ─────────────────────────────────────────────────────────────

/**
 * Enrich a raw IngressCertRecord into a fully computed CertRow suitable for
 * table rendering.
 *
 * @param {IngressCertRecord} record
 * @returns {CertRow}
 */
export function deriveCertRow(record) {
  const days = daysRemaining(record.expiresAt);
  const status = statusFromDays(days);
  return {
    ...record,
    daysRemaining: days,
    status,
    statusColor: colorFromStatus(status),
    expiryLabel: expiryLabel(days, record.expiresAt),
    sansDisplay: Array.isArray(record.sans) ? record.sans.join(', ') : '',
  };
}

// ─── Status sort weight ──────────────────────────────────────────────────────

const STATUS_WEIGHT = { expired: 0, critical: 1, warning: 2, healthy: 3 };

/**
 * Sort comparator placing most-urgent entries first.
 * Primary: status severity (expired → critical → warning → healthy).
 * Secondary: daysRemaining ascending (fewer days ≡ more urgent).
 *
 * @param {CertRow} a
 * @param {CertRow} b
 * @returns {number}
 */
function urgencyComparator(a, b) {
  const ws = (STATUS_WEIGHT[a.status] ?? 4) - (STATUS_WEIGHT[b.status] ?? 4);
  if (ws !== 0) return ws;
  return a.daysRemaining - b.daysRemaining;
}

/**
 * Sort a copy of `rows` by a given column key and direction.
 * When `key` is 'daysRemaining' and `dir` is 'asc', uses the urgency comparator
 * so near-expiry certs always float to the top.
 *
 * @param {CertRow[]} rows
 * @param {SortKey}   key
 * @param {'asc'|'desc'} dir
 * @returns {CertRow[]}
 */
export function sortCertRows(rows, key = 'daysRemaining', dir = 'asc') {
  const copy = [...rows];
  copy.sort((a, b) => {
    let cmp;
    if (key === 'daysRemaining' || key === 'status') {
      cmp = urgencyComparator(a, b);
    } else {
      const av = a[key] ?? '';
      const bv = b[key] ?? '';
      cmp = typeof av === 'number' ? av - bv : String(av).localeCompare(String(bv));
    }
    return dir === 'asc' ? cmp : -cmp;
  });
  return copy;
}

/**
 * Filter `rows` according to a FilterState object.
 *
 * @param {CertRow[]}   rows
 * @param {FilterState} filter
 * @returns {CertRow[]}
 */
export function filterCertRows(rows, filter) {
  const { status = 'all', namespace = '', search = '' } = filter;
  const q = search.toLowerCase().trim();
  return rows.filter((row) => {
    if (status !== 'all' && row.status !== status) return false;
    if (namespace && row.namespace !== namespace) return false;
    if (q && !(row.host.toLowerCase().includes(q) ||
               row.ingressName.toLowerCase().includes(q) ||
               row.namespace.toLowerCase().includes(q))) return false;
    return true;
  });
}

/**
 * Derive summary counts across all cert rows.
 *
 * @param {CertRow[]} rows
 * @returns {{ total: number, expired: number, critical: number, warning: number, healthy: number }}
 */
export function summaryCounts(rows) {
  return rows.reduce(
    (acc, row) => {
      acc.total++;
      acc[row.status] = (acc[row.status] ?? 0) + 1;
      return acc;
    },
    { total: 0, expired: 0, critical: 0, warning: 0, healthy: 0 },
  );
}

/**
 * Extract sorted unique namespaces from a list of rows.
 *
 * @param {CertRow[]} rows
 * @returns {string[]}
 */
export function uniqueNamespaces(rows) {
  return [...new Set(rows.map((r) => r.namespace))].sort();
}
