import test from "node:test";
import assert from "node:assert/strict";
import {
    calculateDaysRemaining,
    getCertUrgency,
    formatDaysRemaining,
    formatDate,
    calculateMetrics,
    filterCertificates,
    sortCertificates,
} from "./certUtils.ts";
import type { CertificateInfo } from "./types.ts";

// Sample Reference Date: 2026-09-01T00:00:00.000Z
const REF_DATE = new Date("2026-09-01T00:00:00.000Z");

test("calculateDaysRemaining calculates accurate positive, zero, and negative day counts", () => {
    // 10 days in the future
    const futureDate = new Date("2026-09-11T00:00:00.000Z");
    assert.equal(calculateDaysRemaining(futureDate, REF_DATE), 10);

    // Exact same day (0 days)
    assert.equal(
        calculateDaysRemaining(new Date("2026-09-01T12:00:00.000Z"), REF_DATE),
        0,
    );

    // Expired 5 days ago
    const pastDate = new Date("2026-08-27T00:00:00.000Z");
    assert.equal(calculateDaysRemaining(pastDate, REF_DATE), -5);

    // String format handling
    assert.equal(
        calculateDaysRemaining("2026-09-05T00:00:00.000Z", REF_DATE),
        4,
    );
});

test("getCertUrgency accurately classifies urgency tiers per specification", () => {
    // Expired (negative days) -> Critical (Red)
    assert.equal(getCertUrgency(-3), "critical");
    assert.equal(getCertUrgency(0), "critical");

    // Critical (Red): < 7 days
    assert.equal(getCertUrgency(1), "critical");
    assert.equal(getCertUrgency(6), "critical");

    // Warning (Yellow / Amber): 7 to < 30 days
    assert.equal(getCertUrgency(7), "warning");
    assert.equal(getCertUrgency(15), "warning");
    assert.equal(getCertUrgency(29), "warning");

    // Healthy (Green): >= 30 days
    assert.equal(getCertUrgency(30), "healthy");
    assert.equal(getCertUrgency(90), "healthy");
    assert.equal(getCertUrgency(365), "healthy");
});

test("formatDaysRemaining returns intuitive human-readable strings", () => {
    assert.equal(formatDaysRemaining(-5), "Expired 5d ago");
    assert.equal(formatDaysRemaining(0), "Expires today");
    assert.equal(formatDaysRemaining(1), "1 day remaining");
    assert.equal(formatDaysRemaining(14), "14 days remaining");
    assert.equal(formatDaysRemaining(180), "180 days remaining");
});

test("formatDate correctly outputs standardized UTC dates", () => {
    const formatted = formatDate("2026-09-15T14:30:00.000Z");
    assert.equal(formatted, "2026-09-15 14:30 UTC");
});

