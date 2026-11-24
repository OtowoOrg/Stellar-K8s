import test from "node:test";
import assert from "node:assert/strict";
import { generateMockCertificates } from "../monitors/ingress/src/mockData.ts";
import {
    calculateDaysRemaining,
    calculateMetrics,
    filterCertificates,
    getCertUrgency,
    sortCertificates,
} from "../monitors/ingress/src/certUtils.ts";

test("mock dataset contains 50+ exposed cluster routes across Ingress and HTTPRoute types", () => {
    const fleet = generateMockCertificates();
    assert.ok(
        fleet.length >= 50,
        `Expected 50+ certificates, got ${fleet.length}`,
    );

    const ingressCount = fleet.filter((c) => c.routeType === "Ingress").length;
    const httpRouteCount = fleet.filter(
        (c) => c.routeType === "HTTPRoute",
    ).length;

    assert.ok(ingressCount > 0, "Must have Ingress endpoints");
    assert.ok(httpRouteCount > 0, "Must have HTTPRoute endpoints");
    assert.equal(ingressCount + httpRouteCount, fleet.length);
});

test("mock dataset contains expired, < 7d, < 30d, and > 30d certificates", () => {
    const fleet = generateMockCertificates();

    const expired = fleet.filter((c) => c.isExpired || c.daysRemaining <= 0);
    const critical = fleet.filter((c) => c.urgency === "critical");
    const warning = fleet.filter((c) => c.urgency === "warning");
    const healthy = fleet.filter((c) => c.urgency === "healthy");

    assert.ok(expired.length > 0, "Must have at least one expired certificate");
    assert.ok(
        critical.length > expired.length,
        "Must have near-expiry critical certs (<7d)",
    );
    assert.ok(warning.length > 0, "Must have warning certs (7-30d)");
    assert.ok(healthy.length > 0, "Must have healthy certs (>30d)");
});

test("default table sort orders fleet with most critical & expired at the very top", () => {
    const fleet = generateMockCertificates();
    const sorted = sortCertificates(fleet, "daysRemaining", "asc");

    // Verify first items have smallest / negative days remaining
    for (let i = 1; i < sorted.length; i++) {
        assert.ok(
            sorted[i].daysRemaining >= sorted[i - 1].daysRemaining,
            `Item at index ${i} (${sorted[i].daysRemaining}d) is not sorted after index ${i - 1} (${sorted[i - 1].daysRemaining}d)`,
        );
    }

    // First item must be expired or critical (<7d)
    assert.equal(sorted[0].urgency, "critical");
});

test("search filtering processes 50+ routes within < 5ms for 60fps responsiveness", () => {
    const fleet = generateMockCertificates();
    const start = performance.now();

    for (let i = 0; i < 100; i++) {
        filterCertificates(fleet, {
            search: "soroban",
            urgency: "all",
            routeType: "all",
            namespace: "all",
        });
    }

    const duration = performance.now() - start;
    const avgPerFilter = duration / 100;
    assert.ok(
        avgPerFilter < 5,
        `Filtering took ${avgPerFilter.toFixed(2)}ms per call, expected < 5ms`,
    );
});

test("renewal simulation updates certificate status to healthy with extended validity", () => {
    const fleet = generateMockCertificates();
    const target = fleet.find((c) => c.urgency === "critical");
    assert.ok(target, "Should find a critical certificate");

    // Simulate renewal
    const now = new Date();
    const newExpiry = new Date(now.getTime() + 90 * 24 * 60 * 60 * 1000);
    const renewedTarget = {
        ...target,
        renewalStatus: "renewed" as const,
        isExpired: false,
        daysRemaining: calculateDaysRemaining(newExpiry, now),
        urgency: getCertUrgency(90),
        notBefore: now.toISOString(),
        notAfter: newExpiry.toISOString(),
    };

    assert.equal(renewedTarget.renewalStatus, "renewed");
    assert.equal(renewedTarget.isExpired, false);
    assert.equal(renewedTarget.urgency, "healthy");
    assert.ok(renewedTarget.daysRemaining >= 89);
});
