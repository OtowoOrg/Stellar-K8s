# Multi-Layered Caching for Horizon using Redis

## Overview
Deploys a Redis-based cache layer for Horizon to improve performance for frequent API requests (e.g., Account lookups) and reduce Postgres IO.

## CRD Extension
To enable caching for Horizon, the `cache` block is supported in the Horizon spec:

```yaml
spec:
  horizon:
    cache:
      enabled: true
      type: redis
      redisCluster:
        replicas: 3
```

## Features
- Automatically provisions a Redis cluster using a StatefulSet or Redis Operator.
- Configures Horizon node to point to the created Redis cache layer.
- Monitors Cache Hit Ratio (exported to Grafana via Redis exporter metrics).
