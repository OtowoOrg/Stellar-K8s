# Advanced Database Management & Optimization Guide

This guide details best practices for running and optimizing PostgreSQL databases for the Stellar-K8s operator and high-performance Horizon / Soroban RPC workloads.

---

## 1. Connection Pooling (PgBouncer)

For high-throughput, low-latency applications like Horizon and Soroban RPC, direct database connections cause significant resource overhead due to PostgreSQL's process-based connection model. Using PgBouncer as a connection pooler is highly recommended.

### Pooling Modes

PgBouncer supports three pooling modes:

| Pooling Mode | Description | Best Use Case | Horizon / Soroban Context |
|---|---|---|---|
| **Session** | Keeps a physical connection assigned to the client until it disconnects. | Legacy apps requiring transaction-spanning session states. | Not recommended (restricts concurrency). |
| **Transaction** | Returns the connection to the pool as soon as a transaction finishes. | Modern REST APIs, microservices with short transactions. | **Recommended default** for Horizon and Soroban RPC. |
| **Statement** | Returns the connection to the pool after each SQL statement. | Batch processes with absolutely no multi-statement transactions. | **Not supported** (breaks transaction blocks and prepared statements). |

### PgBouncer Configuration Parameters

- `max_client_conn`: Total maximum clients that can connect (typically `1000` to `5000`).
- `default_pool_size`: Number of physical connections kept open per user/database pair (typically `20` to `50`).
- `pool_mode`: Set to `transaction`.

---

## 2. Read Replica Scaling (1–10 Replicas)

Read replica auto-scaling distributes select queries away from the primary instance, scaling from `1` up to `10` replicas dynamically based on query volume or CPU utilization.

### Configuration Rules

- **Min Replicas**: `1` (always run at least one standby replica for instant failover).
- **Max Replicas**: `10` (scaling beyond 10 can increase replication lag and WAL sender CPU overhead).
- **Auto-Scaling Metrics**: Target CPU utilization at `70%`.

---

## 3. High Availability and Automatic Failover (Patroni / CNPG)

High availability is maintained via active replication and automatic failover managed by **Patroni** or the **CloudNativePG (CNPG)** operator.

- **Consensus / Split-Brain Protection**: Failover managers use a distributed consensus store (like Etcd or Kubernetes API Leases) to ensure only one master/primary writes at a time.
- **Failover Trigger**: If the primary node fails, a replica is automatically promoted to primary within seconds.
- **Sync/Async Replication**: Configure synchronous replication if zero-data-loss is required (RPO=0), or asynchronous replication for maximum performance under high write load.

---

## 4. PostgreSQL Engine Auto-Tuning

Database performance depends heavily on OS and memory allocation parameters. The operator automatically tunes these parameters depending on the selected workload profile:

### Memory Configurations by Profile

- **OLTP (Horizon REST API / Soroban RPC)**: Focuses on quick index lookups and high concurrency.
- **Batch (Catchup / Data Ingestion)**: Focuses on bulk insert throughput and index creation.
- **Mixed**: Balanced allocation.

| Parameter | OLTP | Batch | Mixed | Rationale |
|---|---|---|---|---|
| `shared_buffers` | 25% of RAM | 25% of RAM | 25% of RAM | Cache size for frequently accessed tables/indexes. |
| `effective_cache_size` | 75% of RAM | 75% of RAM | 75% of RAM | Optimizer estimate of kernel disk cache availability. |
| `work_mem` | 16MB | 64MB | 32MB | Memory per sorting or hash operation before writing to disk. |
| `maintenance_work_mem` | 128MB | 1GB | 256MB | Memory for indexes, VACUUM, foreign keys. |
| `max_connections` | 200 | 50 | 100 | Maximum concurrency (should be offset by PgBouncer). |

---

## 5. Slow Query Profiling and Index Recommendations

Identifying and indexing slow queries can improve query performance by **>5x**.

### Recommended PostgreSQL Settings

```ini
shared_preload_libraries = 'pg_stat_statements'
pg_stat_statements.max = 10000
pg_stat_statements.track = all
```

### Auto-Indexing Strategy

1. The operator queries `pg_stat_statements` periodically for queries exceeding `100ms` (configurable via `slowQueryThresholdMs`).
2. It parses equality filters (e.g., `WHERE payment_id = $1`) and recommends appropriate multi-column composite indexes.
3. The operator creates suggested indexes asynchronously using `CREATE INDEX CONCURRENTLY` to avoid blocking read/write traffic.

---

## 6. Zero-Downtime Schema Migrations

When upgrading the database schema, lock escalation can block database connections and trigger outages.

### Migration Best Practices

1. **Avoid `SELECT *`**: Always reference explicit column lists in code so new columns do not cause parsing errors.
2. **Add Columns with Defaults Carefully**: In PostgreSQL 11+, adding a column with a default value is a metadata-only operation (instant). For older versions, create the column, populate it in batches, and then add the `NOT NULL` constraint.
3. **Concurrent Indexes**: Always use `CREATE INDEX CONCURRENTLY` to add indexes.
4. **Lock Timeout**: Always set a lock timeout before running DDL statements to prevent blocking other transactions:
   ```sql
   SET lock_timeout = '10s';
   ```
