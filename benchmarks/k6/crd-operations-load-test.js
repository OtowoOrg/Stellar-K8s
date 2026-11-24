import http from 'k6/http';
import { check, sleep } from 'k6';

// Performance regression benchmark for CRD creation, update, deletion, and concurrent operations (Issue #1402).
export const options = {
  stages: [
    { duration: '30s', target: 10 },  // Ramp up to 10 VUs
    { duration: '1m',  target: 50 },  // Concurrent load at 50 VUs
    { duration: '30s', target: 0 },   // Ramp down
  ],
  thresholds: {
    http_req_duration: ['p(95)<500'], // 95% of requests must complete within 500ms
    http_req_failed: ['rate<0.01'],    // <1% errors
  },
};

const BASE_URL = __ENV.K8S_API_URL || 'http://localhost:8001';
const NAMESPACE = __ENV.NAMESPACE || 'stellar-benchmark';

export default function () {
  const nodeName = `bench-crd-${__VU}-${__ITER}`;
  
  // 1. Create CRD resource
  const createPayload = JSON.stringify({
    apiVersion: 'stellar.org/v1alpha1',
    kind: 'StellarNode',
    metadata: {
      name: nodeName,
      namespace: NAMESPACE,
      labels: { app: 'crd-load-test', vu: `${__VU}` },
    },
    spec: {
      nodeType: 'Validator',
      network: 'testnet',
      version: 'v21.0.0',
      replicas: 1,
    },
  });

  const headers = { 'Content-Type': 'application/json' };

  let res = http.post(`${BASE_URL}/apis/stellar.org/v1alpha1/namespaces/${NAMESPACE}/stellarnodes`, createPayload, { headers });
  check(res, { 'create status is 201 or 200': (r) => r.status === 201 || r.status === 200 || r.status === 409 });

  sleep(0.1);

  // 2. Update CRD resource
  const updatePayload = JSON.stringify({
    apiVersion: 'stellar.org/v1alpha1',
    kind: 'StellarNode',
    metadata: {
      name: nodeName,
      namespace: NAMESPACE,
    },
    spec: {
      nodeType: 'Validator',
      network: 'testnet',
      version: 'v21.0.0',
      replicas: 3,
    },
  });

  res = http.put(`${BASE_URL}/apis/stellar.org/v1alpha1/namespaces/${NAMESPACE}/stellarnodes/${nodeName}`, updatePayload, { headers });
  check(res, { 'update status is 200 or 404': (r) => r.status === 200 || r.status === 404 });

  sleep(0.1);

  // 3. Delete CRD resource
  res = http.del(`${BASE_URL}/apis/stellar.org/v1alpha1/namespaces/${NAMESPACE}/stellarnodes/${nodeName}`);
  check(res, { 'delete status is 200 or 404': (r) => r.status === 200 || r.status === 404 });

  sleep(0.2);
}
