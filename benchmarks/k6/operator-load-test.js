/**
 * Stellar-K8s Operator Performance Load Test
 * 
 * This k6 script measures TPS, latency, and resource consumption
 * for the Stellar operator's API endpoints and reconciliation loops.
 * 
 * Usage:
 *   k6 run --out json=results.json operator-load-test.js
 *   k6 run --env BASE_URL=http://localhost:8080 operator-load-test.js
 */

import http from 'k6/http';
import { check, sleep, group } from 'k6';
import { Counter, Rate, Trend, Gauge } from 'k6/metrics';
import { randomString, randomIntBetween } from 'https://jslib.k6.io/k6-utils/1.2.0/index.js';

// =============================================================================
// Configuration
// =============================================================================

const BASE_URL = __ENV.BASE_URL || 'http://localhost:8080';
const K8S_API_URL = __ENV.K8S_API_URL || 'http://localhost:8001';
const NAMESPACE = __ENV.NAMESPACE || 'stellar-benchmark';

// Performance thresholds - CI/CD will fail if these are exceeded
export const options = {
    scenarios: {
        // Scenario 1: Steady state load for baseline measurement
        steady_state: {
            executor: 'constant-vus',
            vus: 10,
            duration: '2m',
            startTime: '0s',
            tags: { scenario: 'steady_state' },
        },
        // Scenario 2: Stress test with ramping VUs
        stress_test: {
            executor: 'ramping-vus',
            startVUs: 0,
            stages: [
                { duration: '30s', target: 20 },
                { duration: '1m', target: 50 },
                { duration: '30s', target: 100 },
                { duration: '1m', target: 100 },
                { duration: '30s', target: 0 },
            ],
            startTime: '2m',
            tags: { scenario: 'stress_test' },
        },
        // Scenario 3: Spike test for sudden load
        spike_test: {
            executor: 'ramping-vus',
            startVUs: 0,
            stages: [
                { duration: '10s', target: 200 },
                { duration: '30s', target: 200 },
                { duration: '10s', target: 0 },
            ],
            startTime: '6m',
            tags: { scenario: 'spike_test' },
        },
        // Scenario 4: Reconciliation throughput test
        reconciliation_load: {
            executor: 'constant-arrival-rate',
            rate: 50, // 50 reconciliations per second
            timeUnit: '1s',
            duration: '2m',
            preAllocatedVUs: 100,
            startTime: '7m',
            tags: { scenario: 'reconciliation' },
        },
    },
    thresholds: {
        // Response time thresholds
        http_req_duration: ['p(95)<500', 'p(99)<1000'],
        'http_req_duration{endpoint:health}': ['p(95)<100'],
        'http_req_duration{endpoint:metrics}': ['p(95)<200'],
        'http_req_duration{endpoint:reconcile}': ['p(95)<2000'],
        
        // Error rate thresholds
        http_req_failed: ['rate<0.01'], // Less than 1% errors
        
        // Custom thresholds
        reconciliation_duration: ['p(95)<3000', 'p(99)<5000'],
        api_latency: ['p(95)<200', 'p(99)<500'],
        tps: ['rate>100'], // Minimum 100 TPS
        
        // Regression detection (10% threshold)
        'regression_check': ['rate>0.9'], // 90% of requests within baseline
    },
};

// =============================================================================
// Custom Metrics
// =============================================================================

// Counters
const totalRequests = new Counter('total_requests');
const reconciliationCount = new Counter('reconciliation_count');
const nodeCreations = new Counter('node_creations');
const nodeUpdates = new Counter('node_updates');
const nodeDeletions = new Counter('node_deletions');

// Rates
const tps = new Rate('tps');
const errorRate = new Rate('error_rate');
const regressionCheck = new Rate('regression_check');

// Trends (latency distributions)
const reconciliationDuration = new Trend('reconciliation_duration');
const apiLatency = new Trend('api_latency');
const healthCheckLatency = new Trend('health_check_latency');
const metricsLatency = new Trend('metrics_latency');
const crdOperationLatency = new Trend('crd_operation_latency');

// Gauges
const activeReconciliations = new Gauge('active_reconciliations');
const queueDepth = new Gauge('queue_depth');

// =============================================================================
// Test Data
// =============================================================================

const nodeTypes = ['Validator', 'Horizon', 'SorobanRpc'];
const networks = ['Mainnet', 'Testnet', 'Futurenet'];

