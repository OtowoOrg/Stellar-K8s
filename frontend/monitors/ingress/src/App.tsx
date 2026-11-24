import React, { useCallback, useEffect, useMemo, useState } from "react";
import { CertTable } from "../../../components/cert_table.js";
import { calculateMetrics } from "./certUtils.js";
import { AlertBanner } from "./components/AlertBanner.js";
import { DetailDrawer } from "./components/DetailDrawer.js";
import { MetricStrip } from "./components/MetricStrip.js";
import { generateMockCertificates } from "./mockData.js";
import "./styles.css";
import { CertificateInfo } from "./types.js";

export const App: React.FC = () => {
    const [dataSource, setDataSource] = useState<"mock" | "live">("mock");
    const [certificates, setCertificates] = useState<CertificateInfo[]>(() =>
        generateMockCertificates(),
    );
    const [selectedCert, setSelectedCert] = useState<CertificateInfo | null>(
        null,
    );
    const [isRenewingAll, setIsRenewingAll] = useState<boolean>(false);
    const [lastRefreshed, setLastRefreshed] = useState<Date>(new Date());
    const [autoRefresh, setAutoRefresh] = useState<boolean>(true);
    const [refreshSeconds, setRefreshSeconds] = useState<number>(30);
    const [activeUrgencyFilter, setActiveUrgencyFilter] = useState<
        "all" | "critical" | "warning" | "healthy"
    >("all");

    // Load certificates (either from API or mock dataset)
    const refreshData = useCallback(async () => {
        if (dataSource === "live") {
            try {
                const response = await fetch("/api/v1/certificates/ingress");
                if (response.ok) {
                    const data = await response.json();
                    if (Array.isArray(data)) {
                        setCertificates(data);
                        setLastRefreshed(new Date());
                        return;
                    }
                }
            } catch {
                // Fallback to mock if live API is unavailable
            }
        }
        setCertificates(generateMockCertificates());
        setLastRefreshed(new Date());
    }, [dataSource]);

    // Periodic Auto-refresh
    useEffect(() => {
        if (!autoRefresh) return;
        const interval = setInterval(() => {
            refreshData();
        }, refreshSeconds * 1000);
        return () => clearInterval(interval);
    }, [autoRefresh, refreshSeconds, refreshData]);

    // Aggregate summary metrics
    const metrics = useMemo(
        () => calculateMetrics(certificates),
        [certificates],
    );

    // Critical / Expired certificates list for alert banner
    const criticalCerts = useMemo(() => {
        return certificates.filter(
            (c) =>
                c.urgency === "critical" || c.isExpired || c.daysRemaining < 7,
        );
    }, [certificates]);

    // Handler: Single Certificate Renewal
    const handleRenewCertificate = useCallback(async (certId: string) => {
        // 1. Set renewing status in state
        setCertificates((prev) =>
            prev.map((c) =>
                c.id === certId ? { ...c, renewalStatus: "renewing" } : c,
            ),
        );

        // If detail drawer open, update selected cert too
        setSelectedCert((prev) =>
            prev?.id === certId ? { ...prev, renewalStatus: "renewing" } : prev,
        );

        // Simulate cert-manager reissuance cycle
        await new Promise((resolve) => setTimeout(resolve, 1200));

        // 2. Adopt renewed certificate (+90 days validity, healthy urgency)
        const now = new Date();
        const newExpiry = new Date(now.getTime() + 90 * 24 * 60 * 60 * 1000);

        setCertificates((prev) =>
            prev.map((c) => {
                if (c.id !== certId) return c;
                return {
                    ...c,
                    renewalStatus: "renewed",
                    isExpired: false,
                    daysRemaining: 90,
                    urgency: "healthy",
                    notBefore: now.toISOString(),
                    notAfter: newExpiry.toISOString(),
                    lastRenewalAttempt: now.toISOString(),
                };
            }),
        );

        setSelectedCert((prev) => {
            if (prev?.id !== certId) return prev;
            return {
                ...prev,
                renewalStatus: "renewed",
                isExpired: false,
                daysRemaining: 90,
                urgency: "healthy",
                notBefore: now.toISOString(),
                notAfter: newExpiry.toISOString(),
                lastRenewalAttempt: now.toISOString(),
            };
        });
    }, []);

    // Handler: Batch Emergency Renewal for all Critical Certificates
    const handleRenewAllCritical = useCallback(async () => {
        if (criticalCerts.length === 0 || isRenewingAll) return;
        setIsRenewingAll(true);

        const criticalIds = new Set(criticalCerts.map((c) => c.id));

        setCertificates((prev) =>
            prev.map((c) =>
                criticalIds.has(c.id) ? { ...c, renewalStatus: "renewing" } : c,
            ),
        );

        await new Promise((resolve) => setTimeout(resolve, 1800));

        const now = new Date();
        const newExpiry = new Date(now.getTime() + 90 * 24 * 60 * 60 * 1000);

        setCertificates((prev) =>
            prev.map((c) => {
                if (!criticalIds.has(c.id)) return c;
                return {
                    ...c,
                    renewalStatus: "renewed",
                    isExpired: false,
                    daysRemaining: 90,
                    urgency: "healthy",
                    notBefore: now.toISOString(),
                    notAfter: newExpiry.toISOString(),
                    lastRenewalAttempt: now.toISOString(),
                };
            }),
        );

        setIsRenewingAll(false);
    }, [criticalCerts, isRenewingAll]);

    return (
        <main className="app-shell">
            {/* Top Header Bar */}
            <header className="topbar">
                <div className="brand-block">
                    <span className="eyebrow">
                        STELLAR / OBSERVABILITY &amp; SECURITY
                    </span>
                    <h1>Public Ingress &amp; SSL/TLS Certificate Monitor</h1>
                    <p>
                        Real-time SSL/TLS health and certificate expiration
                        tracking across Horizon REST and Soroban RPC endpoints.
                    </p>
                </div>

                <div
                    className="header-controls"
                    role="toolbar"
                    aria-label="Dashboard controls"
                >
                    <label className="control-item">
                        <span>Data Source:</span>
                        <select
                            value={dataSource}
                            onChange={(e) =>
                                setDataSource(e.target.value as "mock" | "live")
                            }
                            aria-label="Select data source"
                        >
                            <option value="mock">
                                Mock Payload (50+ Routes Fleet)
                            </option>
                            <option value="live">
                                Cluster API (/api/v1/certificates)
                            </option>
                        </select>
                    </label>

                    <label className="control-item">
                        <span>Auto Refresh:</span>
                        <select
                            value={autoRefresh ? String(refreshSeconds) : "off"}
                            onChange={(e) => {
                                if (e.target.value === "off") {
                                    setAutoRefresh(false);
                                } else {
                                    setAutoRefresh(true);
                                    setRefreshSeconds(Number(e.target.value));
                                }
                            }}
                            aria-label="Auto refresh frequency"
                        >
                            <option value="10">Every 10s</option>
                            <option value="30">Every 30s</option>
                            <option value="60">Every 60s</option>
                            <option value="off">Manual Only</option>
                        </select>
                    </label>

                    <button
                        type="button"
                        className="btn-secondary"
                        onClick={refreshData}
                        title="Refresh certificate statuses now"
                        aria-label="Refresh certificates"
                    >
                        ⟳ Refresh
                    </button>
                </div>
            </header>

            {/* Critical Alert Banner (Appears when any cert is expired or < 7 days) */}
            <AlertBanner
                criticalCerts={criticalCerts}
                onRenewAllCritical={handleRenewAllCritical}
                isRenewingAll={isRenewingAll}
            />

            {/* Summary KPI Metric Strip */}
            <MetricStrip
                metrics={metrics}
                onFilterUrgency={(u) => setActiveUrgencyFilter(u)}
                activeUrgency={activeUrgencyFilter}
            />

            {/* Reusable Core Data Table Component */}
            <section
                className="table-workspace-section"
                aria-label="Certificate inventory table"
            >
                <CertTable
                    certificates={certificates}
                    onRenewCertificate={handleRenewCertificate}
                    onSelectCertificate={(cert) => setSelectedCert(cert)}
                    selectedCertId={selectedCert?.id}
                    initialSortField="daysRemaining"
                    initialSortDirection="asc"
                    defaultPageSize={15}
                />
            </section>

            {/* Side Inspector Drawer for Deep Diagnostics */}
            {selectedCert && (
                <DetailDrawer
                    certificate={selectedCert}
                    onClose={() => setSelectedCert(null)}
                    onRenewCertificate={handleRenewCertificate}
                />
            )}
        </main>
    );
};

export default App;
