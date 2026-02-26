/**
 * Stellar-K8s Webhook Performance Benchmark
 * 
 * This k6 script measures latency (p99) and throughput for the
 * StellarNode Validation and Mutation webhooks to quantify Rust's
 * low-latency advantage in Kubernetes operators.
 * 
 * Usage:
 *   k6 run --out json=results/webhook-benchmark.json benchmarks/k6/webhook-load-test.js
 *   k6 run --env WEBHOOK_URL=https://webhook:8443 benchmarks/k6/webhook-load-test.js
 */

import http from 'k6/http';
import { check, sleep } from 'k6';
import { Counter, Rate, Trend } from 'k6/metrics';
import { randomString, randomIntBetween } from 'https://jslib.k6.io/k6-utils/1.2.0/index.js';

// =============================================================================
// Configuration
// =============================================================================

const WEBHOOK_URL = __ENV.WEBHOOK_URL || 'http://localhost:8443';
const VALIDATE_ENDPOINT = `${WEBHOOK_URL}/validate`;
const MUTATE_ENDPOINT = `${WEBHOOK_URL}/mutate`;

// Performance test scenarios
export const options = {
    scenarios: {
        // Baseline: Measure steady-state performance
        baseline_validation: {
            executor: 'constant-vus',
            vus: 10,
            duration: '1m',
            startTime: '0s',
            tags: { scenario: 'baseline_validation', webhook: 'validate' },
        },
        baseline_mutation: {
            executor: 'constant-vus',
            vus: 10,
            duration: '1m',
            startTime: '1m',
            tags: { scenario: 'baseline_mutation', webhook: 'mutate' },
        },

        // Stress test: 100+ concurrent requests
        stress_validation: {
            executor: 'ramping-vus',
            startVUs: 0,
            stages: [
                { duration: '30s', target: 50 },
                { duration: '1m', target: 100 },
                { duration: '1m', target: 150 },
                { duration: '30s', target: 0 },
            ],
            startTime: '2m',
            tags: { scenario: 'stress_validation', webhook: 'validate' },
        },
        stress_mutation: {
            executor: 'ramping-vus',
            startVUs: 0,
            stages: [
                { duration: '30s', target: 50 },
                { duration: '1m', target: 100 },
                { duration: '1m', target: 150 },
                { duration: '30s', target: 0 },
            ],
            startTime: '5m',
            tags: { scenario: 'stress_mutation', webhook: 'mutate' },
        },

        // Spike test: Sudden load burst
        spike_test: {
            executor: 'ramping-vus',
            startVUs: 0,
            stages: [
                { duration: '10s', target: 200 },
                { duration: '30s', target: 200 },
                { duration: '10s', target: 0 },
            ],
            startTime: '8m',
            tags: { scenario: 'spike', webhook: 'both' },
        },

        // Sustained high load
        sustained_load: {
            executor: 'constant-arrival-rate',
            rate: 100, // 100 requests per second
            timeUnit: '1s',
            duration: '2m',
            preAllocatedVUs: 200,
            startTime: '9m',
            tags: { scenario: 'sustained', webhook: 'both' },
        },
    },

    thresholds: {
        // Critical: p99 latency must be under 50ms for webhooks
        'http_req_duration{webhook:validate}': ['p(99)<50', 'p(95)<30'],
        'http_req_duration{webhook:mutate}': ['p(99)<50', 'p(95)<30'],

        // Throughput: Must handle 100+ req/s
        'webhook_throughput': ['rate>100'],

        // Error rate: Less than 0.1%
        'http_req_failed': ['rate<0.001'],

        // Specific webhook thresholds
        'validation_latency': ['p(99)<50', 'p(95)<30', 'avg<20'],
        'mutation_latency': ['p(99)<50', 'p(95)<30', 'avg<20'],

        // Regression detection
        'regression_check': ['rate>0.95'], // 95% within baseline
    },
};

// =============================================================================
// Custom Metrics
// =============================================================================

const validationRequests = new Counter('validation_requests');
const mutationRequests = new Counter('mutation_requests');
const webhookThroughput = new Rate('webhook_throughput');
const validationLatency = new Trend('validation_latency', true);
const mutationLatency = new Trend('mutation_latency', true);
const regressionCheck = new Rate('regression_check');

// =============================================================================
// Test Data
// =============================================================================

const nodeTypes = ['Validator', 'Horizon', 'SorobanRpc'];
const networks = ['Mainnet', 'Testnet', 'Futurenet'];
const operations = ['CREATE', 'UPDATE', 'DELETE'];