function generateStellarNodeSpec(nodeType) {
    const baseName = `benchmark-${nodeType.toLowerCase()}-${randomString(8)}`;
    
    const baseSpec = {
        apiVersion: 'stellar.org/v1alpha1',
        kind: 'StellarNode',
        metadata: {
            name: baseName,
            namespace: NAMESPACE,
            labels: {
                'app.kubernetes.io/managed-by': 'k6-benchmark',
                'benchmark/run-id': __ENV.RUN_ID || 'local',
            },
        },
        spec: {
            nodeType: nodeType,
            network: networks[randomIntBetween(0, 2)],
            version: 'v21.0.0',
            replicas: 1,
            resources: {
                requests: { cpu: '100m', memory: '256Mi' },
                limits: { cpu: '500m', memory: '512Mi' },
            },
            storage: {
                storageClass: 'standard',
                size: '10Gi',
                retentionPolicy: 'Delete',
            },
        },
    };

    // Add type-specific configuration
    switch (nodeType) {
        case 'Validator':
            baseSpec.spec.validatorConfig = {
                seedSecretRef: 'benchmark-validator-seed',
                enableHistoryArchive: false,
            };
            break;
        case 'Horizon':
            baseSpec.spec.horizonConfig = {
                databaseSecretRef: 'benchmark-horizon-db',
                enableIngest: true,
                stellarCoreUrl: 'http://stellar-core:11626',
                ingestWorkers: 1,
            };
            break;
        case 'SorobanRpc':
            baseSpec.spec.sorobanConfig = {
                stellarCoreUrl: 'http://stellar-core:11626',
                enablePreflight: true,
                maxEventsPerRequest: 1000,
            };
            break;
    }

    return baseSpec;
}

// =============================================================================
// Helper Functions
// =============================================================================

function getHeaders() {
    return {
        'Content-Type': 'application/json',
        'Accept': 'application/json',
    };
}

// Load baseline metrics if available
let baseline = null;
if (__ENV.BASELINE_FILE) {
    try {
        baseline = JSON.parse(open(__ENV.BASELINE_FILE));
        console.log('Loaded baseline metrics from:', __ENV.BASELINE_FILE);
    } catch (e) {
        console.log('No baseline file found, running without regression detection');
    }
}

function checkRegression(metric, value, thresholdPercent = 10) {
    if (!baseline || !baseline[metric]) {
        return true; // No baseline, pass by default
    }
    
    const baselineValue = baseline[metric];
    const allowedIncrease = baselineValue * (1 + thresholdPercent / 100);
    const withinThreshold = value <= allowedIncrease;
    
    regressionCheck.add(withinThreshold ? 1 : 0);
    
    if (!withinThreshold) {
        console.warn(`REGRESSION DETECTED: ${metric} = ${value}ms (baseline: ${baselineValue}ms, threshold: ${allowedIncrease}ms)`);
    }
    
    return withinThreshold;
}

// =============================================================================
// Test Functions
// =============================================================================

export function setup() {
    console.log('Setting up benchmark environment...');
    console.log(`BASE_URL: ${BASE_URL}`);
    console.log(`K8S_API_URL: ${K8S_API_URL}`);
    console.log(`NAMESPACE: ${NAMESPACE}`);
    
    // Create benchmark namespace
    const nsSpec = {
        apiVersion: 'v1',
        kind: 'Namespace',
        metadata: {
            name: NAMESPACE,
            labels: {
                'app.kubernetes.io/managed-by': 'k6-benchmark',
            },
        },
    };
    
    const createNs = http.post(
        `${K8S_API_URL}/api/v1/namespaces`,
        JSON.stringify(nsSpec),
        { headers: getHeaders() }
    );
    
    // Create required secrets for benchmarks
    const secrets = [
        { name: 'benchmark-validator-seed', data: { 'STELLAR_CORE_SEED': 'benchmark-seed' } },
        { name: 'benchmark-horizon-db', data: { 'DATABASE_URL': 'postgres://benchmark:benchmark@localhost/horizon' } },
    ];
    
    for (const secret of secrets) {
        const secretSpec = {
            apiVersion: 'v1',
            kind: 'Secret',
            metadata: { name: secret.name, namespace: NAMESPACE },
            type: 'Opaque',
            stringData: secret.data,
        };
        
        http.post(
            `${K8S_API_URL}/api/v1/namespaces/${NAMESPACE}/secrets`,
            JSON.stringify(secretSpec),
            { headers: getHeaders() }
        );
    }
    
    return {
        startTime: new Date().toISOString(),
        runId: __ENV.RUN_ID || `local-${Date.now()}`,
    };
}

