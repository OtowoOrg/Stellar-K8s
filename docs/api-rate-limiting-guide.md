# API Rate Limiting Guide

This guide covers the API rate limiting implementation for Stellar-K8s, including token bucket algorithm, endpoint tiers, per-API-key limits, and monitoring.

## Overview

Stellar-K8s provides comprehensive API rate limiting through:

- **Token bucket algorithm** with configurable capacity and refill rates
- **Per-API-key limits** with independent quotas
- **Endpoint-specific tiers** (free, standard, premium, admin)
- **Distributed rate limiting** via Redis for multi-replica deployments
- **Rate-limit headers** in HTTP responses
- **429 responses** with `Retry-After` header

## Configuration

### Helm Values

```yaml
rateLimiting:
  enabled: true
  distributed:
    enabled: false
    failOpen: true
  redis:
    address: "redis.stellar-system.svc:6379"
    poolSize: 8
    timeoutMs: 50
  tiers:
    - name: free
      requestsPerMinute: 60
      requestsPerHour: 1000
      requestsPerDay: 10000
      burst: 10
    - name: standard
      requestsPerMinute: 100
      requestsPerHour: 10000
      requestsPerDay: 100000
      burst: 20
    - name: premium
      requestsPerMinute: 200
      requestsPerHour: 20000
      requestsPerDay: 200000
      burst: 50
    - name: admin
      requestsPerMinute: 500
      requestsPerHour: 50000
      requestsPerDay: 500000
      burst: 100
  endpointTiers:
    "/health": "free"
    "/healthz": "free"
    "/readyz": "free"
    "/livez": "free"
    "/metrics": "free"
    "/api/v1/nodes": "standard"
    "/api/v1/debug": "admin"
```

### Enable Rate Limiting

```bash
helm install stellar-operator ./charts/stellar-operator \
  --set rateLimiting.enabled=true \
  --set rateLimiting.distributed.enabled=true \
  --set rateLimiting.redis.address=redis.stellar-system.svc:6379
```

## Token Bucket Algorithm

Each API key gets a token bucket with:

- **Capacity:** Maximum tokens (burst size)
- **Refill rate:** Tokens added per second
- **Current tokens:** Available tokens for requests

### How It Works

1. **Request arrives** with API key
2. **Bucket checked** for available tokens
3. **Tokens consumed** (one per request)
4. **Response returned** with rate-limit headers
5. **Tokens refill** over time

### Example

```rust
use stellar_k8s::rest_api::gateway::{TokenBucket, RateLimitConfig};

let bucket = TokenBucket::new(RateLimitConfig {
    requests_per_minute: 100,
    burst: 20,
});

// Consume a token
let allowed = bucket.try_consume();
// Returns: true if tokens available, false if limit exceeded
```

## Endpoint Tiers

Endpoints are classified into tiers with different rate limits:

| Tier | Requests/Min | Requests/Hour | Requests/Day | Burst |
|------|--------------|---------------|--------------|-------|
| Free | 60 | 1,000 | 10,000 | 10 |
| Standard | 100 | 10,000 | 100,000 | 20 |
| Premium | 200 | 20,000 | 200,000 | 50 |
| Admin | 500 | 50,000 | 500,000 | 100 |

### Default Endpoint Mapping

| Endpoint | Tier | Reason |
|----------|------|--------|
| `/health`, `/healthz`, `/readyz`, `/livez` | Free | Health checks |
| `/metrics` | Free | Monitoring scraping |
| `/api/v1/nodes` | Standard | Normal API usage |
| `/api/v1/debug` | Admin | Debug endpoints |

## Per-API-Key Limits

Each API key has independent rate limits:

```rust
use stellar_k8s::rest_api::gateway::{RateLimiter, AuthContext};

let limiter = RateLimiter::new(100, 60);

// Check rate limit for API key
let client_id = auth_context.client_id;
let allowed = limiter.check_client(&client_id).await;

if !allowed {
    return Response::builder()
        .status(429)
        .header("Retry-After", "60")
        .body("Rate limit exceeded")
}
```

## Rate-Limit Headers

Every API response includes rate-limit headers:

```
X-RateLimit-Limit-Minute: 100
X-RateLimit-Remaining-Minute: 85
X-RateLimit-Reset: 1704067200
X-RateLimit-Limit-Hour: 10000
X-RateLimit-Remaining-Hour: 9850
Retry-After: 60
```

| Header | Description |
|--------|-------------|
| `X-RateLimit-Limit-Minute` | Requests allowed per minute |
| `X-RateLimit-Remaining-Minute` | Remaining requests this minute |
| `X-RateLimit-Reset` | Unix timestamp when minute resets |
| `X-RateLimit-Limit-Hour` | Requests allowed per hour |
| `X-RateLimit-Remaining-Hour` | Remaining requests this hour |
| `Retry-After` | Seconds until next request allowed (429 only) |

