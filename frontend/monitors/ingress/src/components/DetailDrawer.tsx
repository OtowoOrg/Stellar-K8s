import React, { useState } from "react";
import { formatDate, formatDaysRemaining } from "../certUtils.js";
import { CertificateInfo } from "../types.js";

interface DetailDrawerProps {
    certificate: CertificateInfo | null;
    onClose: () => void;
    onRenewCertificate?: (certId: string) => Promise<void> | void;
    isRenewing?: boolean;
}

export const DetailDrawer: React.FC<DetailDrawerProps> = ({
    certificate,
    onClose,
    onRenewCertificate,
    isRenewing = false,
}) => {
    const [copiedField, setCopiedField] = useState<string | null>(null);

    if (!certificate) return null;

    const copyToClipboard = (text: string, fieldName: string) => {
        navigator.clipboard.writeText(text);
        setCopiedField(fieldName);
        setTimeout(() => setCopiedField(null), 2000);
    };

    const urgency = certificate.urgency;

    return (
        <div
            className="drawer-overlay"
            onClick={onClose}
            role="dialog"
            aria-modal="true"
            aria-label="Certificate Details"
        >
            <aside
                className="drawer-panel"
                onClick={(e) => e.stopPropagation()}
            >
                {/* Header */}
                <div className="drawer-header">
                    <div className="drawer-header-title">
                        <span className={`status-indicator-dot ${urgency}`} />
                        <div>
                            <span className="drawer-eyebrow">
                                X.509 CERTIFICATE INSPECTOR
                            </span>
                            <h2>{certificate.hostname}</h2>
                        </div>
                    </div>
                    <button
                        type="button"
                        className="drawer-close-btn"
                        onClick={onClose}
                        aria-label="Close Inspector"
                    >
                        ✕
                    </button>
                </div>

                {/* Urgency Status Banner */}
                <div className={`drawer-urgency-strip urgency-${urgency}`}>
                    <div className="urgency-strip-left">
                        <span className="urgency-label">
                            {urgency === "critical"
                                ? "🔴 CRITICAL URGENCY"
                                : urgency === "warning"
                                  ? "🟡 WARNING"
                                  : "🟢 HEALTHY"}
                        </span>
                        <span className="urgency-expiry-text">
                            {formatDaysRemaining(certificate.daysRemaining)} (
                            {formatDate(certificate.notAfter)})
                        </span>
                    </div>
                    <button
                        type="button"
                        className="btn-renew-drawer"
                        onClick={() =>
                            onRenewCertificate &&
                            onRenewCertificate(certificate.id)
                        }
                        disabled={
                            isRenewing ||
                            certificate.renewalStatus === "renewing"
                        }
                    >
                        {isRenewing
                            ? "Renewing..."
                            : certificate.renewalStatus === "renewed"
                              ? "✓ Renewed"
                              : "⟳ Trigger Renewal"}
                    </button>
                </div>

                {/* Content Body */}
                <div className="drawer-body">
                    {/* Section: Route & Networking */}
                    <section className="drawer-section">
                        <h3 className="section-title">
                            Ingress &amp; Routing Specifications
                        </h3>
                        <dl className="property-grid">
                            <div className="prop-row">
                                <dt>Resource Name</dt>
                                <dd>
                                    <code>{certificate.name}</code>
                                </dd>
                            </div>
                            <div className="prop-row">
                                <dt>Route Type</dt>
                                <dd>
                                    <span
                                        className={`badge badge-${certificate.routeType.toLowerCase()}`}
                                    >
                                        {certificate.routeType}
                                    </span>
                                </dd>
                            </div>
                            <div className="prop-row">
                                <dt>Namespace</dt>
                                <dd>
                                    <span className="namespace-badge">
                                        {certificate.namespace}
                                    </span>
                                </dd>
                            </div>
                            <div className="prop-row">
                                <dt>Cluster</dt>
                                <dd>{certificate.cluster}</dd>
                            </div>
                            <div className="prop-row">
                                <dt>Service Backend</dt>
                                <dd>
                                    <code>{certificate.serviceEndpoint}</code>
                                </dd>
                            </div>
                            <div className="prop-row">
                                <dt>TLS Secret Reference</dt>
                                <dd>
                                    <code>{certificate.secretName}</code>
                                </dd>
                            </div>
                        </dl>
                    </section>

                    {/* Section: X.509 Certificate Metadata */}
                    <section className="drawer-section">
                        <h3 className="section-title">
                            Certificate &amp; Cryptographic Details
                        </h3>
                        <dl className="property-grid">
                            <div className="prop-row">
                                <dt>Issuer</dt>
                                <dd>{certificate.issuer}</dd>
                            </div>
                            <div className="prop-row">
                                <dt>Issuer Type</dt>
                                <dd>
                                    <span className="issuer-type-tag">
                                        {certificate.issuerType}
                                    </span>
                                </dd>
                            </div>
                            <div className="prop-row">
                                <dt>Signature Algorithm</dt>
                                <dd>
                                    <code>
                                        {certificate.signatureAlgorithm}
                                    </code>
                                </dd>
                            </div>
                            <div className="prop-row">
                                <dt>Serial Number</dt>
                                <dd>
                                    <code>{certificate.serialNumber}</code>
                                </dd>
                            </div>
                            <div className="prop-row">
                                <dt>Valid From (Issued)</dt>
                                <dd>{formatDate(certificate.notBefore)}</dd>
                            </div>
                            <div className="prop-row">
                                <dt>Valid Until (Expires)</dt>
                                <dd
                                    className={
                                        urgency === "critical"
                                            ? "tone-red font-bold"
                                            : ""
                                    }
                                >
                                    {formatDate(certificate.notAfter)}
                                </dd>
                            </div>
                            <div className="prop-row">
                                <dt>Auto-Renewal (cert-manager)</dt>
                                <dd>
                                    {certificate.autoRenewal ? (
                                        <span className="badge badge-healthy-sm">
                                            ✓ Enabled
                                        </span>
                                    ) : (
                                        <span className="badge badge-warning-sm">
                                            Manual Management
                                        </span>
                                    )}
                                </dd>
                            </div>
                        </dl>
                    </section>

                    {/* Section: Subject Alternative Names (SANs) */}
                    <section className="drawer-section">
                        <div className="section-header-row">
                            <h3 className="section-title">
                                Subject Alternative Names (
                                {certificate.sans.length})
                            </h3>
                            <button
                                type="button"
                                className="btn-copy-all"
                                onClick={() =>
                                    copyToClipboard(
                                        certificate.sans.join(", "),
                                        "all-sans",
                                    )
                                }
                            >
                                {copiedField === "all-sans"
                                    ? "✓ Copied SANs"
                                    : "Copy All SANs"}
                            </button>
                        </div>
                        <div className="drawer-sans-list">
                            {certificate.sans.map((san, idx) => (
                                <div key={idx} className="drawer-san-item">
                                    <code>{san}</code>
                                    <button
                                        type="button"
                                        className="btn-copy-chip"
                                        onClick={() =>
                                            copyToClipboard(san, `san-${idx}`)
                                        }
                                        title="Copy hostname"
                                        aria-label={`Copy SAN ${san}`}
                                    >
                                        {copiedField === `san-${idx}`
                                            ? "✓"
                                            : "📋"}
                                    </button>
                                </div>
                            ))}
                        </div>
                    </section>

                    {/* Section: Annotations (if any) */}
                    {certificate.annotations &&
                        Object.keys(certificate.annotations).length > 0 && (
                            <section className="drawer-section">
                                <h3 className="section-title">
                                    Kubernetes Resource Annotations
                                </h3>
                                <div className="annotations-table-wrap">
                                    <table className="annotations-table">
                                        <tbody>
                                            {Object.entries(
                                                certificate.annotations,
                                            ).map(([k, v]) => (
                                                <tr key={k}>
                                                    <td className="annotation-key">
                                                        <code>{k}</code>
                                                    </td>
                                                    <td className="annotation-val">
                                                        <code>{v}</code>
                                                    </td>
                                                </tr>
                                            ))}
                                        </tbody>
                                    </table>
                                </div>
                            </section>
                        )}
                </div>

                {/* Footer */}
                <div className="drawer-footer">
                    <button
                        type="button"
                        className="btn-secondary"
                        onClick={onClose}
                    >
                        Close Inspector
                    </button>
                    <button
                        type="button"
                        className="btn-primary"
                        onClick={() =>
                            copyToClipboard(
                                `kubectl describe ${certificate.routeType.toLowerCase()} ${certificate.name} -n ${certificate.namespace}`,
                                "kubectl",
                            )
                        }
                    >
                        {copiedField === "kubectl"
                            ? "✓ Command Copied"
                            : "Copy kubectl Command"}
                    </button>
                </div>
            </aside>
        </div>
    );
};

export default DetailDrawer;