function generateAdmissionReview(operation, nodeType, includeOldObject = false) {
    const name = `test-${nodeType.toLowerCase()}-${randomString(8)}`;
    const namespace = 'benchmark';

    const stellarNode = {
        apiVersion: 'stellar.org/v1alpha1',
        kind: 'StellarNode',
        metadata: {
            name: name,
            namespace: namespace,
            uid: `uid-${randomString(16)}`,
            generation: 1,
        },
        spec: {
            nodeType: nodeType,
            network: networks[randomIntBetween(0, 2)],
            version: operation === 'CREATE' ? '' : 'v21.0.0', // Empty for mutation testing
            replicas: randomIntBetween(1, 3),
            resources: {
                requests: { cpu: '500m', memory: '1Gi' },
                limits: { cpu: '2', memory: '4Gi' },
            },
            storage: {
                storageClass: 'standard',
                size: '10Gi',
            },
        },
    };

    const admissionReview = {
        apiVersion: 'admission.k8s.io/v1',
        kind: 'AdmissionReview',
        request: {
            uid: `req-${randomString(16)}`,
            kind: { group: 'stellar.org', version: 'v1alpha1', kind: 'StellarNode' },
            resource: { group: 'stellar.org', version: 'v1alpha1', resource: 'stellarnodes' },
            requestKind: { group: 'stellar.org', version: 'v1alpha1', kind: 'StellarNode' },
            requestResource: { group: 'stellar.org', version: 'v1alpha1', resource: 'stellarnodes' },
            name: name,
            namespace: namespace,
            operation: operation,
            userInfo: {
                username: 'system:serviceaccount:kube-system:benchmark',
                uid: 'benchmark-uid',
                groups: ['system:serviceaccounts', 'system:authenticated'],
            },
            object: stellarNode,
            oldObject: includeOldObject ? stellarNode : null,
            dryRun: false,
        },
    };

    return admissionReview;
}

// Load baseline if available
let baseline = null;
if (__ENV.BASELINE_FILE) {
    try {
        baseline = JSON.parse(open(__ENV.BASELINE_FILE));
        console.log('Loaded baseline from:', __ENV.BASELINE_FILE);
    } catch (e) {
        console.log('No baseline found, running without regression detection');
    }
}

function checkRegression(metric, value, thresholdPercent = 10) {
    if (!baseline || !baseline.webhook_metrics || !baseline.webhook_metrics[metric]) {
        regressionCheck.add(1);
        return true;
    }

    const baselineValue = baseline.webhook_metrics[metric];
    const allowedIncrease = baselineValue * (1 + thresholdPercent / 100);
    const withinThreshold = value <= allowedIncrease;

    regressionCheck.add(withinThreshold ? 1 : 0);

    if (!withinThreshold) {
        console.warn(
            `REGRESSION: ${metric}=${value.toFixed(2)}ms ` +
            `(baseline=${baselineValue.toFixed(2)}ms, ` +
            `threshold=${allowedIncrease.toFixed(2)}ms)`
        );
    }

    return withinThreshold;
}

// =============================================================================
// Test Functions
// =============================================================================

export function setup() {
    console.log('='.repeat(70));
    console.log('  STELLAR-K8S WEBHOOK PERFORMANCE BENCHMARK');
    console.log('='.repeat(70));
    console.log(`Webhook URL: ${WEBHOOK_URL}`);
    console.log(`Baseline: ${__ENV.BASELINE_FILE || 'None'}`);
    console.log('');

    // Verify webhook is accessible
    const healthCheck = http.get(`${WEBHOOK_URL}/health`);
    if (healthCheck.status !== 200) {
        console.error('Webhook health check failed!');
        console.error(`Status: ${healthCheck.status}`);
        console.error(`Body: ${healthCheck.body}`);
    }

    return {
        startTime: new Date().toISOString(),
        runId: __ENV.RUN_ID || `local-${Date.now()}`,
    };
}

export default function () {
    const scenario = __ENV.SCENARIO || 'both';

    // Randomly choose operation type
    const operation = operations[randomIntBetween(0, 2)];
    const nodeType = nodeTypes[randomIntBetween(0, 2)];

    // Test validation webhook
    if (scenario === 'both' || scenario === 'validate') {
        testValidationWebhook(operation, nodeType);
    }

    // Test mutation webhook
    if (scenario === 'both' || scenario === 'mutate') {
        testMutationWebhook(operation, nodeType);
    }

    sleep(0.1); // Small delay between iterations
}

