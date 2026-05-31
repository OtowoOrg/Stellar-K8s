import http from 'k6/http';
import { sleep } from 'k6';
import { Counter } from 'k6/metrics';
import { textSummary } from 'https://jslib.k6.io/k6-summary/0.0.1/index.js';

const downtimeEvents = new Counter('downtime_events');

const BASE_URL = __ENV.BASE_URL || 'http://localhost:8080';

export const options = {
    vus: 10,
    duration: '10m',
    thresholds: {
        http_req_failed: ['rate<0.01'],
        'http_req_duration{endpoint:healthz}': ['p(95)<200'],
    },
};

export default function () {
    const res = http.get(`${BASE_URL}/healthz`, {
        tags: { endpoint: 'healthz' },
    });

    if (res.status !== 200) {
        downtimeEvents.add(1);
    }

    sleep(1);
}

export function handleSummary(data) {
    const totalRequests = data.metrics.http_reqs ? data.metrics.http_reqs.values.count : 0;
    const downtime = data.metrics.downtime_events ? data.metrics.downtime_events.values.count : 0;
    const errorRate = totalRequests > 0 ? downtime / totalRequests : 0;

    const summary = {
        downtime_events: downtime,
        total_requests: totalRequests,
        error_rate: errorRate,
    };

    console.log(JSON.stringify(summary, null, 2));

    return {
        'upgrade-load-test-summary.json': JSON.stringify(summary, null, 2),
        stdout: textSummary(data, { indent: ' ', enableColors: true }),
    };
}