export default function(data) {
    group('Health & Metrics Endpoints', function() {
        // Health check endpoint
        group('Health Check', function() {
            const start = Date.now();
            const res = http.get(`${BASE_URL}/healthz`, {
                tags: { endpoint: 'health' },
            });
            const duration = Date.now() - start;
            
            healthCheckLatency.add(duration);
            totalRequests.add(1);
            tps.add(1);
            
            check(res, {
                'health check returns 200': (r) => r.status === 200,
                'health check within SLA': (r) => duration < 100,
            });
            
            checkRegression('health_check_p95', duration);
        });
        
        // Metrics endpoint
        group('Metrics Scrape', function() {
            const start = Date.now();
            const res = http.get(`${BASE_URL}/metrics`, {
                tags: { endpoint: 'metrics' },
            });
            const duration = Date.now() - start;
            
            metricsLatency.add(duration);
            totalRequests.add(1);
            tps.add(1);
            
            check(res, {
                'metrics returns 200': (r) => r.status === 200,
                'metrics contains reconciliation data': (r) => r.body.includes('reconciliation'),
                'metrics within SLA': (r) => duration < 200,
            });
            
            // Parse queue depth from metrics if available
            const queueMatch = res.body.match(/stellar_operator_queue_depth\s+(\d+)/);
            if (queueMatch) {
                queueDepth.add(parseInt(queueMatch[1]));
            }
            
            checkRegression('metrics_p95', duration);
        });
    });
    
    group('REST API Operations', function() {
        // List StellarNodes
        group('List Nodes', function() {
            const start = Date.now();
            const res = http.get(`${BASE_URL}/api/v1/nodes`, {
                tags: { endpoint: 'list_nodes' },
            });
            const duration = Date.now() - start;
            
            apiLatency.add(duration);
            totalRequests.add(1);
            tps.add(1);
            
            check(res, {
                'list nodes returns 200': (r) => r.status === 200,
                'list nodes is array': (r) => {
                    try {
                        return Array.isArray(JSON.parse(r.body));
                    } catch {
                        return false;
                    }
                },
            });
            
            checkRegression('api_list_p95', duration);
        });
        
        // Get specific node status
        group('Get Node Status', function() {
            const start = Date.now();
            const res = http.get(`${BASE_URL}/api/v1/nodes/${NAMESPACE}/test-node/status`, {
                tags: { endpoint: 'node_status' },
            });
            const duration = Date.now() - start;
            
            apiLatency.add(duration);
            totalRequests.add(1);
            tps.add(1);
            
            // 404 is acceptable if node doesn't exist
            check(res, {
                'get status returns valid response': (r) => r.status === 200 || r.status === 404,
            });
            
            checkRegression('api_status_p95', duration);
        });
    });
    
    group('CRD Operations', function() {
        const nodeType = nodeTypes[randomIntBetween(0, 2)];
        const nodeSpec = generateStellarNodeSpec(nodeType);
        const nodeName = nodeSpec.metadata.name;
        
        // Create StellarNode
        group('Create Node', function() {
            const start = Date.now();
            const res = http.post(
                `${K8S_API_URL}/apis/stellar.org/v1alpha1/namespaces/${NAMESPACE}/stellarnodes`,
                JSON.stringify(nodeSpec),
                {
                    headers: getHeaders(),
                    tags: { endpoint: 'create_node', nodeType: nodeType },
                }
            );
            const duration = Date.now() - start;
            
            crdOperationLatency.add(duration);
            nodeCreations.add(1);
            totalRequests.add(1);
            tps.add(1);
            
            const success = check(res, {
                'create returns 201': (r) => r.status === 201,
                'create within SLA': (r) => duration < 1000,
            });
            
            if (success) {
                // Track reconciliation time
                reconciliationCount.add(1);
                activeReconciliations.add(1);
            }
            
            checkRegression('crd_create_p95', duration);
        });
        
        // Wait for reconciliation
        sleep(randomIntBetween(1, 3));
        
        // Get and verify node status
        group('Verify Reconciliation', function() {
            const start = Date.now();
            const res = http.get(
                `${K8S_API_URL}/apis/stellar.org/v1alpha1/namespaces/${NAMESPACE}/stellarnodes/${nodeName}`,
                {
                    headers: getHeaders(),
                    tags: { endpoint: 'reconcile' },
                }
            );
            const duration = Date.now() - start;
            
            reconciliationDuration.add(duration);
            totalRequests.add(1);
            
            if (res.status === 200) {
                try {
                    const node = JSON.parse(res.body);
                    check(node, {
                        'has status': (n) => n.status !== undefined,
                        'has phase': (n) => n.status && n.status.phase !== undefined,
                        'reconciled successfully': (n) => {
                            const phase = n.status?.phase;
                            return phase === 'Ready' || phase === 'Creating' || phase === 'Pending';
                        },
                    });
                    
                    activeReconciliations.add(node.status?.phase === 'Creating' ? 1 : -1);
                } catch (e) {
                    errorRate.add(1);
                }
            }
            
            checkRegression('reconciliation_p95', duration);
        });
        
        // Update node (scale replicas)
        group('Update Node', function() {
            const patch = [{
                op: 'replace',
                path: '/spec/replicas',
                value: randomIntBetween(1, 3),
            }];
            
            const start = Date.now();
            const res = http.patch(
                `${K8S_API_URL}/apis/stellar.org/v1alpha1/namespaces/${NAMESPACE}/stellarnodes/${nodeName}`,
                JSON.stringify(patch),
                {
                    headers: {
                        ...getHeaders(),
                        'Content-Type': 'application/json-patch+json',
                    },
                    tags: { endpoint: 'update_node' },
                }
            );
            const duration = Date.now() - start;
            
            crdOperationLatency.add(duration);
            nodeUpdates.add(1);
            totalRequests.add(1);
            tps.add(1);
            
            check(res, {
                'update returns 200': (r) => r.status === 200,
                'update within SLA': (r) => duration < 1000,
            });
            
            checkRegression('crd_update_p95', duration);
        });
        
        sleep(randomIntBetween(1, 2));
        
        // Delete node
        group('Delete Node', function() {
            const start = Date.now();
            const res = http.del(
                `${K8S_API_URL}/apis/stellar.org/v1alpha1/namespaces/${NAMESPACE}/stellarnodes/${nodeName}`,
                null,
                {
                    headers: getHeaders(),
                    tags: { endpoint: 'delete_node' },
                }
            );
            const duration = Date.now() - start;
            
            crdOperationLatency.add(duration);
            nodeDeletions.add(1);
            totalRequests.add(1);
            tps.add(1);
            
            check(res, {
                'delete returns 200': (r) => r.status === 200,
                'delete within SLA': (r) => duration < 1000,
            });
            
            checkRegression('crd_delete_p95', duration);
        });
    });
    
    sleep(randomIntBetween(1, 3));
}

