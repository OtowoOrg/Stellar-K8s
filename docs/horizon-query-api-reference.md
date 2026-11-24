# Horizon Query API Reference

Reference for querying **Horizon** HTTP APIs in Stellar-K8s: endpoints used by operators, pagination and filtering patterns, deployment-specific URLs, caching, and error handling.

**See also:** [Horizon deployment guide](deployment-guides/horizon.md) · [Horizon query cache](horizon-query-cache.md) · [Health checks](health-checks.md) · [Example manifest](https://github.com/OtowoOrg/Stellar-K8s/blob/main/examples/horizon-with-health-check.yaml) · [Official Horizon API docs](https://developers.stellar.org/docs/data/apis/horizon)

---

## Navigation

| Section | Topic |
|---------|--------|
| [1. Horizon in Stellar-K8s](#1-horizon-in-stellar-k8s) | How Horizon nodes are exposed |
| [2. Base URL and discovery](#2-base-url-and-discovery) | Finding your endpoint |
| [3. Core query endpoints](#3-core-query-endpoints) | Ledgers, accounts, transactions, payments |
| [4. Pagination, ordering, and filtering](#4-pagination-ordering-and-filtering) | Cursor model |
| [5. Operator cache API](#5-operator-cache-api) | `/api/v1/horizon/cache/status` |
| [6. Monitoring and operations](#6-monitoring-and-operations) | Health, ingestion lag, metrics |
| [7. Common errors](#7-common-errors) | HTTP status codes and fixes |
| [8. Repository references](#8-repository-references) | CRD, examples, code |

---

## 1. Horizon in Stellar-K8s

Horizon is deployed with `spec.nodeType: Horizon` on the `StellarNode` CRD. The operator configures ingestion from Stellar Core (`spec.horizonConfig.stellarCoreUrl`, typically `http://stellar-core:11626`) and a PostgreSQL database (`spec.horizonConfig.databaseSecretRef`).

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: my-horizon
  namespace: stellar
spec:
  nodeType: Horizon
  network: testnet
  version: "v21.0.0"
  replicas: 2
  horizonConfig:
    databaseSecretRef: horizon-db-credentials
    enableIngest: true
    stellarCoreUrl: "http://stellar-core:11626"
```

Full example: [`examples/horizon-with-health-check.yaml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/examples/horizon-with-health-check.yaml).

Horizon serves a **REST JSON API** (Stellar Horizon protocol). Stellar-K8s does not change URL paths; it manages pods, services, ingress, and optional **Horizon query caching** (`spec.horizonCache` — see [horizon-query-cache.md](horizon-query-cache.md)).

---

## 2. Base URL and discovery

| Access pattern | Typical base URL |
|----------------|------------------|
| In-cluster Service | `http://<service-name>.<namespace>.svc.cluster.local:8000` |
| `kubectl port-forward` | `http://localhost:8000` |
| Ingress / LoadBalancer | `https://horizon.example.com` |
| Public testnet (examples below) | `https://horizon-testnet.stellar.org` |

### Procedure: port-forward to a cluster Horizon

```bash
kubectl port-forward -n stellar svc/my-horizon 8000:8000
```

### Procedure: verify API root

```bash
curl -sS http://localhost:8000/ | jq '._links | keys'
```

Against public testnet (validated 2026-07-27):

```bash
curl -sS https://horizon-testnet.stellar.org/ | jq '._links | keys[0:5]'
```

---

## 3. Core query endpoints

Horizon uses **GET** for read endpoints. Responses include `_links` (HAL-style) and `_embedded` records where applicable.

### Service health

| Purpose | Method | Path | Notes |
|---------|--------|------|-------|
| Ingestion health | `GET` | `/health` | JSON: `database_connected`, `core_up`, `core_synced` |

```bash
curl -sS https://horizon-testnet.stellar.org/health
```

Example response (testnet, validated):

```json
{"database_connected":true,"core_up":true,"core_synced":true}
```

### Ledgers

| Purpose | Method | Path |
|---------|--------|------|
| List ledgers | `GET` | `/ledgers` |
| Single ledger | `GET` | `/ledgers/{sequence_or_hash}` |

```bash
curl -sS "https://horizon-testnet.stellar.org/ledgers?order=desc&limit=1" | jq '._embedded.records[0].sequence'
```

### Accounts

| Purpose | Method | Path |
|---------|--------|------|
| Account details | `GET` | `/accounts/{account_id}` |
| Account operations | `GET` | `/accounts/{account_id}/operations` |
| Account payments | `GET` | `/accounts/{account_id}/payments` |
| Account transactions | `GET` | `/accounts/{account_id}/transactions` |

```bash
curl -sS "https://horizon-testnet.stellar.org/accounts/GBDCULE53LUPK4XHUCXBI35MAZFQHENMZ3JRKAJS2PPYBV646M6XKVHG" | jq '.sequence'
```

### Global collections

| Resource | Path |
|----------|------|
| Transactions | `/transactions` |
| Payments | `/payments` |
| Operations | `/operations` |
| Effects | `/effects` |
| Trades | `/trades` |
| Order book | `/order_book?selling_asset_type=native&buying_asset_type=native` |

```bash
curl -sS "https://horizon-testnet.stellar.org/transactions?order=desc&limit=2" \
  | jq '._embedded.records | length'
```

### Submit transactions (write path)

| Purpose | Method | Path |
|---------|--------|------|
| Submit transaction | `POST` | `/transactions` |

Submission requires a signed XDR envelope (`Content-Type: application/x-www-form-urlencoded` with `tx=` parameter). Operational testing is network-specific; use the [Stellar SDK](https://developers.stellar.org/docs/tools/sdks) against your deployment base URL.

---

## 4. Pagination, ordering, and filtering

### Pagination (cursor)

Horizon paginates with:

| Parameter | Description |
|-----------|-------------|
| `limit` | Page size (default/max per Horizon version) |
| `order` | `asc` or `desc` |
| `cursor` | Opaque cursor from `_links.next.href` |

```bash
# First page
curl -sS "https://horizon-testnet.stellar.org/payments?order=desc&limit=5" -o /tmp/p1.json
# Extract next cursor from _links.next (or use jq on records[-1].paging_token)
jq -r '._embedded.records[-1].paging_token' /tmp/p1.json
```

Use the paging token as `cursor=` on the next request.

### Filtering

Many collection endpoints support filters documented in the [official Horizon reference](https://developers.stellar.org/docs/data/apis/horizon/api-reference). Common patterns:

| Parameter | Example use |
|-----------|-------------|
| `?account=` | Payments involving an account |
| `?asset_code=` / `?asset_issuer=` | Asset-scoped queries |
| `?tx_hash=` | Effects for a transaction |

### Response shape

Typical list response:

```json
{
  "_links": { "self": {}, "next": {}, "prev": {} },
  "_embedded": { "records": [ ] }
}
```

---

## 5. Operator cache API

When the operator REST API is enabled, cache observability is exposed at:

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/v1/horizon/cache/status` | Cache config and hit-rate stats |

```bash
kubectl port-forward -n stellar-system svc/stellar-operator 9090:9090
curl -sS http://localhost:9090/api/v1/horizon/cache/status | jq .
```

CRD configuration for multi-tier caching: [horizon-query-cache.md](horizon-query-cache.md). Implementation: [`src/rest_api/horizon_cache_handlers.rs`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/rest_api/horizon_cache_handlers.rs).

---

## 6. Monitoring and operations

### Kubernetes readiness

The operator marks Horizon `Ready` only when ingestion is caught up:

```bash
kubectl get stellarnode my-horizon -n stellar -o jsonpath='{.status.phase}{"\n"}'
kubectl get stellarnode my-horizon -n stellar -o jsonpath='{.status.ledgerSequence}{"\n"}'
```

See [health-checks.md](health-checks.md).

### Prometheus metrics

| Metric | Meaning |
|--------|---------|
| `stellar_horizon_ingestion_lag_seconds` | Ingestion behind network |
| `stellar_horizon_request_duration_seconds` | API latency |
| `stellar_horizon_requests_total` | Request volume |
| `stellar_horizon_cache_hit_rate` | Operator cache effectiveness (when enabled) |

Details: [STELLAR_METRICS_GUIDE.md — Horizon Metrics](metrics/STELLAR_METRICS_GUIDE.md#horizon-metrics).

### Cross-cloud failover

For multi-cloud Horizon endpoints and health checks, see [cross-cloud-failover.md](cross-cloud-failover.md) (`crossCloudFailover` on `StellarNode`).

---

## 7. Common errors

| HTTP | Meaning | Operational action |
|------|---------|-------------------|
| `400` | Bad request (invalid params or account id) | Fix query string; verify G-address checksum |
| `404` | Resource not found | Confirm network (testnet vs mainnet) and ledger/account existence |
| `429` | Rate limited | Back off; scale Horizon replicas; enable cache per [horizon-query-cache.md](horizon-query-cache.md) |
| `503` | Horizon not ready | Check `/health`; verify `core_synced` and DB connectivity |
| Connection refused | Service not exposed | Check Service/Ingress and port-forward target port |

Horizon pod logs:

```bash
kubectl logs -n stellar -l app.kubernetes.io/instance=my-horizon --tail=100
```

Database and ingestion tuning: [performance-tuning.md](performance-tuning.md#postgresql-for-horizon).

---

## 8. Repository references

| Item | Location |
|------|----------|
| `StellarNode` Horizon spec | [`config/crd/stellarnode-crd.yaml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/config/crd/stellarnode-crd.yaml) |
| Health check reconciler | [`src/controller/`](https://github.com/OtowoOrg/Stellar-K8s/tree/main/src/controller) (Horizon sync phases) |
| Horizon cache controller | [`src/controller/horizon_cache/`](https://github.com/OtowoOrg/Stellar-K8s/tree/main/src/controller/horizon_cache) |
| Ingress examples | [ingress-guide.md](ingress-guide.md) |
| Deployment pattern (RPC farm) | [deployment-patterns/horizon-rpc-farm.md](deployment-patterns/horizon-rpc-farm.md) |

---

**Last updated:** 2026-07-27