test("calculateMetrics calculates aggregate stats, critical counts, and auto-renewal coverage", () => {
    const sampleCerts: CertificateInfo[] = [
        {
            id: "1",
            name: "c1",
            namespace: "ns",
            cluster: "cl",
            routeType: "Ingress",
            hostname: "h1.com",
            serviceEndpoint: "svc:80",
            issuer: "CA1",
            issuerType: "Let's Encrypt",
            sans: [],
            notBefore: "2026-01-01T00:00:00Z",
            notAfter: "2026-08-30T00:00:00Z",
            daysRemaining: -2,
            isExpired: true,
            urgency: "critical",
            serialNumber: "1",
            signatureAlgorithm: "RSA",
            autoRenewal: false,
            secretName: "s1",
            renewalStatus: "idle",
        },
        {
            id: "2",
            name: "c2",
            namespace: "ns",
            cluster: "cl",
            routeType: "Ingress",
            hostname: "h2.com",
            serviceEndpoint: "svc:80",
            issuer: "CA1",
            issuerType: "Let's Encrypt",
            sans: [],
            notBefore: "2026-01-01T00:00:00Z",
            notAfter: "2026-09-04T00:00:00Z",
            daysRemaining: 3,
            isExpired: false,
            urgency: "critical",
            serialNumber: "2",
            signatureAlgorithm: "RSA",
            autoRenewal: true,
            secretName: "s2",
            renewalStatus: "idle",
        },
        {
            id: "3",
            name: "c3",
            namespace: "ns",
            cluster: "cl",
            routeType: "HTTPRoute",
            hostname: "h3.com",
            serviceEndpoint: "svc:80",
            issuer: "CA1",
            issuerType: "Let's Encrypt",
            sans: [],
            notBefore: "2026-01-01T00:00:00Z",
            notAfter: "2026-09-16T00:00:00Z",
            daysRemaining: 15,
            isExpired: false,
            urgency: "warning",
            serialNumber: "3",
            signatureAlgorithm: "RSA",
            autoRenewal: true,
            secretName: "s3",
            renewalStatus: "idle",
        },
        {
            id: "4",
            name: "c4",
            namespace: "ns",
            cluster: "cl",
            routeType: "HTTPRoute",
            hostname: "h4.com",
            serviceEndpoint: "svc:80",
            issuer: "CA1",
            issuerType: "Let's Encrypt",
            sans: [],
            notBefore: "2026-01-01T00:00:00Z",
            notAfter: "2026-11-01T00:00:00Z",
            daysRemaining: 60,
            isExpired: false,
            urgency: "healthy",
            serialNumber: "4",
            signatureAlgorithm: "RSA",
            autoRenewal: true,
            secretName: "s4",
            renewalStatus: "idle",
        },
    ];

    const metrics = calculateMetrics(sampleCerts);
    assert.equal(metrics.total, 4);
    assert.equal(metrics.critical, 2);
    assert.equal(metrics.expired, 1);
    assert.equal(metrics.warning, 1);
    assert.equal(metrics.healthy, 1);
    assert.equal(metrics.autoRenewalCount, 3);
    assert.equal(metrics.autoRenewalPercentage, 75);
});

test("filterCertificates filters by full-text search across hostname, issuer, and SANs", () => {
    const certs: CertificateInfo[] = [
        {
            id: "1",
            hostname: "horizon.stellar.org",
            name: "horizon-ingress",
            namespace: "stellar-mainnet",
            cluster: "us-east-prod",
            issuer: "Let's Encrypt Authority X3",
            issuerType: "Let's Encrypt",
            secretName: "horizon-tls",
            serviceEndpoint: "horizon:8000",
            sans: ["horizon.stellar.org", "api.horizon.stellar.org"],
            routeType: "Ingress",
            urgency: "critical",
            daysRemaining: 2,
            isExpired: false,
            notBefore: "2026-06-01T00:00:00Z",
            notAfter: "2026-09-03T00:00:00Z",
            serialNumber: "1",
            signatureAlgorithm: "ECDSA-SHA256",
            autoRenewal: false,
            renewalStatus: "idle",
        },
        {
            id: "2",
            hostname: "soroban-rpc.stellar.org",
            name: "soroban-route",
            namespace: "stellar-mainnet",
            cluster: "us-east-prod",
            issuer: "DigiCert TLS RSA",
            issuerType: "DigiCert",
            secretName: "soroban-tls",
            serviceEndpoint: "soroban:8000",
            sans: ["soroban-rpc.stellar.org", "rpc.soroban.stellar.org"],
            routeType: "HTTPRoute",
            urgency: "healthy",
            daysRemaining: 90,
            isExpired: false,
            notBefore: "2026-06-01T00:00:00Z",
            notAfter: "2026-12-01T00:00:00Z",
            serialNumber: "2",
            signatureAlgorithm: "RSA-4096",
            autoRenewal: true,
            renewalStatus: "idle",
        },
        {
            id: "3",
            hostname: "testnet.stellar.org",
            name: "testnet-ingress",
            namespace: "stellar-testnet",
            cluster: "us-west-testnet",
            issuer: "ZeroSSL CA",
            issuerType: "ZeroSSL",
            secretName: "testnet-tls",
            serviceEndpoint: "testnet:8000",
            sans: ["testnet.stellar.org"],
            routeType: "Ingress",
            urgency: "warning",
            daysRemaining: 15,
            isExpired: false,
            notBefore: "2026-06-01T00:00:00Z",
            notAfter: "2026-09-16T00:00:00Z",
            serialNumber: "3",
            signatureAlgorithm: "RSA-2048",
            autoRenewal: true,
            renewalStatus: "idle",
        },
    ];

    // Search by SAN
    const sanResult = filterCertificates(certs, {
        search: "rpc.soroban",
        urgency: "all",
        routeType: "all",
        namespace: "all",
    });
    assert.equal(sanResult.length, 1);
    assert.equal(sanResult[0].id, "2");

    // Search by issuer
    const issuerResult = filterCertificates(certs, {
        search: "ZeroSSL",
        urgency: "all",
        routeType: "all",
        namespace: "all",
    });
    assert.equal(issuerResult.length, 1);
    assert.equal(issuerResult[0].id, "3");

    // Filter by routeType
    const httpRouteResult = filterCertificates(certs, {
        search: "",
        urgency: "all",
        routeType: "HTTPRoute",
        namespace: "all",
    });
    assert.equal(httpRouteResult.length, 1);
    assert.equal(httpRouteResult[0].hostname, "soroban-rpc.stellar.org");

    // Filter by urgency
    const criticalResult = filterCertificates(certs, {
        search: "",
        urgency: "critical",
        routeType: "all",
        namespace: "all",
    });
    assert.equal(criticalResult.length, 1);
    assert.equal(criticalResult[0].hostname, "horizon.stellar.org");
});