function testValidationWebhook(operation, nodeType) {
    const admissionReview = generateAdmissionReview(operation, nodeType, operation === 'UPDATE');

    const start = Date.now();
    const res = http.post(
        VALIDATE_ENDPOINT,
        JSON.stringify(admissionReview),
        {
            headers: { 'Content-Type': 'application/json' },
            tags: { webhook: 'validate', operation: operation, nodeType: nodeType },
        }
    );
    const duration = Date.now() - start;

    validationLatency.add(duration);
    validationRequests.add(1);
    webhookThroughput.add(1);

    const success = check(res, {
        'validation returns 200': (r) => r.status === 200,
        'validation has response': (r) => {
            try {
                const body = JSON.parse(r.body);
                return body.response !== undefined;
            } catch {
                return false;
            }
        },
        'validation within 50ms': (r) => duration < 50,
        'validation within 30ms (p95)': (r) => duration < 30,
    });

    if (!success) {
        console.error(`Validation failed: ${res.status} - ${res.body}`);
    }

    checkRegression('validation_p99', duration);
}

function testMutationWebhook(operation, nodeType) {
    const admissionReview = generateAdmissionReview(operation, nodeType, operation === 'UPDATE');

    const start = Date.now();
    const res = http.post(
        MUTATE_ENDPOINT,
        JSON.stringify(admissionReview),
        {
            headers: { 'Content-Type': 'application/json' },
            tags: { webhook: 'mutate', operation: operation, nodeType: nodeType },
        }
    );
    const duration = Date.now() - start;

    mutationLatency.add(duration);
    mutationRequests.add(1);
    webhookThroughput.add(1);

    const success = check(res, {
        'mutation returns 200': (r) => r.status === 200,
        'mutation has response': (r) => {
            try {
                const body = JSON.parse(r.body);
                return body.response !== undefined;
            } catch {
                return false;
            }
        },
        'mutation within 50ms': (r) => duration < 50,
        'mutation within 30ms (p95)': (r) => duration < 30,
    });

    if (!success) {
        console.error(`Mutation failed: ${res.status} - ${res.body}`);
    }

    // Check if patch was applied for CREATE operations
    if (operation === 'CREATE' && success) {
        try {
            const body = JSON.parse(res.body);
            if (body.response && body.response.patch) {
                check(body, {
                    'mutation applied patch': (b) => b.response.patch !== null,
                });
            }
        } catch (e) {
            // Ignore parse errors
        }
    }

    checkRegression('mutation_p99', duration);
}

export function teardown(data) {
    console.log('');
    console.log('='.repeat(70));
    console.log('  BENCHMARK COMPLETED');
    console.log('='.repeat(70));
    console.log(`Run ID: ${data.runId}`);
    console.log(`Duration: ${new Date().toISOString()} - ${data.startTime}`);
}

// =============================================================================
// Summary Handler
// =============================================================================

export function handleSummary(data) {
    const summary = {
        timestamp: new Date().toISOString(),
        runId: __ENV.RUN_ID || 'local',
        version: __ENV.VERSION || 'unknown',
        gitSha: __ENV.GIT_SHA || 'unknown',

        webhook_metrics: {
            // Validation webhook
            validation_avg: data.metrics.validation_latency?.values?.avg || 0,
            validation_p50: data.metrics.validation_latency?.values?.med || 0,
            validation_p95: data.metrics.validation_latency?.values['p(95)'] || 0,
            validation_p99: data.metrics.validation_latency?.values['p(99)'] || 0,
            validation_max: data.metrics.validation_latency?.values?.max || 0,
            validation_min: data.metrics.validation_latency?.values?.min || 0,

            // Mutation webhook
            mutation_avg: data.metrics.mutation_latency?.values?.avg || 0,
            mutation_p50: data.metrics.mutation_latency?.values?.med || 0,
            mutation_p95: data.metrics.mutation_latency?.values['p(95)'] || 0,
            mutation_p99: data.metrics.mutation_latency?.values['p(99)'] || 0,
            mutation_max: data.metrics.mutation_latency?.values?.max || 0,
            mutation_min: data.metrics.mutation_latency?.values?.min || 0,

            // Throughput
            throughput: data.metrics.webhook_throughput?.values?.rate || 0,
            total_requests: (data.metrics.validation_requests?.values?.count || 0) +
                (data.metrics.mutation_requests?.values?.count || 0),
            validation_requests: data.metrics.validation_requests?.values?.count || 0,
            mutation_requests: data.metrics.mutation_requests?.values?.count || 0,

            // Error rate
            error_rate: data.metrics.http_req_failed?.values?.rate || 0,
        },

        thresholds: data.thresholds || {},

        regression: {
            detected: (data.metrics.regression_check?.values?.rate || 1) < 0.95,
            passRate: data.metrics.regression_check?.values?.rate || 1,
            threshold: 0.95,
        },

        checks: {
            total: data.root_group?.checks?.length || 0,
            passed: data.root_group?.checks?.filter(c => c.passes > 0)?.length || 0,
            failed: data.root_group?.checks?.filter(c => c.fails > 0)?.length || 0,
        },
    };

    // Generate markdown report
    const markdown = generateMarkdownReport(summary, data);

    return {
        'stdout': textSummary(data),
        'results/webhook-benchmark.json': JSON.stringify(summary, null, 2),
        'results/webhook-benchmark-full.json': JSON.stringify(data, null, 2),
        'results/webhook-benchmark-report.md': markdown,
    };
}

