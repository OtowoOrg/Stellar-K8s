import React, { useMemo, useState } from "react";
import {
    formatDate,
    formatDaysRemaining,
    getCertUrgency,
} from "../monitors/ingress/src/certUtils.js";
import {
    CertificateInfo,
    CertUrgency,
    FilterState,
    RouteType,
    SortDirection,
    SortField,
    UrgencyFilter,
} from "../monitors/ingress/src/types.js";

export interface CertTableProps {
    certificates: CertificateInfo[];
    onRenewCertificate?: (certId: string) => Promise<void> | void;
    onSelectCertificate?: (cert: CertificateInfo) => void;
    selectedCertId?: string | null;
    initialSortField?: SortField;
    initialSortDirection?: SortDirection;
    defaultPageSize?: number;
    showControls?: boolean;
}

export const CertTable: React.FC<CertTableProps> = ({
    certificates,
    onRenewCertificate,
    onSelectCertificate,
    selectedCertId = null,
    initialSortField = "daysRemaining",
    initialSortDirection = "asc",
    defaultPageSize = 15,
    showControls = true,
}) => {
    const [search, setSearch] = useState<string>("");
    const [urgencyFilter, setUrgencyFilter] = useState<UrgencyFilter>("all");
    const [routeTypeFilter, setRouteTypeFilter] = useState<"all" | RouteType>(
        "all",
    );
    const [namespaceFilter, setNamespaceFilter] = useState<string>("all");
    const [sortField, setSortField] = useState<SortField>(initialSortField);
    const [sortDirection, setSortDirection] =
        useState<SortDirection>(initialSortDirection);
    const [currentPage, setCurrentPage] = useState<number>(1);
    const [pageSize, setPageSize] = useState<number>(defaultPageSize);
    const [renewingMap, setRenewingMap] = useState<Record<string, boolean>>({});
    const [expandedSansMap, setExpandedSansMap] = useState<
        Record<string, boolean>
    >({});

    // Collect unique namespaces for dropdown filter
    const namespaces = useMemo(() => {
        const set = new Set<string>();
        for (const cert of certificates) {
            if (cert.namespace) set.add(cert.namespace);
        }
        return Array.from(set).sort();
    }, [certificates]);

    // Filter logic
    const filteredCertificates = useMemo(() => {
        const query = search.trim().toLowerCase();

        return certificates.filter((cert) => {
            // 1. Urgency filter
            if (urgencyFilter !== "all") {
                if (urgencyFilter === "expired") {
                    if (!cert.isExpired && cert.daysRemaining > 0) return false;
                } else if (cert.urgency !== urgencyFilter) {
                    return false;
                }
            }

            // 2. Route type filter
            if (
                routeTypeFilter !== "all" &&
                cert.routeType !== routeTypeFilter
            ) {
                return false;
            }

            // 3. Namespace filter
            if (
                namespaceFilter !== "all" &&
                cert.namespace !== namespaceFilter
            ) {
                return false;
            }

            // 4. Full-text search
            if (query) {
                const matchHostname = cert.hostname
                    .toLowerCase()
                    .includes(query);
                const matchName = cert.name.toLowerCase().includes(query);
                const matchNamespace = cert.namespace
                    .toLowerCase()
                    .includes(query);
                const matchCluster = cert.cluster.toLowerCase().includes(query);
                const matchIssuer = cert.issuer.toLowerCase().includes(query);
                const matchSecret = cert.secretName
                    .toLowerCase()
                    .includes(query);
                const matchEndpoint = cert.serviceEndpoint
                    .toLowerCase()
                    .includes(query);
                const matchSans = cert.sans.some((san) =>
                    san.toLowerCase().includes(query),
                );

                if (
                    !matchHostname &&
                    !matchName &&
                    !matchNamespace &&
                    !matchCluster &&
                    !matchIssuer &&
                    !matchSecret &&
                    !matchEndpoint &&
                    !matchSans
                ) {
                    return false;
                }
            }

            return true;
        });
    }, [certificates, search, urgencyFilter, routeTypeFilter, namespaceFilter]);

    // Sort logic (Prioritizes expiring certificates by default)
    const sortedCertificates = useMemo(() => {
        const modifier = sortDirection === "asc" ? 1 : -1;

        return [...filteredCertificates].sort((a, b) => {
            let comparison = 0;

            switch (sortField) {
                case "daysRemaining":
                    comparison = a.daysRemaining - b.daysRemaining;
                    break;
                case "notAfter":
                    comparison =
                        new Date(a.notAfter).getTime() -
                        new Date(b.notAfter).getTime();
                    break;
                case "hostname":
                    comparison = a.hostname.localeCompare(b.hostname);
                    break;
                case "name":
                    comparison = a.name.localeCompare(b.name);
                    break;
                case "namespace":
                    comparison = a.namespace.localeCompare(b.namespace);
                    break;
                case "cluster":
                    comparison = a.cluster.localeCompare(b.cluster);
                    break;
                case "routeType":
                    comparison = a.routeType.localeCompare(b.routeType);
                    break;
                case "issuer":
                    comparison = a.issuer.localeCompare(b.issuer);
                    break;
                case "renewalStatus":
                    comparison = a.renewalStatus.localeCompare(b.renewalStatus);
                    break;
                default:
                    comparison = a.daysRemaining - b.daysRemaining;
                    break;
            }

            if (comparison !== 0) {
                return comparison * modifier;
            }

            // Stable tie-breakers: daysRemaining ASC then hostname ASC
            const tieBreakDays = a.daysRemaining - b.daysRemaining;
            if (tieBreakDays !== 0) return tieBreakDays;
            return a.hostname.localeCompare(b.hostname);
        });
    }, [filteredCertificates, sortField, sortDirection]);

    // Pagination calculation
    const totalItems = sortedCertificates.length;
    const totalPages =
        pageSize === 0 ? 1 : Math.max(1, Math.ceil(totalItems / pageSize));
    const activePage = Math.min(currentPage, totalPages);

    const paginatedCertificates = useMemo(() => {
        if (pageSize === 0) return sortedCertificates;
        const startIndex = (activePage - 1) * pageSize;
        return sortedCertificates.slice(startIndex, startIndex + pageSize);
    }, [sortedCertificates, activePage, pageSize]);

    const handleSort = (field: SortField) => {
        if (sortField === field) {
            setSortDirection((prev) => (prev === "asc" ? "desc" : "asc"));
        } else {
            setSortField(field);
            // Default to ascending for dates/expiry, otherwise ascending
            setSortDirection("asc");
        }
    };

    const handleTriggerRenewal = async (
        e: React.MouseEvent,
        cert: CertificateInfo,
    ) => {
        e.stopPropagation();
        if (renewingMap[cert.id] || cert.renewalStatus === "renewing") return;

        setRenewingMap((prev) => ({ ...prev, [cert.id]: true }));
        try {
            if (onRenewCertificate) {
                await onRenewCertificate(cert.id);
            }
        } finally {
            setRenewingMap((prev) => ({ ...prev, [cert.id]: false }));
        }
    };

    const toggleExpandSans = (e: React.MouseEvent, certId: string) => {
        e.stopPropagation();
        setExpandedSansMap((prev) => ({ ...prev, [certId]: !prev[certId] }));
    };

    const clearFilters = () => {
        setSearch("");
        setUrgencyFilter("all");
        setRouteTypeFilter("all");
        setNamespaceFilter("all");
        setCurrentPage(1);
    };

    const renderSortIndicator = (field: SortField) => {
        if (sortField !== field) {
            return (
                <span className="sort-icon inactive" aria-hidden="true">
                    ↕
                </span>
            );
        }
        return (
            <span className="sort-icon active" aria-hidden="true">
                {sortDirection === "asc" ? "▲" : "▼"}
            </span>
        );
    };

    return (
        <div className="cert-table-container">
            {showControls && (
                <div
                    className="cert-table-controls"
                    role="region"
                    aria-label="Table filters and search"
                >
                    <div className="search-bar-wrap">
                        <span className="search-icon" aria-hidden="true">
                            🔍
                        </span>
                        <input
                            type="text"
                            className="search-input"
                            placeholder="Search by hostname, route name, SANs, namespace, issuer..."
                            value={search}
                            onChange={(e) => {
                                setSearch(e.target.value);
                                setCurrentPage(1);
                            }}
                            aria-label="Search certificates"
                        />
                        {search && (
                            <button
                                type="button"
                                className="search-clear-btn"
                                onClick={() => setSearch("")}
                                aria-label="Clear search"
                            >
                                ✕
                            </button>
                        )}
                    </div>

                    <div className="filter-group">
                        {/* Urgency Filter Pills */}
                        <div
                            className="pill-group"
                            role="radiogroup"
                            aria-label="Filter by urgency"
                        >
                            <button
                                type="button"
                                className={`pill-btn ${urgencyFilter === "all" ? "active" : ""}`}
                                onClick={() => {
                                    setUrgencyFilter("all");
                                    setCurrentPage(1);
                                }}
                            >
                                All
                            </button>
                            <button
                                type="button"
                                className={`pill-btn tone-red ${urgencyFilter === "critical" ? "active" : ""}`}
                                onClick={() => {
                                    setUrgencyFilter("critical");
                                    setCurrentPage(1);
                                }}
                            >
                                Critical (&lt; 7d)
                            </button>
                            <button
                                type="button"
                                className={`pill-btn tone-amber ${urgencyFilter === "warning" ? "active" : ""}`}
                                onClick={() => {
                                    setUrgencyFilter("warning");
                                    setCurrentPage(1);
                                }}
                            >
                                Warning (&lt; 30d)
                            </button>
                            <button
                                type="button"
                                className={`pill-btn tone-green ${urgencyFilter === "healthy" ? "active" : ""}`}
                                onClick={() => {
                                    setUrgencyFilter("healthy");
                                    setCurrentPage(1);
                                }}
                            >
                                Healthy (&gt; 30d)
                            </button>
                        </div>

                        {/* Route Type Dropdown */}
                        <label className="filter-select-label">
                            <span className="filter-select-title">
                                Route Type
                            </span>
                            <select
                                className="filter-select"
                                value={routeTypeFilter}
                                onChange={(e) => {
                                    setRouteTypeFilter(
                                        e.target.value as "all" | RouteType,
                                    );
                                    setCurrentPage(1);
                                }}
                                aria-label="Filter by Route Type"
                            >
                                <option value="all">All Types</option>
                                <option value="Ingress">Ingress</option>
                                <option value="HTTPRoute">HTTPRoute</option>
                            </select>
                        </label>

                        {/* Namespace Dropdown */}
                        <label className="filter-select-label">
                            <span className="filter-select-title">
                                Namespace
                            </span>
                            <select
                                className="filter-select"
                                value={namespaceFilter}
                                onChange={(e) => {
                                    setNamespaceFilter(e.target.value);
                                    setCurrentPage(1);
                                }}
                                aria-label="Filter by Namespace"
                            >
                                <option value="all">All Namespaces</option>
                                {namespaces.map((ns) => (
                                    <option key={ns} value={ns}>
                                        {ns}
                                    </option>
                                ))}
                            </select>
                        </label>

                        {(search ||
                            urgencyFilter !== "all" ||
                            routeTypeFilter !== "all" ||
                            namespaceFilter !== "all") && (
                            <button
                                type="button"
                                className="btn-reset-filters"
                                onClick={clearFilters}
                            >
                                Reset Filters
                            </button>
                        )}
                    </div>
                </div>
            )}

            {/* Main Table */}
            <div
                className="table-responsive"
                tabIndex={0}
                aria-label="SSL/TLS Certificates Table"
            >
                <table className="cert-data-table">
                    <thead>
                        <tr>
                            <th
                                scope="col"
                                className={`sortable-th ${sortField === "hostname" ? "sorted" : ""}`}
                                onClick={() => handleSort("hostname")}
                                aria-sort={
                                    sortField === "hostname"
                                        ? sortDirection === "asc"
                                            ? "ascending"
                                            : "descending"
                                        : "none"
                                }
                            >
                                <div className="th-content">
                                    <span>Endpoint & Route</span>
                                    {renderSortIndicator("hostname")}
                                </div>
                            </th>

                            <th
                                scope="col"
                                className={`sortable-th ${sortField === "namespace" ? "sorted" : ""}`}
                                onClick={() => handleSort("namespace")}
                                aria-sort={
                                    sortField === "namespace"
                                        ? sortDirection === "asc"
                                            ? "ascending"
                                            : "descending"
                                        : "none"
                                }
                            >
                                <div className="th-content">
                                    <span>Namespace / Cluster</span>
                                    {renderSortIndicator("namespace")}
                                </div>
                            </th>

                            <th
                                scope="col"
                                className={`sortable-th ${sortField === "issuer" ? "sorted" : ""}`}
                                onClick={() => handleSort("issuer")}
                                aria-sort={
                                    sortField === "issuer"
                                        ? sortDirection === "asc"
                                            ? "ascending"
                                            : "descending"
                                        : "none"
                                }
                            >
                                <div className="th-content">
                                    <span>Certificate Issuer</span>
                                    {renderSortIndicator("issuer")}
                                </div>
                            </th>

                            <th scope="col">
                                <div className="th-content">
                                    <span>
                                        Subject Alternative Names (SANs)
                                    </span>
                                </div>
                            </th>

                            <th
                                scope="col"
                                className={`sortable-th ${sortField === "notAfter" ? "sorted" : ""}`}
                                onClick={() => handleSort("notAfter")}
                                aria-sort={
                                    sortField === "notAfter"
                                        ? sortDirection === "asc"
                                            ? "ascending"
                                            : "descending"
                                        : "none"
                                }
                            >
                                <div className="th-content">
                                    <span>Expiration Date</span>
                                    {renderSortIndicator("notAfter")}
                                </div>
                            </th>

                            <th
                                scope="col"
                                className={`sortable-th ${sortField === "daysRemaining" ? "sorted" : ""}`}
                                onClick={() => handleSort("daysRemaining")}
                                aria-sort={
                                    sortField === "daysRemaining"
                                        ? sortDirection === "asc"
                                            ? "ascending"
                                            : "descending"
                                        : "none"
                                }
                            >
                                <div className="th-content">
                                    <span>Days Remaining & Urgency</span>
                                    {renderSortIndicator("daysRemaining")}
                                </div>
                            </th>

                            <th scope="col" className="text-right">
                                <div className="th-content justify-end">
                                    <span>Actions</span>
                                </div>
                            </th>
                        </tr>
                    </thead>

                    <tbody>
                        {paginatedCertificates.length === 0 ? (
                            <tr className="empty-row">
                                <td colSpan={7}>
                                    <div className="empty-table-state">
                                        <span className="empty-state-icon">
                                            🛡️
                                        </span>
                                        <h3>
                                            No matching ingress certificates
                                            found
                                        </h3>
                                        <p>
                                            Try adjusting your search criteria
                                            or clearing active filters.
                                        </p>
                                        <button
                                            type="button"
                                            className="btn-secondary"
                                            onClick={clearFilters}
                                        >
                                            Clear All Filters
                                        </button>
                                    </div>
                                </td>
                            </tr>
                        ) : (
                            paginatedCertificates.map((cert) => {
                                const urgency =
                                    cert.urgency ||
                                    getCertUrgency(cert.daysRemaining);
                                const isSelected = selectedCertId === cert.id;
                                const isRenewing =
                                    renewingMap[cert.id] ||
                                    cert.renewalStatus === "renewing";
                                const isExpandedSans = Boolean(
                                    expandedSansMap[cert.id],
                                );

                                // Row urgency class:
                                // Red = Critical (< 7d) or Expired
                                // Yellow/Amber = Warning (< 30d)
                                // Green = Healthy (> 30d)
                                let rowUrgencyClass = "row-healthy";
                                if (
                                    urgency === "critical" ||
                                    cert.isExpired ||
                                    cert.daysRemaining < 7
                                ) {
                                    rowUrgencyClass = "row-critical";
                                } else if (
                                    urgency === "warning" ||
                                    cert.daysRemaining < 30
                                ) {
                                    rowUrgencyClass = "row-warning";
                                }

                                return (
                                    <tr
                                        key={cert.id}
                                        className={`cert-row ${rowUrgencyClass} ${isSelected ? "row-selected" : ""}`}
                                        onClick={() =>
                                            onSelectCertificate &&
                                            onSelectCertificate(cert)
                                        }
                                        data-cert-id={cert.id}
                                        data-urgency={urgency}
                                    >
                                        {/* Hostname & Route */}
                                        <td className="cell-endpoint">
                                            <div className="endpoint-meta">
                                                <div className="hostname-line">
                                                    <span
                                                        className={`status-indicator-dot ${urgency}`}
                                                    />
                                                    <strong
                                                        className="hostname-text"
                                                        title={cert.hostname}
                                                    >
                                                        {cert.hostname}
                                                    </strong>
                                                    <span
                                                        className={`badge badge-route-type badge-${cert.routeType.toLowerCase()}`}
                                                    >
                                                        {cert.routeType}
                                                    </span>
                                                </div>
                                                <div className="service-subtext">
                                                    <span className="resource-name">
                                                        {cert.name}
                                                    </span>
                                                    <span className="bullet-sep">
                                                        •
                                                    </span>
                                                    <span
                                                        className="service-endpoint"
                                                        title={
                                                            cert.serviceEndpoint
                                                        }
                                                    >
                                                        {cert.serviceEndpoint}
                                                    </span>
                                                </div>
                                            </div>
                                        </td>

                                        {/* Namespace / Cluster */}
                                        <td className="cell-namespace">
                                            <div className="namespace-meta">
                                                <span className="namespace-badge">
                                                    {cert.namespace}
                                                </span>
                                                <span className="cluster-text">
                                                    {cert.cluster}
                                                </span>
                                            </div>
                                        </td>

                                        {/* Certificate Issuer */}
                                        <td className="cell-issuer">
                                            <div className="issuer-meta">
                                                <span
                                                    className="issuer-name"
                                                    title={cert.issuer}
                                                >
                                                    {cert.issuer}
                                                </span>
                                                <span className="issuer-type-tag">
                                                    {cert.issuerType}
                                                </span>
                                            </div>
                                        </td>

                                        {/* Subject Alternative Names (SANs) */}
                                        <td className="cell-sans">
                                            <div className="sans-container">
                                                {cert.sans.length === 0 ? (
                                                    <span className="muted-text">
                                                        —
                                                    </span>
                                                ) : (
                                                    <>
                                                        <div className="sans-list">
                                                            {(isExpandedSans
                                                                ? cert.sans
                                                                : cert.sans.slice(
                                                                      0,
                                                                      2,
                                                                  )
                                                            ).map(
                                                                (san, idx) => (
                                                                    <span
                                                                        key={
                                                                            idx
                                                                        }
                                                                        className="san-chip"
                                                                        title={
                                                                            san
                                                                        }
                                                                    >
                                                                        {san}
                                                                    </span>
                                                                ),
                                                            )}
                                                        </div>
                                                        {cert.sans.length >
                                                            2 && (
                                                            <button
                                                                type="button"
                                                                className="san-expand-toggle"
                                                                onClick={(e) =>
                                                                    toggleExpandSans(
                                                                        e,
                                                                        cert.id,
                                                                    )
                                                                }
                                                                aria-label={
                                                                    isExpandedSans
                                                                        ? "Show fewer SANs"
                                                                        : `Show all ${cert.sans.length} SANs`
                                                                }
                                                            >
                                                                {isExpandedSans
                                                                    ? "▲ Show less"
                                                                    : `+${cert.sans.length - 2} more`}
                                                            </button>
                                                        )}
                                                    </>
                                                )}
                                            </div>
                                        </td>

                                        {/* Expiration Date */}
                                        <td className="cell-date">
                                            <div className="date-meta">
                                                <time
                                                    dateTime={cert.notAfter}
                                                    className="date-primary"
                                                >
                                                    {formatDate(cert.notAfter)}
                                                </time>
                                                <span className="valid-from-subtext">
                                                    Issued:{" "}
                                                    {cert.notBefore
                                                        ? formatDate(
                                                              cert.notBefore,
                                                          ).split(" ")[0]
                                                        : "—"}
                                                </span>
                                            </div>
                                        </td>

                                        {/* Days Remaining Counter & Urgency Badge */}
                                        <td className="cell-days">
                                            <div className="days-counter-meta">
                                                <span
                                                    className={`urgency-badge badge-${urgency}`}
                                                >
                                                    {urgency === "critical"
                                                        ? "🔴 Critical"
                                                        : urgency === "warning"
                                                          ? "🟡 Warning"
                                                          : "🟢 Healthy"}
                                                </span>
                                                <span
                                                    className={`days-text tone-${urgency === "critical" ? "red" : urgency === "warning" ? "amber" : "green"}`}
                                                >
                                                    {formatDaysRemaining(
                                                        cert.daysRemaining,
                                                    )}
                                                </span>
                                            </div>
                                        </td>

                                        {/* Actions: Trigger Certificate Renewal */}
                                        <td
                                            className="cell-actions text-right"
                                            onClick={(e) => e.stopPropagation()}
                                        >
                                            <div className="action-button-group">
                                                <button
                                                    type="button"
                                                    className={`btn-renew ${isRenewing ? "renewing" : ""} ${cert.renewalStatus === "renewed" ? "renewed" : ""}`}
                                                    onClick={(e) =>
                                                        handleTriggerRenewal(
                                                            e,
                                                            cert,
                                                        )
                                                    }
                                                    disabled={
                                                        isRenewing ||
                                                        cert.renewalStatus ===
                                                            "renewing"
                                                    }
                                                    title="Trigger certificate reissuance via cert-manager"
                                                    aria-label={`Trigger renewal for ${cert.hostname}`}
                                                >
                                                    {isRenewing ? (
                                                        <>
                                                            <span
                                                                className="spinner-icon"
                                                                aria-hidden="true"
                                                            />
                                                            <span>
                                                                Renewing...
                                                            </span>
                                                        </>
                                                    ) : cert.renewalStatus ===
                                                      "renewed" ? (
                                                        <>
                                                            <span
                                                                className="check-icon"
                                                                aria-hidden="true"
                                                            >
                                                                ✓
                                                            </span>
                                                            <span>Renewed</span>
                                                        </>
                                                    ) : (
                                                        <>
                                                            <span
                                                                className="refresh-icon"
                                                                aria-hidden="true"
                                                            >
                                                                ⟳
                                                            </span>
                                                            <span>
                                                                Trigger Renewal
                                                            </span>
                                                        </>
                                                    )}
                                                </button>
                                            </div>
                                        </td>
                                    </tr>
                                );
                            })
                        )}
                    </tbody>
                </table>
            </div>

            {/* Pagination Footer */}
            {showControls && sortedCertificates.length > 0 && (
                <div
                    className="cert-table-pagination"
                    role="navigation"
                    aria-label="Table pagination"
                >
                    <div className="pagination-info">
                        Showing{" "}
                        <strong>
                            {pageSize === 0
                                ? 1
                                : Math.min(
                                      (activePage - 1) * pageSize + 1,
                                      totalItems,
                                  )}
                        </strong>{" "}
                        to{" "}
                        <strong>
                            {pageSize === 0
                                ? totalItems
                                : Math.min(activePage * pageSize, totalItems)}
                        </strong>{" "}
                        of <strong>{totalItems}</strong> route certificates
                        {filteredCertificates.length !==
                            certificates.length && (
                            <span className="filter-count-note">
                                {" "}
                                (filtered from {certificates.length} total)
                            </span>
                        )}
                    </div>

                    <div className="pagination-controls">
                        <label className="page-size-selector">
                            <span>Per page:</span>
                            <select
                                value={pageSize}
                                onChange={(e) => {
                                    setPageSize(Number(e.target.value));
                                    setCurrentPage(1);
                                }}
                                aria-label="Rows per page"
                            >
                                <option value={10}>10</option>
                                <option value={15}>15</option>
                                <option value={25}>25</option>
                                <option value={50}>50</option>
                                <option value={0}>
                                    All ({certificates.length})
                                </option>
                            </select>
                        </label>

                        {pageSize > 0 && totalPages > 1 && (
                            <div className="pagination-nav">
                                <button
                                    type="button"
                                    className="page-nav-btn"
                                    onClick={() => setCurrentPage(1)}
                                    disabled={activePage === 1}
                                    aria-label="First page"
                                >
                                    «
                                </button>
                                <button
                                    type="button"
                                    className="page-nav-btn"
                                    onClick={() =>
                                        setCurrentPage((p) =>
                                            Math.max(1, p - 1),
                                        )
                                    }
                                    disabled={activePage === 1}
                                    aria-label="Previous page"
                                >
                                    ‹
                                </button>
                                <span className="page-current-text">
                                    Page {activePage} of {totalPages}
                                </span>
                                <button
                                    type="button"
                                    className="page-nav-btn"
                                    onClick={() =>
                                        setCurrentPage((p) =>
                                            Math.min(totalPages, p + 1),
                                        )
                                    }
                                    disabled={activePage === totalPages}
                                    aria-label="Next page"
                                >
                                    ›
                                </button>
                                <button
                                    type="button"
                                    className="page-nav-btn"
                                    onClick={() => setCurrentPage(totalPages)}
                                    disabled={activePage === totalPages}
                                    aria-label="Last page"
                                >
                                    »
                                </button>
                            </div>
                        )}
                    </div>
                </div>
            )}
        </div>
    );
};

export default CertTable;