export function teardown(data) {
    console.log('Cleaning up benchmark resources...');
    
    // Delete all benchmark nodes
    const listRes = http.get(
        `${K8S_API_URL}/apis/stellar.org/v1alpha1/namespaces/${NAMESPACE}/stellarnodes?labelSelector=app.kubernetes.io/managed-by=k6-benchmark`,
        { headers: getHeaders() }
    );
    
    if (listRes.status === 200) {
        try {
            const nodes = JSON.parse(listRes.body);
            for (const node of nodes.items || []) {
                http.del(
                    `${K8S_API_URL}/apis/stellar.org/v1alpha1/namespaces/${NAMESPACE}/stellarnodes/${node.metadata.name}`,
                    null,
                    { headers: getHeaders() }
                );
            }
        } catch (e) {
            console.error('Error during cleanup:', e);
        }
    }
    
    // Delete benchmark namespace
    http.del(
        `${K8S_API_URL}/api/v1/namespaces/${NAMESPACE}`,
        null,
        { headers: getHeaders() }
    );
    
    console.log('Benchmark completed');
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
        
        // Core metrics
        metrics: {
            tps: {
                avg: data.metrics.tps?.values?.rate || 0,
            },
            http_req_duration: {
                avg: data.metrics.http_req_duration?.values?.avg || 0,
                p50: data.metrics.http_req_duration?.values?.med || 0,
                p95: data.metrics.http_req_duration?.values['p(95)'] || 0,
                p99: data.metrics.http_req_duration?.values['p(99)'] || 0,
                max: data.metrics.http_req_duration?.values?.max || 0,
            },
            reconciliation_duration: {
                avg: data.metrics.reconciliation_duration?.values?.avg || 0,
                p95: data.metrics.reconciliation_duration?.values['p(95)'] || 0,
                p99: data.metrics.reconciliation_duration?.values['p(99)'] || 0,
            },
            api_latency: {
                avg: data.metrics.api_latency?.values?.avg || 0,
                p95: data.metrics.api_latency?.values['p(95)'] || 0,
                p99: data.metrics.api_latency?.values['p(99)'] || 0,
            },
            health_check_latency: {
                p95: data.metrics.health_check_latency?.values['p(95)'] || 0,
            },
            crd_operation_latency: {
                avg: data.metrics.crd_operation_latency?.values?.avg || 0,
                p95: data.metrics.crd_operation_latency?.values['p(95)'] || 0,
            },
            error_rate: data.metrics.http_req_failed?.values?.rate || 0,
            total_requests: data.metrics.total_requests?.values?.count || 0,
            node_creations: data.metrics.node_creations?.values?.count || 0,
            node_updates: data.metrics.node_updates?.values?.count || 0,
            node_deletions: data.metrics.node_deletions?.values?.count || 0,
        },
        
        // Threshold results
        thresholds: data.thresholds || {},
        
        // Check results
        checks: {
            total: data.root_group?.checks?.length || 0,
            passed: data.root_group?.checks?.filter(c => c.passes > 0)?.length || 0,
            failed: data.root_group?.checks?.filter(c => c.fails > 0)?.length || 0,
        },
    };
    
    // Check for regressions
    const regressionRate = data.metrics.regression_check?.values?.rate || 1;
    summary.regression = {
        detected: regressionRate < 0.9,
        passRate: regressionRate,
        threshold: 0.9,
    };
    
    return {
        'stdout': textSummary(data, { indent: ' ', enableColors: true }),
        'results/benchmark-summary.json': JSON.stringify(summary, null, 2),
        'results/benchmark-full.json': JSON.stringify(data, null, 2),
    };
}