## 429 Response

When rate limit is exceeded:

```json
{
  "error": "rate_limit_exceeded",
  "message": "Too many requests. Please retry after 60 seconds.",
  "retry_after": 60
}
```

HTTP Status: `429 Too Many Requests`

## Distributed Rate Limiting

For multi-replica deployments, use distributed rate limiting with Redis:

### Architecture

```
Request → API Key → Redis Counter → Response
                    (shared state)
```

### Key Derivation

```
{prefix}:{scope}:{identifier}:{window_start_epoch_seconds}
```

Example: `stellar:ratelimit:client_abc:1704067200`

### Atomicity

Each check uses a Lua script for atomic INCR+EXPIRE:

```lua
local c = redis.call('INCR', KEYS[1])
if c == 1 then redis.call('PEXPIRE', KEYS[1], ARGV[1]) end
return c
```

### Failure Behavior

- **Fail-open (default):** Requests proceed if Redis is unreachable
- **Fail-closed:** Requests rejected if Redis is unreachable

```yaml
rateLimiting:
  distributed:
    enabled: true
    failOpen: true  # or false for strict enforcement
```

## Monitoring

### Prometheus Metrics

| Metric | Description |
|--------|-------------|
| `stellar_gateway_rate_limit_checks_total{scope}` | Total rate limit checks |
| `stellar_gateway_rate_limit_exceeded_total{scope}` | Total rejections |
| `stellar_gateway_rate_limit_backend_errors_total` | Redis connection failures |
| `stellar_gateway_rate_limit_check_duration_seconds` | Check latency histogram |

### Alert Rules

| Alert | Condition | Severity |
|-------|-----------|----------|
| `StellarGatewayRateLimitRejectionsHigh` | >5% rejection rate | Warning |
| `StellarGatewayRateLimitRejectionsCritical` | >25% rejection rate | Critical |
| `StellarGatewayRateLimitBackendDegraded` | Redis errors > 0 | Critical |
| `StellarGatewayRateLimitOverheadBudgetExceeded` | p99 > 1ms | Warning |

### Enable Rate Limit Alerts

```yaml
monitoring:
  enabled: true
  prometheusRule:
    enabled: true
    rateLimitAlerts:
      enabled: true
```

## Testing

### Unit Tests

```bash
K8S_OPENAPI_ENABLED_VERSION=1.30 cargo test --lib distributed_ratelimit
```

36 tests covering:
- Token bucket algorithm
- Per-client limits
- Tier-based limits
- Rate-limit headers
- Distributed state
- Fail-open/fail-closed behavior

### Integration Testing

```bash
# Start Redis
docker run --rm -p 6379:6379 redis:7-alpine

# Run tests against real Redis
REDIS_ADDRESS=127.0.0.1:6379 cargo test --lib distributed_ratelimit
```

## Tuning Guidelines

### For Low Traffic

```yaml
rateLimiting:
  tiers:
    - name: standard
      requestsPerMinute: 50
      burst: 10
```

### For High Traffic

```yaml
rateLimiting:
  tiers:
    - name: standard
      requestsPerMinute: 200
      burst: 50
  distributed:
    enabled: true
```

### For Critical APIs

```yaml
rateLimiting:
  endpointTiers:
    "/api/v1/critical": "premium"
  tiers:
    - name: premium
      requestsPerMinute: 1000
      burst: 200
```

## Security Considerations

- API keys are validated before rate limit check
- Revoked/invalid keys do not receive quota
- Rate-limit state cannot be manipulated by clients
- Multiple replicas share state via Redis
- 429 responses do not disclose internal state

## Troubleshooting

### High Rejection Rate

1. Check if limits are too low for traffic
2. Review client behavior for abuse
3. Adjust tier limits if needed

### Redis Connection Issues

1. Check Redis availability:
   ```bash
   kubectl get pods -n stellar-system -l app=redis
   ```

2. Check Redis logs:
   ```bash
   kubectl logs -n stellar-system -l app=redis
   ```

3. Verify network connectivity:
   ```bash
   kubectl exec -it <pod> -- redis-cli ping
   ```

### High Latency

1. Check Redis latency:
   ```bash
   redis-cli --latency -h redis.stellar-system.svc
   ```

2. Verify co-location of gateway and Redis
3. Check for Redis saturation

## References

- [Token bucket algorithm](https://en.wikipedia.org/wiki/Token_bucket)
- [Redis INCR command](https://redis.io/commands/incr/)
- [HTTP 429 status code](https://developer.mozilla.org/en-US/docs/Web/HTTP/Status/429)
