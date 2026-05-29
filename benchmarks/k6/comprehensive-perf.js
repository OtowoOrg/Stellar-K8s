/**
 * Stellar-K8s Comprehensive Performance Test
 * 
 * Measures the operator's performance across various lifecycle operations.
 */

import http from 'k6/http';
import { check, sleep, group } from 'k6';
import { Counter, Rate, Trend, Gauge } from 'k6/metrics';
import { randomString, randomIntBetween } from 'https://jslib.k6.io/k6-utils/1.2.0/index.js';

const BASE_URL = __ENV.BASE_URL || 'http://localhost:8080';
const K8S_API_URL = __ENV.K8S_API_URL || 'http://localhost:8001';
const NAMESPACE = __ENV.NAMESPACE || 'stellar-perf-test';

export const options = {
    scenarios: {
        lifecycle_operations: {
            executor: 'per-vu-iterations',
            vus: 5,
            iterations: 20,
            maxDuration: '10m',
        },
    },
    thresholds: {
        'reconciliation_latency': ['p(95)<5000'],
        'api_latency': ['p(95)<500'],
    },
};

// Custom metrics
const reconciliationLatency = new Trend('reconciliation_latency');
const apiLatency = new Trend('api_latency');

function generateNodeSpec(name) {
    return {
        apiVersion: 'stellar.org/v1alpha1',
        kind: 'StellarNode',
        metadata: {
            name: name,
            namespace: NAMESPACE,
        },
        spec: {
            nodeType: 'Validator',
            network: 'Testnet',
            version: 'v21.0.0',
            replicas: 1,
            storage: {
                storageClass: 'standard',
                size: '10Gi',
            },
            validatorConfig: {
                seedSecretRef: 'perf-seed',
            }
        },
    };
}

export function setup() {
    // Ensure namespace exists
    http.post(`${K8S_API_URL}/api/v1/namespaces`, JSON.stringify({
        apiVersion: 'v1',
        kind: 'Namespace',
        metadata: { name: NAMESPACE }
    }), { headers: { 'Content-Type': 'application/json' } });
}

export default function () {
    const nodeName = `perf-${randomString(6)}`;
    const nodeUrl = `${K8S_API_URL}/apis/stellar.org/v1alpha1/namespaces/${NAMESPACE}/stellarnodes`;

    group('Node Lifecycle', function () {
        // 1. Create Node
        const startCreate = Date.now();
        const createRes = http.post(nodeUrl, JSON.stringify(generateNodeSpec(nodeName)), {
            headers: { 'Content-Type': 'application/json' }
        });
        check(createRes, { 'create success': (r) => r.status === 201 });
        apiLatency.add(Date.now() - startCreate);

        // 2. Wait for Reconciliation
        let reconciled = false;
        const startReconcile = Date.now();
        for (let i = 0; i < 30; i++) {
            const getRes = http.get(`${nodeUrl}/${nodeName}`);
            const body = JSON.parse(getRes.body);
            const conditions = body.status?.conditions || [];
            const ready = conditions.find(c => c.type === 'Ready' && c.status === 'True');
            
            if (ready) {
                reconciled = true;
                reconciliationLatency.add(Date.now() - startReconcile);
                break;
            }
            sleep(2);
        }
        check(reconciled, { 'node reconciled': (r) => r === true });

        // 3. Update Node (Scale)
        const updateSpec = generateNodeSpec(nodeName);
        updateSpec.spec.replicas = 2;
        const startUpdate = Date.now();
        const updateRes = http.patch(`${nodeUrl}/${nodeName}`, JSON.stringify(updateSpec), {
            headers: { 'Content-Type': 'application/merge-patch+json' }
        });
        check(updateRes, { 'update success': (r) => r.status === 200 });
        apiLatency.add(Date.now() - startUpdate);

        // 4. Delete Node
        const deleteRes = http.del(`${nodeUrl}/${nodeName}`);
        check(deleteRes, { 'delete success': (r) => r.status === 200 });
    });

    sleep(1);
}

export function teardown() {
    // Cleanup namespace
    http.del(`${K8S_API_URL}/api/v1/namespaces/${NAMESPACE}`);
}