test("sortCertificates prioritizes expiring certificates at top by default", () => {
    const certs: CertificateInfo[] = [
        {
            id: "1",
            hostname: "c.stellar.org",
            name: "c",
            namespace: "stellar-mainnet",
            cluster: "us-1",
            routeType: "Ingress",
            issuer: "Issuer A",
            issuerType: "CA",
            sans: [],
            notBefore: "2026-01-01",
            notAfter: "2026-12-01T00:00:00Z",
            daysRemaining: 90,
            urgency: "healthy",
            isExpired: false,
            serialNumber: "1",
            signatureAlgorithm: "RSA",
            autoRenewal: true,
            secretName: "s1",
            serviceEndpoint: "svc:80",
            renewalStatus: "idle",
        },
        {
            id: "2",
            hostname: "a.stellar.org",
            name: "a",
            namespace: "stellar-mainnet",
            cluster: "us-1",
            routeType: "Ingress",
            issuer: "Issuer B",
            issuerType: "CA",
            sans: [],
            notBefore: "2026-01-01",
            notAfter: "2026-08-25T00:00:00Z",
            daysRemaining: -5,
            urgency: "critical",
            isExpired: true,
            serialNumber: "2",
            signatureAlgorithm: "RSA",
            autoRenewal: true,
            secretName: "s2",
            serviceEndpoint: "svc:80",
            renewalStatus: "idle",
        },
        {
            id: "3",
            hostname: "b.stellar.org",
            name: "b",
            namespace: "stellar-mainnet",
            cluster: "us-1",
            routeType: "Ingress",
            issuer: "Issuer C",
            issuerType: "CA",
            sans: [],
            notBefore: "2026-01-01",
            notAfter: "2026-09-04T00:00:00Z",
            daysRemaining: 3,
            urgency: "critical",
            isExpired: false,
            serialNumber: "3",
            signatureAlgorithm: "RSA",
            autoRenewal: true,
            secretName: "s3",
            serviceEndpoint: "svc:80",
            renewalStatus: "idle",
        },
    ];

    const sortedAsc = sortCertificates(certs, "daysRemaining", "asc");
    // Order must be: id: 2 (-5 days), id: 3 (3 days), id: 1 (90 days)
    assert.equal(sortedAsc[0].id, "2");
    assert.equal(sortedAsc[1].id, "3");
    assert.equal(sortedAsc[2].id, "1");

    // Descending sort
    const sortedDesc = sortCertificates(certs, "daysRemaining", "desc");
    assert.equal(sortedDesc[0].id, "1");
    assert.equal(sortedDesc[1].id, "3");
    assert.equal(sortedDesc[2].id, "2");

    // Alphabetical sort by hostname
    const sortedHost = sortCertificates(certs, "hostname", "asc");
    assert.equal(sortedHost[0].hostname, "a.stellar.org");
    assert.equal(sortedHost[1].hostname, "b.stellar.org");
    assert.equal(sortedHost[2].hostname, "c.stellar.org");
});
