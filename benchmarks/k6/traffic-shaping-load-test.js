import http from 'k6/http';
import { check, sleep } from 'k6';
import { Counter, Rate, Trend } from 'k6/metrics';

const BASE_URL = __ENV.BASE_URL || 'http://localhost:9090';

export const options = {
    scenarios: {
        baseline: {
            executor: 'constant-arrival-rate',
            rate: 200,
            timeUnit: '1s',
            duration: '2m',
            preAllocatedVUs: 120,
            tags: { qos: 'normal' },
        },
        burst: {
            executor: 'ramping-arrival-rate',
            startRate: 150,
            timeUnit: '1s',
            preAllocatedVUs: 200,
            stages: [
                { duration: '45s', target: 400 },
                { duration: '30s', target: 1000 },
                { duration: '45s', target: 250 },
            ],
            startTime: '2m',
            tags: { qos: 'mixed' },
        },
    },
    thresholds: {
        http_req_failed: ['rate<0.05'],
        http_req_duration: ['p(95)<800', 'p(99)<1500'],
        high_priority_success_rate: ['rate>0.98'],
        low_priority_shed_rate: ['rate>0.20'],
    },
};

const highPrioritySuccessRate = new Rate('high_priority_success_rate');
const lowPriorityShedRate = new Rate('low_priority_shed_rate');
const policyLatency = new Trend('policy_latency_ms');
const rejectedRequests = new Counter('traffic_rejected_total');

function classifyPriority(iteration) {
    if (iteration % 10 === 0) {
        return 'critical';
    }
    if (iteration % 4 === 0) {
        return 'high';
    }
    if (iteration % 3 === 0) {
        return 'low';
    }
    return 'normal';
}

export default function () {
    const priority = classifyPriority(__ITER);
    const headers = {
        'x-priority-class': priority,
        Accept: 'application/json',
    };

    const response = http.get(`${BASE_URL}/api/v1/traffic/dashboard`, { headers });
    const ok = check(response, {
        'dashboard endpoint responds': (r) => r.status === 200,
    });

    policyLatency.add(response.timings.duration);

    let body = {};
    try {
        body = response.json();
    } catch (_e) {
        body = {};
    }

    const dropped = Number(body.droppedRequests || 0);
    const total = Number(body.totalRequests || 0);
    const dropRate = total > 0 ? dropped / total : 0;

    if (priority === 'critical' || priority === 'high') {
        highPrioritySuccessRate.add(ok && response.status < 500 ? 1 : 0);
    }

    if (priority === 'low') {
        lowPriorityShedRate.add(dropRate > 0.0 ? 1 : 0);
    }

    if (response.status >= 429 || response.status >= 500) {
        rejectedRequests.add(1);
    }

    sleep(0.05);
}
