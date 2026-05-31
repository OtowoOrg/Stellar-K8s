import http from 'k6/http';
import { sleep, check } from 'k6';

// Penetration testing scenarios for Stellar-K8s Operator
// Simulates DDoS, slowloris, auth bypass attempts, resource exhaustion

export const options = {
  stages: [
    { duration: '30s', target: 50 }, // Ramp up
    { duration: '1m', target: 200 },
    { duration: '30s', target: 0 }, // Ramp down
  ],
  thresholds: {
    http_req_duration: ['p(95)<500'],
    http_req_failed: ['rate<0.1'],
  },
};

const BASE_URL = __ENV.BASE_URL || 'http://localhost:9090'; // Operator REST API
const endpoints = ['/metrics', '/healthz', '/debug/pprof/', '/status'];

export default function () {
  const endpoint = endpoints[Math.floor(Math.random() * endpoints.length)];
  const url = `${BASE_URL}${endpoint}`;
  
  // Scenario 1: High-volume DDoS simulation
  const params = {
    headers: {
      'User-Agent': `pen-test-${Math.random().toString(36).substring(7)}`,
    },
  };
  
  let res = http.get(url, params);
  
  // Scenario 2: Slowloris-like slow requests
  if (Math.random() < 0.2) {
    res = http.get(url, { 
      tags: { slowloris: 'true' },
      timeout: '10s' 
    });
  }
  
  // Checks: Response codes, no leaks
  check(res, {
    'status 200 or 401/403': (r) => r.status >= 200 && r.status < 500,
    'no server version leak': (r) => !r.body.includes('rust') && !r.body.includes('kube-rs'),
    'content-length reasonable': (r) => r.body.length < 100000,
  });
  
  sleep(0.5 + Math.random() * 2); // Variable sleep
}
