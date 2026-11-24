export type CertUrgency = "critical" | "warning" | "healthy";

export type RouteType = "Ingress" | "HTTPRoute";

export type RenewalStatus = "idle" | "renewing" | "renewed" | "failed";

export interface CertificateInfo {
    id: string;
    name: string;
    namespace: string;
    cluster: string;
    routeType: RouteType;
    hostname: string;
    serviceEndpoint: string;
    issuer: string;
    issuerType: string;
    sans: string[];
    notBefore: string;
    notAfter: string;
    daysRemaining: number;
    urgency: CertUrgency;
    isExpired: boolean;
    serialNumber: string;
    signatureAlgorithm: string;
    autoRenewal: boolean;
    secretName: string;
    renewalStatus: RenewalStatus;
    lastRenewalAttempt?: string;
    annotations?: Record<string, string>;
}

export type SortField =
    | "hostname"
    | "name"
    | "namespace"
    | "cluster"
    | "routeType"
    | "issuer"
    | "notAfter"
    | "daysRemaining"
    | "renewalStatus";

export type SortDirection = "asc" | "desc";

export type UrgencyFilter =
    "all" | "critical" | "warning" | "healthy" | "expired";

export interface FilterState {
    search: string;
    urgency: UrgencyFilter;
    routeType: "all" | RouteType;
    namespace: string;
}

export interface MetricSummary {
    total: number;
    critical: number;
    expired: number;
    warning: number;
    healthy: number;
    autoRenewalCount: number;
    autoRenewalPercentage: number;
}