function textSummary(data) {
    const lines = [];
    lines.push('');
    lines.push('='.repeat(70));
    lines.push('  WEBHOOK PERFORMANCE RESULTS');
    lines.push('='.repeat(70));
    lines.push('');

    // Validation metrics
    lines.push('🔍 VALIDATION WEBHOOK');
    lines.push('-'.repeat(40));
    const valLatency = data.metrics.validation_latency?.values || {};
    lines.push(`  Average:    ${(valLatency.avg || 0).toFixed(2)} ms`);
    lines.push(`  p50:        ${(valLatency.med || 0).toFixed(2)} ms`);
    lines.push(`  p95:        ${(valLatency['p(95)'] || 0).toFixed(2)} ms`);
    lines.push(`  p99:        ${(valLatency['p(99)'] || 0).toFixed(2)} ms`);
    lines.push(`  Max:        ${(valLatency.max || 0).toFixed(2)} ms`);
    lines.push(`  Requests:   ${data.metrics.validation_requests?.values?.count || 0}`);
    lines.push('');

    // Mutation metrics
    lines.push('✏️  MUTATION WEBHOOK');
    lines.push('-'.repeat(40));
    const mutLatency = data.metrics.mutation_latency?.values || {};
    lines.push(`  Average:    ${(mutLatency.avg || 0).toFixed(2)} ms`);
    lines.push(`  p50:        ${(mutLatency.med || 0).toFixed(2)} ms`);
    lines.push(`  p95:        ${(mutLatency['p(95)'] || 0).toFixed(2)} ms`);
    lines.push(`  p99:        ${(mutLatency['p(99)'] || 0).toFixed(2)} ms`);
    lines.push(`  Max:        ${(mutLatency.max || 0).toFixed(2)} ms`);
    lines.push(`  Requests:   ${data.metrics.mutation_requests?.values?.count || 0}`);
    lines.push('');

    // Throughput
    lines.push('📊 THROUGHPUT');
    lines.push('-'.repeat(40));
    lines.push(`  Rate:       ${(data.metrics.webhook_throughput?.values?.rate || 0).toFixed(2)} req/s`);
    lines.push(`  Total:      ${((data.metrics.validation_requests?.values?.count || 0) + (data.metrics.mutation_requests?.values?.count || 0))}`);
    lines.push(`  Errors:     ${((data.metrics.http_req_failed?.values?.rate || 0) * 100).toFixed(3)}%`);
    lines.push('');

    // Thresholds
    lines.push('🎯 THRESHOLDS');
    lines.push('-'.repeat(40));
    for (const [name, result] of Object.entries(data.thresholds || {})) {
        const status = result.ok ? '✅' : '❌';
        lines.push(`  ${status} ${name}`);
    }
    lines.push('');

    // Regression
    const regressionRate = data.metrics.regression_check?.values?.rate || 1;
    const regressionStatus = regressionRate >= 0.95 ? '✅' : '❌';
    lines.push(`${regressionStatus} REGRESSION: ${(regressionRate * 100).toFixed(1)}% within baseline`);
    lines.push('');

    lines.push('='.repeat(70));

    return lines.join('\n');
}