// Text summary formatter
function textSummary(data, opts) {
    const lines = [];
    lines.push('');
    lines.push('='.repeat(70));
    lines.push('  STELLAR-K8S OPERATOR BENCHMARK RESULTS');
    lines.push('='.repeat(70));
    lines.push('');
    
    // Core metrics
    lines.push('📊 PERFORMANCE METRICS');
    lines.push('-'.repeat(40));
    
    const httpDuration = data.metrics.http_req_duration?.values || {};
    lines.push(`  TPS (avg):               ${(data.metrics.tps?.values?.rate || 0).toFixed(2)} req/s`);
    lines.push(`  HTTP Duration (avg):     ${(httpDuration.avg || 0).toFixed(2)} ms`);
    lines.push(`  HTTP Duration (p95):     ${(httpDuration['p(95)'] || 0).toFixed(2)} ms`);
    lines.push(`  HTTP Duration (p99):     ${(httpDuration['p(99)'] || 0).toFixed(2)} ms`);
    lines.push('');
    
    const reconDuration = data.metrics.reconciliation_duration?.values || {};
    lines.push(`  Reconciliation (avg):    ${(reconDuration.avg || 0).toFixed(2)} ms`);
    lines.push(`  Reconciliation (p95):    ${(reconDuration['p(95)'] || 0).toFixed(2)} ms`);
    lines.push('');
    
    lines.push(`  Error Rate:              ${((data.metrics.http_req_failed?.values?.rate || 0) * 100).toFixed(2)}%`);
    lines.push(`  Total Requests:          ${data.metrics.total_requests?.values?.count || 0}`);
    lines.push('');
    
    // Threshold results
    lines.push('🎯 THRESHOLD RESULTS');
    lines.push('-'.repeat(40));
    
    for (const [name, result] of Object.entries(data.thresholds || {})) {
        const status = result.ok ? '✅' : '❌';
        lines.push(`  ${status} ${name}`);
    }
    lines.push('');
    
    // Regression check
    const regressionRate = data.metrics.regression_check?.values?.rate || 1;
    const regressionStatus = regressionRate >= 0.9 ? '✅' : '❌';
    lines.push(`${regressionStatus} REGRESSION CHECK: ${(regressionRate * 100).toFixed(1)}% within baseline`);
    lines.push('');
    
    lines.push('='.repeat(70));
    
    return lines.join('\n');
}
