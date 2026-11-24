import type {
    CertificateInfo,
    CertUrgency,
    FilterState,
    MetricSummary,
    SortDirection,
    SortField,
} from "./types.ts";

export const MS_PER_DAY = 1000 * 60 * 60 * 24;

/**
 * Calculates integer days remaining until a certificate expires.
 * Positive numbers indicate days until expiration.
 * Negative numbers indicate days since expiration.
 */
export function calculateDaysRemaining(
    notAfter: string | Date,
    referenceDate: Date = new Date(),
): number {
    const expiry = typeof notAfter === "string" ? new Date(notAfter) : notAfter;
    const now = referenceDate;
    const diffMs = expiry.getTime() - now.getTime();
    return Math.floor(diffMs / MS_PER_DAY);
}

/**
 * Classifies certificate urgency based on days remaining:
 * - Red (Critical): < 7 days or already expired
 * - Yellow / Amber (Warning): 7 to < 30 days
 * - Green (Healthy): >= 30 days
 */
export function getCertUrgency(daysRemaining: number): CertUrgency {
    if (daysRemaining < 7) {
        return "critical";
    }
    if (daysRemaining < 30) {
        return "warning";
    }
    return "healthy";
}

/**
 * Formats days remaining into human-readable text.
 */
export function formatDaysRemaining(daysRemaining: number): string {
    if (daysRemaining < 0) {
        const daysAgo = Math.abs(daysRemaining);
        return `Expired ${daysAgo}d ago`;
    }
    if (daysRemaining === 0) {
        return "Expires today";
    }
    if (daysRemaining === 1) {
        return "1 day remaining";
    }
    return `${daysRemaining} days remaining`;
}

/**
 * Formats ISO date into readable UTC format: YYYY-MM-DD HH:mm UTC
 */
export function formatDate(isoString: string): string {
    try {
        const d = new Date(isoString);
        if (Number.isNaN(d.getTime())) return isoString;
        const year = d.getUTCFullYear();
        const month = String(d.getUTCMonth() + 1).padStart(2, "0");
        const day = String(d.getUTCDate()).padStart(2, "0");
        const hours = String(d.getUTCHours()).padStart(2, "0");
        const minutes = String(d.getUTCMinutes()).padStart(2, "0");
        return `${year}-${month}-${day} ${hours}:${minutes} UTC`;
    } catch {
        return isoString;
    }
}

/**
 * Computes aggregate summary metrics for the KPI metric strip.
 */
export function calculateMetrics(
    certificates: CertificateInfo[],
): MetricSummary {
    const total = certificates.length;
    let critical = 0;
    let expired = 0;
    let warning = 0;
    let healthy = 0;
    let autoRenewalCount = 0;

    for (const cert of certificates) {
        if (cert.isExpired || cert.daysRemaining <= 0) {
            expired++;
        }
        if (cert.urgency === "critical") {
            critical++;
        } else if (cert.urgency === "warning") {
            warning++;
        } else {
            healthy++;
        }

        if (cert.autoRenewal) {
            autoRenewalCount++;
        }
    }

    const autoRenewalPercentage =
        total > 0 ? Math.round((autoRenewalCount / total) * 100) : 0;

    return {
        total,
        critical,
        expired,
        warning,
        healthy,
        autoRenewalCount,
        autoRenewalPercentage,
    };
}

/**
 * Filters certificate list by search keyword, urgency status, route type, and namespace.
 */
export function filterCertificates(
    certificates: CertificateInfo[],
    filter: FilterState,
): CertificateInfo[] {
    const query = filter.search.trim().toLowerCase();

    return certificates.filter((cert) => {
        // 1. Urgency filter
        if (filter.urgency !== "all") {
            if (filter.urgency === "expired") {
                if (!cert.isExpired && cert.daysRemaining > 0) return false;
            } else if (cert.urgency !== filter.urgency) {
                return false;
            }
        }

        // 2. Route type filter
        if (filter.routeType !== "all" && cert.routeType !== filter.routeType) {
            return false;
        }

        // 3. Namespace filter
        if (
            filter.namespace &&
            filter.namespace !== "all" &&
            cert.namespace !== filter.namespace
        ) {
            return false;
        }

        // 4. Text search
        if (query) {
            const matchHostname = cert.hostname.toLowerCase().includes(query);
            const matchName = cert.name.toLowerCase().includes(query);
            const matchNamespace = cert.namespace.toLowerCase().includes(query);
            const matchIssuer = cert.issuer.toLowerCase().includes(query);
            const matchSecret = cert.secretName.toLowerCase().includes(query);
            const matchEndpoint = cert.serviceEndpoint
                .toLowerCase()
                .includes(query);
            const matchCluster = cert.cluster.toLowerCase().includes(query);
            const matchSans = cert.sans.some((san) =>
                san.toLowerCase().includes(query),
            );

            if (
                !matchHostname &&
                !matchName &&
                !matchNamespace &&
                !matchIssuer &&
                !matchSecret &&
                !matchEndpoint &&
                !matchCluster &&
                !matchSans
            ) {
                return false;
            }
        }

        return true;
    });
}

/**
 * Sorts certificate list by a selected field and direction.
 * Prioritizes expiring certificates at the top when sorting by daysRemaining ascending (default).
 */
export function sortCertificates(
    certificates: CertificateInfo[],
    field: SortField,
    direction: SortDirection = "asc",
): CertificateInfo[] {
    const modifier = direction === "asc" ? 1 : -1;

    return [...certificates].sort((a, b) => {
        let comparison = 0;

        switch (field) {
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

        // Stable tie-breaker: daysRemaining ASC then hostname ASC
        const tieBreakDays = a.daysRemaining - b.daysRemaining;
        if (tieBreakDays !== 0) return tieBreakDays;
        return a.hostname.localeCompare(b.hostname);
    });
}
