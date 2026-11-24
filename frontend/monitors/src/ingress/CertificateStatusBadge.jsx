import { colorFromStatus } from './certUtils.js';

/**
 * CertificateStatusBadge
 *
 * Renders a compact, colour-coded pill label for a certificate status bucket.
 *
 * Props:
 *   status  – 'expired' | 'critical' | 'warning' | 'healthy'
 *   days    – (optional) signed integer days remaining, shown inside the badge
 *   size    – 'sm' | 'md' (default 'md')
 */
export default function CertificateStatusBadge({ status, days, size = 'md' }) {
  const color = colorFromStatus(status);

  // Derive contrasting text: dark on green/amber, light on red
  const textColor = status === 'expired' || status === 'critical' ? '#ffffff' : '#07110d';

  // Badge label
  const label = LABEL[status] ?? status;

  // Days suffix (omitted for healthy to reduce noise)
  const daysSuffix =
    days !== undefined && Number.isFinite(days) && status !== 'healthy'
      ? ` (${days < 0 ? days : `+${days}`}d)`
      : '';

  return (
    <span
      className={`cert-badge cert-badge--${status} cert-badge--${size}`}
      style={{ '--badge-color': color, '--badge-text': textColor }}
      aria-label={`Certificate status: ${label}${daysSuffix}`}
    >
      <span className="cert-badge__dot" aria-hidden="true" />
      {label}{daysSuffix}
    </span>
  );
}

const LABEL = {
  healthy:  'Healthy',
  warning:  'Warning',
  critical: 'Critical',
  expired:  'Expired',
};
