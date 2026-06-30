# Metrics API

Stellar-K8s exposes Prometheus-compatible metrics for the operator, webhooks, and managed workloads.

## Metrics Endpoint

- `/metrics` — standard Prometheus scrape endpoint

## Common Metrics

- `stellar_operator_reconcile_count_total` — total reconciliations executed by the operator
- `stellar_operator_reconcile_duration_seconds` — duration of reconciliation loops
- `stellar_operator_workqueue_depth` — current work queue depth
- `stellar_operator_crd_events_total` — number of CRD watch events processed
- `stellar_webhook_requests_total` — total admission webhook requests
- `stellar_webhook_request_duration_seconds` — webhook processing latency

## Integration

Use the operator `/metrics` endpoint for:

- health dashboards
- alerting on reconciliation failures
- capacity planning for control-plane load
- identifying stale or overloaded webhook activity

## Best Practices

- Scrape operator and webhook metrics from a central Prometheus instance.
- Record reconciliation latency and error counts separately for alerts.
- Validate custom metrics emitted by sidecars and managed StellarCore pods.
