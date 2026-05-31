# Horizon Query Optimization and Intelligent Caching

Multi-tier cache architecture for Horizon API query optimization with adaptive
caching, predictive prefetching, and ledger-tied invalidation.

## Cache Topology

```text
┌─────────────────────────────────────────────────────────────┐
│                    Horizon Query Path                        │
│                                                              │
│  Request ──► Query Optimizer ──► L1 Memory (LRU, 60s TTL)  │
│                    │ miss                                    │
│                    ▼                                         │
│              L2 Redis (shared, 300s TTL)                    │
│                    │ miss                                    │
│                    ▼                                         │
│              L3 CDN (historical, 3600s TTL)                 │
│                    │ miss                                    │
│                    ▼                                         │
│              Horizon DB ──► Response Streamer (gzip)        │
└─────────────────────────────────────────────────────────────┘
```

## Configuration

```yaml
spec:
  horizonCache:
    l1Capacity: 4096
    l1TtlSecs: 60
    l2RedisEnabled: true
    l2RedisUrl: "redis://horizon-redis:6379"
    l2TtlSecs: 300
    l3CdnEnabled: true
    l3CdnPrefix: "https://cdn.horizon.example.com/cache"
    l3TtlSecs: 3600
    compressionEnabled: true
```

## Query Optimization

| Query Type | Cache Layer | TTL | Cacheable |
|-----------|-------------|-----|-----------|
| Account | L1 Memory | 300s | Yes |
| Payment | L1 Memory | 120s | Yes |
| Transaction | L2 Redis | 60s | Yes |
| Order Book | L1 Memory | 5s | No |
| Ledger | L3 CDN | 3600s | Yes |

## Cache Invalidation

Cache entries are invalidated on ledger close events:

- `ledger:{sequence}:*` — ledger-specific entries
- `transactions:recent:*` — recent transaction cache
- `payments:recent:*` — recent payment cache

## Predictive Prefetching

The prefetch engine learns co-occurrence patterns (e.g., account → payments →
transactions) and proactively loads related queries into cache.

## Metrics

- `stellar_horizon_cache_hits` — hits per layer (L1/L2/L3)
- `stellar_horizon_cache_misses` — misses per layer
- `stellar_horizon_cache_hit_rate` — hit rate percentage
- `stellar_horizon_query_latency` — query latency histogram

## Tuning Strategies

1. **High-traffic accounts**: Increase L1 capacity and reduce TTL for fresher data
2. **Historical queries**: Enable L3 CDN for ledger and effects endpoints
3. **Real-time trading**: Disable caching for order book queries
4. **Multi-replica deployments**: Enable L2 Redis for cross-replica cache sharing

## Benchmark Results

Run benchmarks with:

```bash
cargo test bench_horizon_cache -- --nocapture
```

Typical results on synthetic workloads:
- L1 cache hit latency: <1µs
- L2 cache hit latency: ~0.5ms
- Cache miss (simulated DB): ~10ms
- Speedup with 80% hit rate: ~5x average latency reduction