function generateMarkdownReport(summary, fullData) {
    const lines = [];

    lines.push('# Webhook Performance Benchmark Report');
    lines.push('');
    lines.push(`**Generated:** ${summary.timestamp}`);
    lines.push(`**Run ID:** ${summary.runId}`);
    lines.push(`**Version:** ${summary.version}`);
    lines.push(`**Git SHA:** ${summary.gitSha}`);
    lines.push('');

    // Executive Summary
    lines.push('## Executive Summary');
    lines.push('');
    const valP99 = summary.webhook_metrics.validation_p99;
    const mutP99 = summary.webhook_metrics.mutation_p99;
    const throughput = summary.webhook_metrics.throughput;
    const errorRate = summary.webhook_metrics.error_rate * 100;

    lines.push(`- **Validation p99 Latency:** ${valP99.toFixed(2)} ms`);
    lines.push(`- **Mutation p99 Latency:** ${mutP99.toFixed(2)} ms`);
    lines.push(`- **Throughput:** ${throughput.toFixed(2)} req/s`);
    lines.push(`- **Error Rate:** ${errorRate.toFixed(3)}%`);
    lines.push(`- **Total Requests:** ${summary.webhook_metrics.total_requests}`);
    lines.push('');

    // Threshold Results
    lines.push('## Threshold Results');
    lines.push('');
    lines.push('| Threshold | Status | Result |');
    lines.push('|-----------|--------|--------|');
    for (const [name, result] of Object.entries(summary.thresholds)) {
        const status = result.ok ? '✅ PASS' : '❌ FAIL';
        lines.push(`| ${name} | ${status} | - |`);
    }
    lines.push('');

    // Validation Webhook Metrics
    lines.push('## Validation Webhook Performance');
    lines.push('');
    lines.push('| Metric | Value |');
    lines.push('|--------|-------|');
    lines.push(`| Average | ${summary.webhook_metrics.validation_avg.toFixed(2)} ms |`);
    lines.push(`| p50 (Median) | ${summary.webhook_metrics.validation_p50.toFixed(2)} ms |`);
    lines.push(`| p95 | ${summary.webhook_metrics.validation_p95.toFixed(2)} ms |`);
    lines.push(`| p99 | ${summary.webhook_metrics.validation_p99.toFixed(2)} ms |`);
    lines.push(`| Max | ${summary.webhook_metrics.validation_max.toFixed(2)} ms |`);
    lines.push(`| Min | ${summary.webhook_metrics.validation_min.toFixed(2)} ms |`);
    lines.push(`| Total Requests | ${summary.webhook_metrics.validation_requests} |`);
    lines.push('');

    // Mutation Webhook Metrics
    lines.push('## Mutation Webhook Performance');
    lines.push('');
    lines.push('| Metric | Value |');
    lines.push('|--------|-------|');
    lines.push(`| Average | ${summary.webhook_metrics.mutation_avg.toFixed(2)} ms |`);
    lines.push(`| p50 (Median) | ${summary.webhook_metrics.mutation_p50.toFixed(2)} ms |`);
    lines.push(`| p95 | ${summary.webhook_metrics.mutation_p95.toFixed(2)} ms |`);
    lines.push(`| p99 | ${summary.webhook_metrics.mutation_p99.toFixed(2)} ms |`);
    lines.push(`| Max | ${summary.webhook_metrics.mutation_max.toFixed(2)} ms |`);
    lines.push(`| Min | ${summary.webhook_metrics.mutation_min.toFixed(2)} ms |`);
    lines.push(`| Total Requests | ${summary.webhook_metrics.mutation_requests} |`);
    lines.push('');

    // Regression Analysis
    lines.push('## Regression Analysis');
    lines.push('');
    if (summary.regression.detected) {
        lines.push('⚠️ **REGRESSION DETECTED**');
        lines.push('');
        lines.push(`Only ${(summary.regression.passRate * 100).toFixed(1)}% of requests were within baseline thresholds.`);
    } else {
        lines.push('✅ **NO REGRESSION DETECTED**');
        lines.push('');
        lines.push(`${(summary.regression.passRate * 100).toFixed(1)}% of requests were within baseline thresholds.`);
    }
    lines.push('');

    // Comparison with Baseline
    if (__ENV.BASELINE_FILE) {
        lines.push('## Baseline Comparison');
        lines.push('');
        lines.push('Performance comparison against baseline metrics.');
        lines.push('');
        // This would be populated if baseline data is available
    }

    // Checks Summary
    lines.push('## Test Checks');
    lines.push('');
    lines.push(`- **Total Checks:** ${summary.checks.total}`);
    lines.push(`- **Passed:** ${summary.checks.passed}`);
    lines.push(`- **Failed:** ${summary.checks.failed}`);
    lines.push('');

    // Conclusion
    lines.push('## Conclusion');
    lines.push('');
    const allPassed = Object.values(summary.thresholds).every(t => t.ok);
    if (allPassed && !summary.regression.detected) {
        lines.push('✅ All performance thresholds met. Webhook latency is within acceptable limits.');
    } else {
        lines.push('❌ Some performance thresholds were not met. Review the results above.');
    }
    lines.push('');

    return lines.join('\n');
}
