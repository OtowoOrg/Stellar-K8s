import { useMemo, useState } from 'react';
import { deriveCertRow, sortCertRows, summaryCounts } from './certUtils.js';
import IngressCertTable from './IngressCertTable.jsx';

/**
 * IngressCertDashboard
 *
 * Top-level page component for the TLS Certificate Expiration Monitor.
 *
 * Responsibilities:
 *  1. Accept raw IngressCertRecord[] from the parent/router (or use mock data
 *     via the `useMock` prop for local development).
 *  2. Derive CertRow[] once using deriveCertRow.
 *  3. Render summary stat cards (total, expired, critical, warning, healthy).
 *  4. Render the <IngressCertTable> with sorting defaulting near-expiry to top.
 *  5. Forward the `onRenew` handler down to the table/buttons.
 *
 * Props:
 *   records    – IngressCertRecord[] raw backend payload (optional when useMock)
 *   useMock    – boolean: substitute mock fixtures (default: false)
 *   onRenew    – async function(certRow) → void
 *   lastSyncAt – ISO string: when the data was last fetched (optional)
 *   loading    – boolean: show loading spinner overlay
 *   error      – string | null: error message from data fetch
 */
export default function IngressCertDashboard({
  records = [],
  useMock = false,
  onRenew,
  lastSyncAt,
  loading = false,
  error = null,
}) {
  // When useMock is true, lazily import mock data.  React state used so this
  // only triggers one import round-trip.
  const [mockRows, setMockRows] = useState(null);

  if (useMock && mockRows === null) {
    import('./mockCerts.js').then(({ MOCK_CERTS }) => {
      setMockRows(MOCK_CERTS.map(deriveCertRow));
    });
  }

  const rows = useMemo(() => {
    if (useMock) return mockRows ?? [];
    return sortCertRows(records.map(deriveCertRow), 'daysRemaining', 'asc');
  }, [records, useMock, mockRows]);

  const counts = useMemo(() => summaryCounts(rows), [rows]);

  const lastSyncLabel = lastSyncAt
    ? new Date(lastSyncAt).toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit', second: '2-digit' })
    : null;

  return (
    <div className="icd-page">
      {/* ── Header ── */}
      <header className="icd-header">
        <div className="icd-brand">
          <span className="eyebrow">STELLAR / INGRESS &amp; TLS</span>
          <h1>Certificate Expiration Monitor</h1>
          <p>
            Live health of SSL/TLS certificates across all cluster Ingress and HTTPRoute objects.
            Rows are sorted by urgency — expired and near-expiry certificates appear first.
          </p>
        </div>
        <div className="icd-header-meta">
          {lastSyncLabel && (
            <span className="icd-sync-time muted">
              Last sync: <strong>{lastSyncLabel}</strong>
            </span>
          )}
          {loading && <span className="icd-loading-badge" aria-live="polite" aria-busy="true">Refreshing…</span>}
        </div>
      </header>

      {/* ── Summary cards ── */}
      <section className="icd-metric-strip" aria-label="Certificate health summary">
        <MetricCard
          label="Total"
          value={counts.total}
          detail="Ingress endpoints"
          tone=""
        />
        <MetricCard
          label="Expired"
          value={counts.expired}
          detail="Cert has passed expiry"
          tone="red"
          alert={counts.expired > 0}
        />
        <MetricCard
          label="Critical"
          value={counts.critical}
          detail="< 7 days remaining"
          tone="red"
          alert={counts.critical > 0}
        />
        <MetricCard
          label="Warning"
          value={counts.warning}
          detail="< 30 days remaining"
          tone="amber"
        />
        <MetricCard
          label="Healthy"
          value={counts.healthy}
          detail="≥ 30 days remaining"
          tone="green"
        />
      </section>

      {/* ── Alert banner ── */}
      {(counts.expired > 0 || counts.critical > 0) && (
        <div className="icd-alert-banner" role="alert" aria-live="assertive">
          <span className="icd-alert-icon" aria-hidden="true">⚠</span>
          <strong>
            {counts.expired > 0
              ? `${counts.expired} certificate${counts.expired > 1 ? 's' : ''} have already expired.`
              : `${counts.critical} certificate${counts.critical > 1 ? 's' : ''} will expire within 7 days.`}
          </strong>{' '}
          Use "Force Renewal" on affected rows or check cert-manager logs immediately.
        </div>
      )}

      {/* ── Error state ── */}
      {error && (
        <div className="icd-error-banner" role="alert">
          <strong>Failed to load certificate data:</strong> {error}
        </div>
      )}

      {/* ── Table ── */}
      <section className="icd-table-section" aria-label="Certificate details">
        <div className="icd-table-heading">
          <h2>All Certificates</h2>
          <p className="muted">
            Click a row's ▸ to expand full certificate metadata.
            Colour-coded rows: <ColorSwatch status="expired" /><ColorSwatch status="critical" />
            <ColorSwatch status="warning" /><ColorSwatch status="healthy" />
          </p>
        </div>
        <IngressCertTable rows={rows} onRenew={onRenew} />
      </section>
    </div>
  );
}

// ── Internal sub-components ───────────────────────────────────────────────────

function MetricCard({ label, value, detail, tone, alert = false }) {
  return (
    <div
      className={`icd-metric ${tone ? `icd-metric--${tone}` : ''} ${alert ? 'icd-metric--alert' : ''}`}
      aria-label={`${label}: ${value}`}
    >
      <span className="icd-metric-label">{label}</span>
      <strong className={tone ? `tone-${tone}` : ''}>{value}</strong>
      <span className="muted">{detail}</span>
      {alert && <span className="icd-metric-alert-dot" aria-hidden="true" />}
    </div>
  );
}

function ColorSwatch({ status }) {
  const LABEL = { expired: 'Expired', critical: 'Critical', warning: 'Warning', healthy: 'Healthy' };
  return (
    <span className={`icd-legend-item icd-legend-item--${status}`}>
      <span className={`icd-legend-swatch icd-legend-swatch--${status}`} aria-hidden="true" />
      {LABEL[status]}
    </span>
  );
}
