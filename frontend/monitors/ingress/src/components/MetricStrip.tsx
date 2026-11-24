import React from "react";
import { MetricSummary } from "../types.js";

interface MetricStripProps {
    metrics: MetricSummary;
    onFilterUrgency?: (
        urgency: "all" | "critical" | "warning" | "healthy",
    ) => void;
    activeUrgency?: string;
}

export const MetricStrip: React.FC<MetricStripProps> = ({
    metrics,
    onFilterUrgency,
    activeUrgency = "all",
}) => {
    return (
        <section
            className="metric-strip"
            aria-label="SSL/TLS Certificate Health Summary"
        >
            <div
                className={`metric-card ${activeUrgency === "all" ? "active-filter" : ""}`}
                onClick={() => onFilterUrgency && onFilterUrgency("all")}
                role="button"
                tabIndex={0}
                aria-label="View all monitored certificates"
            >
                <span className="metric-label">Monitored Endpoints</span>
                <strong className="metric-value">
                    {metrics.total.toLocaleString()}
                </strong>
                <span className="metric-detail">Ingress &amp; HTTPRoutes</span>
            </div>

            <div
                className={`metric-card metric-critical ${metrics.critical > 0 ? "pulse-border" : ""} ${activeUrgency === "critical" ? "active-filter" : ""}`}
                onClick={() => onFilterUrgency && onFilterUrgency("critical")}
                role="button"
                tabIndex={0}
                aria-label="Filter critical & expired certificates"
            >
                <div className="metric-label-row">
                    <span className="metric-label tone-red">
                        Critical / Expired
                    </span>
                    <span className="badge badge-critical-sm">&lt; 7 Days</span>
                </div>
                <strong className="metric-value tone-red">
                    {metrics.critical.toLocaleString()}
                </strong>
                <span className="metric-detail">
                    {metrics.expired > 0 ? `${metrics.expired} expired, ` : ""}
                    {metrics.critical - metrics.expired} near expiration
                </span>
            </div>

            <div
                className={`metric-card metric-warning ${activeUrgency === "warning" ? "active-filter" : ""}`}
                onClick={() => onFilterUrgency && onFilterUrgency("warning")}
                role="button"
                tabIndex={0}
                aria-label="Filter warning certificates"
            >
                <div className="metric-label-row">
                    <span className="metric-label tone-amber">
                        Expiring Soon
                    </span>
                    <span className="badge badge-warning-sm">&lt; 30 Days</span>
                </div>
                <strong className="metric-value tone-amber">
                    {metrics.warning.toLocaleString()}
                </strong>
                <span className="metric-detail">
                    Action required within 30d
                </span>
            </div>

            <div
                className={`metric-card metric-healthy ${activeUrgency === "healthy" ? "active-filter" : ""}`}
                onClick={() => onFilterUrgency && onFilterUrgency("healthy")}
                role="button"
                tabIndex={0}
                aria-label="Filter healthy certificates"
            >
                <div className="metric-label-row">
                    <span className="metric-label tone-green">Healthy</span>
                    <span className="badge badge-healthy-sm">&gt; 30 Days</span>
                </div>
                <strong className="metric-value tone-green">
                    {metrics.healthy.toLocaleString()}
                </strong>
                <span className="metric-detail">
                    Certificates valid &gt; 30d
                </span>
            </div>

            <div className="metric-card metric-autorenew">
                <span className="metric-label">Auto-Renewal Coverage</span>
                <strong className="metric-value">
                    {metrics.autoRenewalPercentage}%
                </strong>
                <span className="metric-detail">
                    {metrics.autoRenewalCount} of {metrics.total} managed via
                    cert-manager
                </span>
            </div>
        </section>
    );
};

export default MetricStrip;
