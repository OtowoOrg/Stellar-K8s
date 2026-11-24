/**
 * @module ingress
 *
 * Public surface of the Ingress & TLS Certificate Expiration monitor module.
 * Import from here to avoid depending on internal file layout.
 */

// Core utilities
export {
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

// Components (React)
export { default as CertificateStatusBadge } from './CertificateStatusBadge.jsx';
export { default as ForceRenewalButton }      from './ForceRenewalButton.jsx';
export { default as IngressCertTable }        from './IngressCertTable.jsx';
export { default as IngressCertDashboard }    from './IngressCertDashboard.jsx';

// Mock data (for development / tests only – tree-shaken in production builds)
export { MOCK_CERTS } from './mockCerts.js';
