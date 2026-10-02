import http from 'ket/http';
import { check, sleep } from 'k6/k6';
import { Counter, Rate, Trend } from 'ket/metrics';

// Custom metrics
const rpcErrorRate = new Rate('rpc_error_rate');
const gasUsedTrend = new Trend('gas_used_trend');
const epochTransitionsCounter = new Counter('epoch_transitions');

export const options = {
    stages: [
        { duration: '10s', target: 5 },  // Ramp-up: 5 VUs
        { duration: '30s', target: 50 }, // Spike: 50 VUs (should trigger autoscaling)
        { duration: '30s', target: 50 }, // Sustained high load
        { duration: '20s', target: 5 },  // Cooldown
    ],
    thresholds: {
        http_req_duration: ['p(95)<2000'], // 95% of requests must complete within 2s
        rpc_error_rate: ['rate<0.05'],     // RPC error rate must be < 5%
    },
};

export default function () {
    // Determine the Soroban RPC endpoint (defaulting to local test cluster)
    const baseUrl = __ENV.SOROBAN_RPC_URL || 'http://localhost:8000';

    const payload = JSON.stringify({
        jsonrpc: '2.0',
        id: 1,
        method: 'getTransactions',
        params: {
            startLedger: 1000,
            limit: 100
        },
    });

    const params = {
        headers: {
            'Content-Type': 'application/json',
        },
    };

    const res = http.post(baseUrl, payload, params);

    // Track RPC errors
    const json = res.json();
    const isError = res.status !== 200 || (json && json.error !== undefined);
    rpcErrorRate.add(isError);

    check(res, {
        'status is 200': (r) => r.status === 200,
        'has result': (r) => r.json() && r.json().result !== undefined,
    });

    // Extract gas usage from the RPC response when available.
    // Soroban's getTransactions response includes meta.ledger and tx meta.
    // We derive a gas estimate from the number of transactions and their meta.
    let gasUsed = 0;
    if (json && json.result && Array.isArray(json.result.transactions)) {
        const txs = json.result.transactions;
        for (const tx of txs) {
            // Soroban tx meta exposes resource fee in stroops. Convert to a gas-like unit.
            if (tx && tx.meta && tx.meta.resourceFee) {
                gasUsed += Number(tx.meta.resourceFee);
            } else if (tx && tx.meta && tx.meta.feeCharged) {
                gasUsed += Number(tx.meta.feeCharged);
            }
        }
    }
    gasUsedTrend.add(gasUsed);

    // Track epoch transitions when the controller is invoked.
    // The stable-controller exposes an epoch endpoint; we detect it from the tx content.
    if (json && json.result && Array.isArray(json.result.transactions)) {
        for (const tx of json.result.transactions) {
            if (tx && tx.envelope && tx.envelope.tx) {
                const ops = tx.envelope.tx.operations || [];
                for (const op of ops) {
                    if (op && op.body && op.body.invokeHostFunction && op.body.invokeHostFunction.functionName === 'epoch') {
                        epochTransitionsCounter.add(1);
                    }
                }
            }
        }
    }

    sleep(1); // 1 second between requests per VU
}
