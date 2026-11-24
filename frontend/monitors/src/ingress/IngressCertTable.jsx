import { useMemo, useState } from 'react';
import { filterCertRows, sortCertRows, uniqueNamespaces } from './certUtils.js';
import CertificateStatusBadge from './CertificateStatusBadge.jsx';
import ForceRenewalButton from './ForceRenewalButton.jsx';

const PAGE_SIZE_OPTIONS = [10, 25, 50, 100];

/**
 * IngressCertTable
 *
 * Full-featured table component for listing Ingress TLS certificates.
 * Features:
 *  - Client-side column sorting (click header to toggle asc/desc)
 *  - Status and namespace filter dropdowns, free-text search
 *  - Pagination (configurable page sizes; efficient for 50+ routes)
 *  - Colour-coded rows using CSS custom properties driven by status
 *  - Per-row ForceRenewalButton
 *
 * Props:
 *   rows      – CertRow[] (pre-derived via deriveCertRow)
 *   onRenew   – async function(certRow) → void  (forwarded to ForceRenewalButton)
 */
export default function IngressCertTable({ rows = [], onRenew }) {
  const [sortKey, setSortKey] = useState('daysRemaining');
  const [sortDir, setSortDir] = useState('asc');
  const [statusFilter, setStatusFilter] = useState('all');
  const [nsFilter, setNsFilter] = useState('');
  const [search, setSearch] = useState('');
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(25);
  const [expandedId, setExpandedId] = useState(null);

  const namespaces = useMemo(() => uniqueNamespaces(rows), [rows]);

  const filtered = useMemo(
    () => filterCertRows(rows, { status: statusFilter, namespace: nsFilter, search }),
    [rows, statusFilter, nsFilter, search],
  );

  const sorted = useMemo(() => sortCertRows(filtered, sortKey, sortDir), [filtered, sortKey, sortDir]);

  const totalPages = Math.max(1, Math.ceil(sorted.length / pageSize));
  const safePage = Math.min(page, totalPages);
  const pageRows = sorted.slice((safePage - 1) * pageSize, safePage * pageSize);

  function handleSort(key) {
    if (key === sortKey) {
      setSortDir((d) => (d === 'asc' ? 'desc' : 'asc'));
    } else {
      setSortKey(key);
      setSortDir('asc');
    }
    setPage(1);
  }

  function handleFilterChange(setter) {
    return (e) => { setter(e.target.value); setPage(1); };
  }

  function toggleExpand(id) {
    setExpandedId((prev) => (prev === id ? null : id));
  }

  const sortIndicator = (key) =>
    sortKey === key ? (sortDir === 'asc' ? ' ↑' : ' ↓') : '';

  return (
    <div className="ict-wrapper">
      {/* ── Toolbar ── */}
      <div className="ict-toolbar" role="toolbar" aria-label="Certificate table controls">
        <label className="ict-filter-label">
          <span>Status</span>
          <select value={statusFilter} onChange={handleFilterChange(setStatusFilter)}>
            <option value="all">All</option>
            <option value="expired">Expired</option>
            <option value="critical">Critical (&lt;7 d)</option>
            <option value="warning">Warning (&lt;30 d)</option>
            <option value="healthy">Healthy</option>
          </select>
        </label>

        <label className="ict-filter-label">
          <span>Namespace</span>
          <select value={nsFilter} onChange={handleFilterChange(setNsFilter)}>
            <option value="">All namespaces</option>
            {namespaces.map((ns) => (
              <option key={ns} value={ns}>{ns}</option>
            ))}
          </select>
        </label>

        <label className="ict-filter-label ict-filter-label--search">
          <span>Search</span>
          <input
            type="search"
            className="ict-search"
            placeholder="Host or ingress name…"
            value={search}
            onChange={handleFilterChange(setSearch)}
            aria-label="Search certificates by host or ingress name"
          />
        </label>

        <span className="ict-count" aria-live="polite">
          {filtered.length} / {rows.length} certificate{rows.length !== 1 ? 's' : ''}
        </span>
      </div>

      {/* ── Table ── */}
      <div className="ict-scroll-x" role="region" aria-label="Certificate list" tabIndex="0">
        <table className="ict-table" aria-rowcount={filtered.length}>
          <thead>
            <tr>
              <th scope="col" className="ict-col-expand" aria-label="Expand row" />
              <SortTh col="namespace"     label="Namespace"    current={sortKey} dir={sortDir} onSort={handleSort} sortIndicator={sortIndicator} />
              <SortTh col="host"          label="Host"         current={sortKey} dir={sortDir} onSort={handleSort} sortIndicator={sortIndicator} />
              <SortTh col="issuerOrg"     label="Issuer"       current={sortKey} dir={sortDir} onSort={handleSort} sortIndicator={sortIndicator} />
              <th scope="col" className="ict-col-sans">SANs</th>
              <SortTh col="expiresAt"     label="Expiry Date"  current={sortKey} dir={sortDir} onSort={handleSort} sortIndicator={sortIndicator} />
              <SortTh col="daysRemaining" label="Days Left"    current={sortKey} dir={sortDir} onSort={handleSort} sortIndicator={sortIndicator} />
              <SortTh col="status"        label="Status"       current={sortKey} dir={sortDir} onSort={handleSort} sortIndicator={sortIndicator} />
              <th scope="col" className="ict-col-action">Action</th>
            </tr>
          </thead>
          <tbody>
            {pageRows.length === 0 ? (
              <tr>
                <td colSpan="9" className="ict-empty">No certificates match the current filters.</td>
              </tr>
            ) : (
              pageRows.map((row) => (
                <CertRow
                  key={row.id}
                  row={row}
                  expanded={expandedId === row.id}
                  onToggle={() => toggleExpand(row.id)}
                  onRenew={onRenew}
                />
              ))
            )}
          </tbody>
        </table>
      </div>

      {/* ── Pagination ── */}
      <div className="ict-pagination" aria-label="Table pagination">
        <label className="ict-filter-label ict-filter-label--inline">
          <span>Per page</span>
          <select
            value={pageSize}
            onChange={(e) => { setPageSize(Number(e.target.value)); setPage(1); }}
          >
            {PAGE_SIZE_OPTIONS.map((n) => (
              <option key={n} value={n}>{n}</option>
            ))}
          </select>
        </label>

        <div className="ict-page-nav" role="navigation" aria-label="Page navigation">
          <button
            type="button"
            className="ict-page-btn"
            disabled={safePage === 1}
            onClick={() => setPage(1)}
            aria-label="First page"
          >
            ««
          </button>
          <button
            type="button"
            className="ict-page-btn"
            disabled={safePage === 1}
            onClick={() => setPage((p) => p - 1)}
            aria-label="Previous page"
          >
            ‹
          </button>
          <span className="ict-page-info">
            Page <strong>{safePage}</strong> of <strong>{totalPages}</strong>
          </span>
          <button
            type="button"
            className="ict-page-btn"
            disabled={safePage === totalPages}
            onClick={() => setPage((p) => p + 1)}
            aria-label="Next page"
          >
            ›
          </button>
          <button
            type="button"
            className="ict-page-btn"
            disabled={safePage === totalPages}
            onClick={() => setPage(totalPages)}
            aria-label="Last page"
          >
            »»
          </button>
        </div>
      </div>
    </div>
  );
}

