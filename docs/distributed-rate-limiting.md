# Distributed Rate Limiting

Issue #1335. Source: [`src/rest_api/gateway/distributed_ratelimit.rs`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/rest_api/gateway/distributed_ratelimit.rs).

## The problem

`gateway::ratelimit::RateLimiter` keeps its token buckets in a process-local
`HashMap`. Behind a load balancer with N replicas, each replica sees roughly
1/N of the traffic and enforces the full configured limit against its own
slice — so the fleet lets through about **N times** the intended limit. Scaling
the gateway out silently weakened the protection it exists to provide.

## How instances stay in sync

There is no gossip, leader election, or replication protocol. Every instance
derives the same counter key from the request identity and the wall-clock
window:

```
{prefix}:{scope}:{identifier}:{window_start_epoch_seconds}
```

`window_start` is `now - (now % window)`, so every replica floors the same
clock to the same boundary and addresses the same shared counter.
Synchronisation is a property of the key derivation rather than a protocol
that can lag or split-brain.

## Atomicity

Each check is one Redis round trip running a Lua script:

```lua
local c = redis.call('INCR', KEYS[1])
if c == 1 then redis.call('PEXPIRE', KEYS[1], ARGV[1]) end
return c
```

`INCR` and the TTL set happen as a single server-side operation, so two
replicas racing to create the same counter cannot leave it without an
expiry. The client speaks RESP directly over a pooled `TcpStream` rather than
adding a Redis crate — the two commands needed are a few lines, and the
request path stays free of an extra dependency tree.

**Requires Redis with `EVAL` support (2.6+).**

## Failure behaviour

The store sits in the request path, so it **fails open** by default: when
Redis is unreachable the limiter falls back to a process-local counter, marks
the decision `degraded`, and increments
`stellar_gateway_rate_limit_backend_errors_total`. Dropping production traffic
because a rate-limit counter is unavailable trades an availability incident
for a capacity one.

Set `fail_open: false` where exceeding the limit is worse than dropping
traffic.

Because failing open is invisible to clients, the
`StellarGatewayRateLimitBackendDegraded` alert is **critical** — it is the only
signal that fleet-wide enforcement has quietly degraded to per-replica.

## Usage

```rust
use std::sync::Arc;
use std::time::Duration;
use stellar_k8s::rest_api::gateway::{
    DistributedRateLimitConfig, DistributedRateLimiter, RedisCounterStore, RedisStoreConfig,
};

let store = Arc::new(RedisCounterStore::new(RedisStoreConfig {
    address: "redis.stellar-system.svc:6379".into(),
    pool_size: 8,
    timeout: Duration::from_millis(50),
}));

let limiter = DistributedRateLimiter::new(
    DistributedRateLimitConfig {
        max_requests: 100,
        window: Duration::from_secs(60),
        key_prefix: "stellar:ratelimit".into(),
        fail_open: true,
    },
    store,
);

let decision = limiter.check("ip", &client_ip).await;
if !decision.allowed {
    // 429 with Retry-After
    let retry_after = decision.retry_after_seconds(now_epoch);
}
```

For a single-replica deployment or local development, swap
`RedisCounterStore` for `InMemoryCounterStore` — the limiter is identical.

## Metrics and alerting

| Metric | Meaning |
|---|---|
| `stellar_gateway_rate_limit_checks_total{scope}` | Every decision |
| `stellar_gateway_rate_limit_exceeded_total{scope}` | Rejections |
| `stellar_gateway_rate_limit_backend_errors_total` | Fail-open fallbacks |
| `stellar_gateway_rate_limit_check_duration_seconds` | Check cost histogram |

Alert rules: [`monitoring/rate-limit-alerts.yaml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/monitoring/rate-limit-alerts.yaml)

```bash
kubectl apply -f monitoring/rate-limit-alerts.yaml -n monitoring
```

Four alerts: rejection ratio above 5% (warning) and 25% (critical), backend
degraded (critical), and p99 check latency above the 1ms budget (warning).

## Verification

```bash
K8S_OPENAPI_ENABLED_VERSION=1.30 cargo test --lib distributed_ratelimit
```

36 tests. The ones that speak to this issue's acceptance criteria:

| Test | Proves |
|---|---|
| `a_shared_store_enforces_one_limit_across_instances` | Two limiters over one store share a single budget |
| `per_process_limiters_would_let_through_n_times_the_limit` | Documents the bug being fixed |
| `instances_derive_the_same_key_within_a_window` | Replicas agree on the key |
| `keys_differ_once_the_window_rolls_over` | Windows actually roll |
| `redis_increment_sends_one_eval_and_returns_the_count` | Exactly one round trip, with `PEXPIRE` |
| `an_unreachable_store_fails_open_to_a_local_counter` | Fail-open, flagged degraded |
| `fail_closed_rejects_when_the_store_is_down` | Fail-closed honoured when configured |
| `local_check_overhead_stays_under_the_one_millisecond_budget` | Sub-1ms overhead, asserted over 1,000 iterations |

The Redis tests run against a stub RESP server on a loopback port, so no Redis
installation is needed. To check against a real Redis:

```bash
docker run --rm -p 6379:6379 redis:7-alpine
# then point RedisStoreConfig.address at 127.0.0.1:6379
```
