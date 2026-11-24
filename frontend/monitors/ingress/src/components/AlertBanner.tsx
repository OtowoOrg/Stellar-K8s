import React from "react";
import { CertificateInfo } from "../types.js";

interface AlertBannerProps {
    criticalCerts: CertificateInfo[];
    onRenewAllCritical?: () => Promise<void> | void;
    isRenewingAll?: boolean;
}

export const AlertBanner: React.FC<AlertBannerProps> = ({
    criticalCerts,
    onRenewAllCritical,
    isRenewingAll = false,
}) => {
    if (criticalCerts.length === 0) return null;

    const expiredCount = criticalCerts.filter(
        (c) => c.isExpired || c.daysRemaining <= 0,
    ).length;
    const nearExpiryCount = criticalCerts.length - expiredCount;

    return (
        <aside
            className="critical-alert-banner"
            role="alert"
            aria-live="assertive"
        >
            <div className="alert-content-left">
                <div className="alert-badge-icon">
                    <span className="pulse-dot" />
                    <span className="alert-icon-symbol">⚠️</span>
                </div>
                <div className="alert-text-block">
                    <h2 className="alert-title">
                        Urgent SSL/TLS Expiration Alert ({criticalCerts.length}{" "}
                        Endpoint{criticalCerts.length > 1 ? "s" : ""} Affected)
                    </h2>
                    <p className="alert-description">
                        {expiredCount > 0 && (
                            <strong className="expired-highlight">
                                {expiredCount} certificate
                                {expiredCount > 1 ? "s are" : " is"} EXPIRED.
                            </strong>
                        )}{" "}
                        {nearExpiryCount > 0 && (
                            <span>
                                {nearExpiryCount} certificate
                                {nearExpiryCount > 1
                                    ? "s expire"
                                    : " expires"}{" "}
                                in less than 7 days, threatening immediate RPC
                                &amp; Horizon service disruption.
                            </span>
                        )}
                    </p>
                    <div className="affected-hostnames-list">
                        {criticalCerts.slice(0, 4).map((c) => (
                            <span key={c.id} className="affected-chip">
                                {c.hostname} (
                                {c.daysRemaining <= 0
                                    ? "Expired"
                                    : `${c.daysRemaining}d remaining`}
                                )
                            </span>
                        ))}
                        {criticalCerts.length > 4 && (
                            <span className="affected-chip-more">
                                +{criticalCerts.length - 4} more
                            </span>
                        )}
                    </div>
                </div>
            </div>

            <div className="alert-action-block">
                <button
                    type="button"
                    className="btn-emergency-renew"
                    onClick={() => onRenewAllCritical && onRenewAllCritical()}
                    disabled={isRenewingAll}
                    aria-label="Trigger Emergency Renewal for All Critical Certificates"
                >
                    {isRenewingAll ? (
                        <>
                            <span className="spinner-icon" aria-hidden="true" />
                            <span>
                                Renewing Critical ({criticalCerts.length})...
                            </span>
                        </>
                    ) : (
                        <>
                            <span className="lightning-icon" aria-hidden="true">
                                ⚡
                            </span>
                            <span>Trigger Emergency Renewal</span>
                        </>
                    )}
                </button>
            </div>
        </aside>
    );
};

export default AlertBanner;