// ── Sub-components ────────────────────────────────────────────────────────────

function SortTh({ col, label, current, dir, onSort, sortIndicator }) {
  const active = col === current;
  return (
    <th
      scope="col"
      className={`ict-col-${col} ict-sortable ${active ? 'ict-sortable--active' : ''}`}
      aria-sort={active ? (dir === 'asc' ? 'ascending' : 'descending') : 'none'}
    >
      <button type="button" onClick={() => onSort(col)}>
        {label}{sortIndicator(col)}
      </button>
    </th>
  );
}

function CertRow({ row, expanded, onToggle, onRenew }) {
  const rowClass = `ict-row ict-row--${row.status}`;

  return (
    <>
      <tr
        className={rowClass}
        style={{ '--row-accent': row.statusColor }}
        aria-expanded={expanded}
      >
        <td className="ict-col-expand">
          <button
            type="button"
            className="ict-expand-btn"
            onClick={onToggle}
            aria-label={`${expanded ? 'Collapse' : 'Expand'} details for ${row.host}`}
            aria-expanded={expanded}
          >
            {expanded ? '▾' : '▸'}
          </button>
        </td>
        <td className="ict-cell-ns">
          <code className="ict-ns-pill">{row.namespace}</code>
        </td>
        <td className="ict-cell-host">
          <span className="ict-host">{row.host}</span>
          <span className="ict-ingress-name">{row.ingressName}</span>
        </td>
        <td className="ict-cell-issuer">{row.issuerOrg}</td>
        <td className="ict-cell-sans">
          <SansList sans={row.sans} />
        </td>
        <td className="ict-cell-expiry">
          <time dateTime={row.expiresAt} className="ict-expiry-date">
            {new Date(row.expiresAt).toLocaleDateString('en-GB', {
              day: '2-digit', month: 'short', year: 'numeric',
            })}
          </time>
          <span className="ict-expiry-label muted">{row.expiryLabel}</span>
        </td>
        <td className="ict-cell-days">
          <span
            className="ict-days-badge"
            style={{ color: row.statusColor }}
            aria-label={`${row.daysRemaining} days remaining`}
          >
            {row.daysRemaining < 0 ? row.daysRemaining : `+${row.daysRemaining}`}
          </span>
        </td>
        <td className="ict-cell-status">
          <CertificateStatusBadge status={row.status} days={row.daysRemaining} />
        </td>
        <td className="ict-cell-action">
          <ForceRenewalButton certRow={row} onRenew={onRenew} />
        </td>
      </tr>

      {expanded && (
        <tr className={`ict-row ict-row--expanded ict-row--${row.status}`} aria-label={`Expanded details for ${row.host}`}>
          <td colSpan="9" className="ict-expanded-cell">
            <dl className="ict-detail-list">
              <DetailItem label="Secret name"    value={row.secretName} mono />
              <DetailItem label="Subject DN"     value={row.subject} mono />
              <DetailItem label="Full issuer DN" value={row.issuer} mono />
              <DetailItem label="Not before"     value={new Date(row.notBefore).toUTCString()} />
              <DetailItem label="Expires at"     value={new Date(row.expiresAt).toUTCString()} />
              <DetailItem label="All SANs"       value={row.sansDisplay} mono />
              <DetailItem label="cert-manager"   value={row.certManagerManaged ? 'Managed' : 'Not managed'} />
            </dl>
          </td>
        </tr>
      )}
    </>
  );
}

function SansList({ sans }) {
  if (!Array.isArray(sans) || sans.length === 0) return <span className="muted">—</span>;
  const visible = sans.slice(0, 2);
  const overflow = sans.length - visible.length;
  return (
    <span className="ict-sans-list">
      {visible.map((san, i) => (
        <code key={i} className="ict-san">{san}</code>
      ))}
      {overflow > 0 && (
        <span className="ict-sans-overflow muted" title={sans.join('\n')}>+{overflow} more</span>
      )}
    </span>
  );
}

function DetailItem({ label, value, mono }) {
  return (
    <div className="ict-detail-row">
      <dt>{label}</dt>
      <dd>{mono ? <code>{value}</code> : value}</dd>
    </div>
  );
}
